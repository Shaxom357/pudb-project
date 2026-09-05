// db_ffi/src/kdb_session.rs
// .kdb 暗号化ストレージを Python (ctypes) から本格利用するための FFI
//
// 提供機能:
//   - KdbSession: Database + KdbFile(WAL) を束ねた不透明ポインタ
//   - kdb_open / kdb_close / kdb_save / kdb_count
//   - kdb_insert       … WAL 追記による O(1) INSERT
//   - kdb_insert_many  … JSON 配列を 1 件ずつ WAL 追記（バックフィル用）
//   - kdb_sql          … SELECT / INSERT / UPDATE / DELETE を文字列で実行し JSON を返す
//   - kdb_get_by_label / kdb_get_by_label_ndjson
//
// メモリ管理の規約（既存 db_ffi と共通）:
//   - Rust 側で確保した文字列は db_string_free() で解放する
//   - KdbSession ポインタは kdb_close() で解放する
//
// 単一ライター制約:
//   あるプロセスが kdb_open 中は、同じ .kdb に対して別プロセス（db_client 等）を
//   起動しないこと。WAL の二重書き込みでファイルが破損する。kdb_open は
//   "<path>.lock" を排他作成する advisory lock でこれを補助する（強制ではない）。

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Write};
use std::os::raw::c_char;
use std::path::PathBuf;

use db_engine::kdb_store::KdbFile;
use db_engine::{Database, DataType, Record};
use sql_engine::{run_delete, run_insert_fast, run_select, run_update, CellValue};

use crate::{c_str_to_str, json_to_record, json_value_to_record, record_to_json, to_c_string};

// ---------------------------------------------------------------------------
// advisory lock ファイル
// ---------------------------------------------------------------------------

/// "<path>.lock" を保持し、Drop 時に削除する
struct LockGuard {
    path: PathBuf,
}

impl LockGuard {
    /// ロックを取得する。既に存在する場合は Err（別プロセスが使用中とみなす）。
    fn acquire(kdb_path: &str) -> std::io::Result<Self> {
        let path = PathBuf::from(format!("{kdb_path}.lock"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let _ = write!(file, "{}", std::process::id());
        Ok(LockGuard { path })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// KdbSession
// ---------------------------------------------------------------------------

/// Database（メモリ上の列指向ストア）と KdbFile（暗号化 WAL）を束ねたセッション。
/// Python からは不透明ポインタとして扱う。
pub struct KdbSession {
    db: Database,
    kdb: Option<KdbFile>,
    _lock: Option<LockGuard>,
}

impl KdbSession {
    /// UPDATE / DELETE 後の再圧縮（チェックポイント）。
    /// db_client::handlers::auto_save と同じく、セッションが保持している
    /// KdbFile ハンドルで compact する（同一ファイルを 2 本開かない）。
    fn checkpoint(&mut self) -> bool {
        match self.kdb.as_mut() {
            Some(kdb) => {
                let records: Vec<Record> =
                    self.db.list_all().into_iter().cloned().collect();
                kdb.compact(&records, self.db.next_id()).is_ok()
            }
            // 純メモリセッション（kdb 無し）は何もすることがない
            None => true,
        }
    }
}

/// ポインタを &mut KdbSession に変換する内部ヘルパー
fn as_session<'a>(ptr: *mut KdbSession) -> Option<&'a mut KdbSession> {
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &mut *ptr })
    }
}

// ---------------------------------------------------------------------------
// 値変換ヘルパー
// ---------------------------------------------------------------------------

/// sql_engine::CellValue → JSON 値（db_client/src/sql_handlers.rs と同じ規則）
fn cell_to_json(cell: &CellValue) -> serde_json::Value {
    match cell {
        CellValue::Text(s) => serde_json::Value::String(s.clone()),
        CellValue::Integer(n) => serde_json::Value::Number((*n).into()),
        CellValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CellValue::Boolean(b) => serde_json::Value::Bool(*b),
        CellValue::Null => serde_json::Value::Null,
    }
}

/// DataType → 「型タグ無し」の素の JSON 値（NDJSON フラット形式用）
fn data_type_to_plain(dt: &DataType) -> serde_json::Value {
    match dt {
        DataType::Text(s) => serde_json::Value::String(s.clone()),
        DataType::Integer(n) => serde_json::Value::Number((*n).into()),
        DataType::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        DataType::Boolean(b) => serde_json::Value::Bool(*b),
        DataType::Null => serde_json::Value::Null,
    }
}

/// Record を「SELECT * 相当」のフラット JSON にする（NDJSON 1 行分）。
/// 形: {"id": 1, <各カラム>, "labels": ["y2024", ...]}
/// ※ `id` / `labels` は予約キー。同名カラムがあるとカラム側で上書きされる
///   （sql_engine の SELECT * と同じ挙動）。
fn record_to_flat_json(record: &Record) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::Number(record.id.into()));
    for (k, v) in &record.columns {
        map.insert(k.clone(), data_type_to_plain(v));
    }
    map.insert(
        "labels".to_string(),
        serde_json::Value::Array(
            record
                .labels
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    serde_json::Value::Object(map)
}

fn err_json(msg: impl Into<String>) -> *mut c_char {
    to_c_string(serde_json::json!({ "ok": false, "error": msg.into() }).to_string())
}

// ---------------------------------------------------------------------------
// セッション管理
// ---------------------------------------------------------------------------

/// .kdb ファイルを開く（無ければ新規作成）。
/// 失敗時（ロック取得失敗・I/O エラー等）は NULL を返す。
/// Python: sess = lib.kdb_open(b"race.kdb")
#[unsafe(no_mangle)]
pub extern "C" fn kdb_open(path: *const c_char) -> *mut KdbSession {
    let path = match c_str_to_str(path) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let lock = match LockGuard::acquire(path) {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            eprintln!(
                "[db_ffi] kdb_open: '{path}.lock' が既に存在します。\
                 別プロセスが同じ .kdb を開いている可能性があります（単一ライター制約）。"
            );
            return std::ptr::null_mut();
        }
        Err(e) => {
            eprintln!("[db_ffi] kdb_open: ロックファイル作成に失敗: {e}");
            return std::ptr::null_mut();
        }
    };

    let db = match Database::load_or_new_kdb(path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[db_ffi] kdb_open: ロードに失敗: {e}");
            return std::ptr::null_mut();
        }
    };
    let kdb = match KdbFile::open_or_create(path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("[db_ffi] kdb_open: KdbFile のオープンに失敗: {e}");
            return std::ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(KdbSession {
        db,
        kdb: Some(kdb),
        _lock: Some(lock),
    }))
}

/// セッションを閉じる（ロックファイルも解放される）。
/// Python: lib.kdb_close(sess)
#[unsafe(no_mangle)]
pub extern "C" fn kdb_close(ptr: *mut KdbSession) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr)); }
    }
}

/// チェックポイント（WAL 全書き直し）。UPDATE/DELETE 後や終了前に呼ぶ。
/// 戻り値: 1=成功, 0=失敗
/// Python: ok = lib.kdb_save(sess)
#[unsafe(no_mangle)]
pub extern "C" fn kdb_save(ptr: *mut KdbSession) -> i32 {
    match as_session(ptr) {
        Some(s) => i32::from(s.checkpoint()),
        None => 0,
    }
}

/// 有効レコード数
/// Python: n = lib.kdb_count(sess)
#[unsafe(no_mangle)]
pub extern "C" fn kdb_count(ptr: *mut KdbSession) -> u64 {
    match as_session(ptr) {
        Some(s) => s.db.count() as u64,
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// INSERT
// ---------------------------------------------------------------------------

/// レコードを 1 件挿入する（WAL 追記, O(1)）。
/// json: {"id":0, "columns":{"c":{"type":"text","value":"x"}}, "labels":[...]}
/// 戻り値: 発行されたレコードID（0=失敗）
/// Python: rid = lib.kdb_insert(sess, json_bytes)
#[unsafe(no_mangle)]
pub extern "C" fn kdb_insert(ptr: *mut KdbSession, json: *const c_char) -> u64 {
    let Some(s) = as_session(ptr) else { return 0 };
    let Some(json_str) = c_str_to_str(json) else { return 0 };
    let Some(record) = json_to_record(json_str) else { return 0 };
    s.db.insert_fast(record, s.kdb.as_mut()).unwrap_or(0)
}

/// JSON 配列を 1 件ずつ WAL 追記する（200 万件バックフィル向け。
/// FFI 呼び出し回数と JSON パース回数を 1 回に集約する）。
/// json_array: [{"columns":{...}, "labels":[...]}, ...]
/// 戻り値: 挿入に成功した件数。配列としてパースできない場合のみ -1。
///         個々のレコードの失敗（不正 JSON・重複ラベル等）はスキップして続行する。
/// Python: n = lib.kdb_insert_many(sess, json_array_bytes)
#[unsafe(no_mangle)]
pub extern "C" fn kdb_insert_many(ptr: *mut KdbSession, json_array: *const c_char) -> i64 {
    let Some(s) = as_session(ptr) else { return -1 };
    let Some(json_str) = c_str_to_str(json_array) else { return -1 };
    let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(json_str)
    else {
        return -1;
    };

    let mut ok = 0i64;
    for item in &items {
        let Some(record) = json_value_to_record(item) else { continue };
        if s.db.insert_fast(record, s.kdb.as_mut()).is_ok() {
            ok += 1;
        }
    }
    ok
}

// ---------------------------------------------------------------------------
// SQL
// ---------------------------------------------------------------------------

/// SELECT / INSERT / UPDATE / DELETE を実行し、結果を JSON 文字列で返す。
/// （JOIN・GROUP BY は非対応。WHERE / ORDER BY / LIMIT / ラベル AND-OR /
///   DELETE FROM label.x WHERE ... は利用可能）
///
/// 返す JSON:
///   SELECT : {"ok":true,"columns":[...],"rows":[[...]],"total_matched":N,"returned":M}
///   INSERT : {"ok":true,"inserted_id":N,"columns":[...],"labels":[...]}
///   UPDATE : {"ok":true,"updated_count":N}
///   DELETE : {"ok":true,"deleted_count":N}
///   失敗   : {"ok":false,"error":"..."}
/// → db_string_free() で解放すること
/// Python: ptr = lib.kdb_sql(sess, b"SELECT * FROM label.race LIMIT 10")
#[unsafe(no_mangle)]
pub extern "C" fn kdb_sql(ptr: *mut KdbSession, query: *const c_char) -> *mut c_char {
    let Some(s) = as_session(ptr) else {
        return err_json("invalid session pointer");
    };
    let Some(raw) = c_str_to_str(query) else {
        return err_json("invalid query string");
    };
    let query = raw.trim();
    let head = query.to_uppercase();
    let head = head.trim_start();

    if head.starts_with("SELECT") {
        return match run_select(&s.db, query) {
            Ok(result) => {
                let rows: Vec<Vec<serde_json::Value>> = result
                    .rows
                    .iter()
                    .map(|row| row.iter().map(cell_to_json).collect())
                    .collect();
                to_c_string(
                    serde_json::json!({
                        "ok": true,
                        "columns": result.columns,
                        "rows": rows,
                        "total_matched": result.total_matched,
                        "returned": result.rows.len(),
                    })
                    .to_string(),
                )
            }
            Err(e) => err_json(e.to_string()),
        };
    }

    // EXPLAIN <SELECT文>: クエリは実行せず、二次インデックスを使ったかどうかの
    // 実行計画だけを1行で返す（CREATE INDEX / DROP INDEX 自体は現時点では
    // REST API (`/sql`) 経由のみで、この FFI からはまだ作成できない）。
    if head.starts_with("EXPLAIN") {
        return match sql_engine::run_explain(&s.db, query) {
            Ok(result) => {
                let rows: Vec<Vec<serde_json::Value>> = result
                    .rows
                    .iter()
                    .map(|row| row.iter().map(cell_to_json).collect())
                    .collect();
                to_c_string(
                    serde_json::json!({ "ok": true, "columns": result.columns, "rows": rows }).to_string(),
                )
            }
            Err(e) => err_json(e.to_string()),
        };
    }

    if head.starts_with("INSERT") {
        // kdb モードなので WAL 追記（insert_fast）。追記済みのため checkpoint 不要。
        return match run_insert_fast(&mut s.db, s.kdb.as_mut(), query) {
            Ok(result) => to_c_string(
                serde_json::json!({
                    "ok": true,
                    "inserted_id": result.id,
                    "columns": result.columns,
                    "labels": result.labels,
                })
                .to_string(),
            ),
            Err(e) => err_json(e.to_string()),
        };
    }

    if head.starts_with("UPDATE") {
        return match run_update(&mut s.db, query) {
            Ok(result) => {
                let saved = s.checkpoint();
                to_c_string(
                    serde_json::json!({
                        "ok": saved,
                        "updated_count": result.updated_count,
                        "error": if saved { serde_json::Value::Null }
                                 else { serde_json::Value::String("checkpoint failed".into()) },
                    })
                    .to_string(),
                )
            }
            Err(e) => err_json(e.to_string()),
        };
    }

    if head.starts_with("DELETE") {
        return match run_delete(&mut s.db, query) {
            Ok(result) => {
                let saved = s.checkpoint();
                to_c_string(
                    serde_json::json!({
                        "ok": saved,
                        "deleted_count": result.deleted_count,
                        "error": if saved { serde_json::Value::Null }
                                 else { serde_json::Value::String("checkpoint failed".into()) },
                    })
                    .to_string(),
                )
            }
            Err(e) => err_json(e.to_string()),
        };
    }

    err_json("Only SELECT, EXPLAIN, INSERT, UPDATE, and DELETE statements are supported.")
}

// ---------------------------------------------------------------------------
// ラベル検索
// ---------------------------------------------------------------------------

/// ラベルでレコードを取得し、型タグ付き JSON 配列で返す（db_get_by_label と同形式）。
/// → db_string_free() で解放すること
/// Python: ptr = lib.kdb_get_by_label(sess, b"y2024")
#[unsafe(no_mangle)]
pub extern "C" fn kdb_get_by_label(ptr: *mut KdbSession, label: *const c_char) -> *mut c_char {
    let Some(s) = as_session(ptr) else { return std::ptr::null_mut() };
    let Some(label) = c_str_to_str(label) else { return std::ptr::null_mut() };
    let records: Vec<serde_json::Value> =
        s.db.get_by_label(label).iter().map(|r| record_to_json(r)).collect();
    to_c_string(serde_json::Value::Array(records).to_string())
}

/// ラベルでレコードを取得し、フラット NDJSON をファイルに書き出す。
/// 1 行 = 1 レコード = {"id":.., <各カラム>, "labels":[..]}（SELECT * 相当）。
/// 巨大スライス（年 40 万件クラス）を 1 本の巨大 JSON 文字列で ctypes 越しに
/// 返すのを避け、polars.scan_ndjson で直接読めるファイルにする。
/// 戻り値: 書き出した件数。ファイル I/O 失敗時は -1。
/// Python: n = lib.kdb_get_by_label_ndjson(sess, b"y2024", b"/tmp/y2024.ndjson")
#[unsafe(no_mangle)]
pub extern "C" fn kdb_get_by_label_ndjson(
    ptr: *mut KdbSession,
    label: *const c_char,
    out_path: *const c_char,
) -> i64 {
    let Some(s) = as_session(ptr) else { return -1 };
    let Some(label) = c_str_to_str(label) else { return -1 };
    let Some(out_path) = c_str_to_str(out_path) else { return -1 };

    let file = match File::create(out_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[db_ffi] kdb_get_by_label_ndjson: {out_path}: {e}");
            return -1;
        }
    };
    let mut w = BufWriter::new(file);
    let mut count = 0i64;
    for record in s.db.get_by_label(label) {
        let line = record_to_flat_json(record).to_string();
        if writeln!(w, "{line}").is_err() {
            return -1;
        }
        count += 1;
    }
    if w.flush().is_err() {
        return -1;
    }
    count
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn cstr(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// テスト用の一意な .kdb パス（ロックファイルも隔離される）
    fn tmp_kdb(tag: &str) -> String {
        let p = std::env::temp_dir().join(format!(
            "db_ffi_kdb_{}_{}_{}.kdb",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p.to_str().unwrap().to_string()
    }

    fn cleanup(path: &str) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(format!("{path}.lock"));
    }

    fn read_free(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr).to_str().unwrap().to_string() };
        crate::db_string_free(ptr);
        s
    }

    #[test]
    fn test_kdb_open_insert_count_close_reopen() {
        let path = tmp_kdb("persist");
        let cp = cstr(&path);

        let sess = kdb_open(cp.as_ptr());
        assert!(!sess.is_null());
        let rid = kdb_insert(
            sess,
            cstr(r#"{"id":0,"columns":{"name":{"type":"text","value":"Alice"}},"labels":["y2024"]}"#)
                .as_ptr(),
        );
        assert_eq!(rid, 1);
        assert_eq!(kdb_count(sess), 1);
        kdb_close(sess);

        // 再オープンして WAL から復元されることを確認
        let sess2 = kdb_open(cp.as_ptr());
        assert!(!sess2.is_null());
        assert_eq!(kdb_count(sess2), 1);
        kdb_close(sess2);

        cleanup(&path);
    }

    #[test]
    fn test_kdb_lock_prevents_second_open() {
        let path = tmp_kdb("lock");
        let cp = cstr(&path);

        let sess = kdb_open(cp.as_ptr());
        assert!(!sess.is_null());
        // 同じ .kdb をもう一度開こうとすると NULL
        let sess2 = kdb_open(cp.as_ptr());
        assert!(sess2.is_null());
        kdb_close(sess);
        // 解放後は再度開ける
        let sess3 = kdb_open(cp.as_ptr());
        assert!(!sess3.is_null());
        kdb_close(sess3);

        cleanup(&path);
    }

    #[test]
    fn test_kdb_insert_many() {
        let path = tmp_kdb("many");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        let arr = r#"[
            {"columns":{"n":{"type":"integer","value":1}},"labels":["y2024"]},
            {"columns":{"n":{"type":"integer","value":2}},"labels":["y2024"]},
            {"columns":{"n":{"type":"integer","value":3}},"labels":["y2025"]}
        ]"#;
        assert_eq!(kdb_insert_many(sess, cstr(arr).as_ptr()), 3);
        assert_eq!(kdb_count(sess), 3);

        // 配列でない入力は -1
        assert_eq!(kdb_insert_many(sess, cstr(r#"{"not":"array"}"#).as_ptr()), -1);

        kdb_close(sess);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_sql_select_and_insert() {
        let path = tmp_kdb("sql");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        let r = read_free(kdb_sql(
            sess,
            cstr("INSERT INTO (label.race) (race_no, payout) VALUE (1, 1200)").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["inserted_id"], 1);

        let r = read_free(kdb_sql(
            sess,
            cstr("SELECT * FROM label.race WHERE race_no = 1").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["total_matched"], 1);
        assert_eq!(v["rows"].as_array().unwrap().len(), 1);

        kdb_close(sess);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_sql_explain_reports_full_scan() {
        let path = tmp_kdb("sqlexplain");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        kdb_sql(sess, cstr("INSERT INTO (label.race) (venue) VALUE ('edogawa')").as_ptr());
        let r = read_free(kdb_sql(sess, cstr("EXPLAIN SELECT * FROM label.race WHERE venue = 'edogawa'").as_ptr()));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["rows"][0][0].as_str().unwrap().starts_with("full scan"));

        kdb_close(sess);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_sql_delete_checkpoints() {
        let path = tmp_kdb("sqldel");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        for i in 1..=3 {
            kdb_insert(
                sess,
                cstr(&format!(
                    r#"{{"id":0,"columns":{{"n":{{"type":"integer","value":{i}}}}},"labels":["d20250906"]}}"#
                ))
                .as_ptr(),
            );
        }
        assert_eq!(kdb_count(sess), 3);

        let r = read_free(kdb_sql(sess, cstr("DELETE FROM label.d20250906").as_ptr()));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["deleted_count"], 3);
        assert_eq!(kdb_count(sess), 0);
        kdb_close(sess);

        // checkpoint 済みなので再オープンでも 0 件
        let sess2 = kdb_open(cp.as_ptr());
        assert_eq!(kdb_count(sess2), 0);
        kdb_close(sess2);

        cleanup(&path);
    }

    #[test]
    fn test_kdb_sql_update_persists() {
        let path = tmp_kdb("squpd");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        kdb_sql(
            sess,
            cstr("INSERT INTO (label.race) (race_no, status) VALUE (1, 'open')").as_ptr(),
        );
        let r = read_free(kdb_sql(
            sess,
            cstr("UPDATE label.race SET status = 'closed' WHERE race_no = 1").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["updated_count"], 1);
        kdb_close(sess);

        let sess2 = kdb_open(cp.as_ptr());
        let r = read_free(kdb_sql(
            sess2,
            cstr("SELECT status FROM label.race WHERE race_no = 1").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["rows"][0][0], "closed");
        kdb_close(sess2);

        cleanup(&path);
    }

    #[test]
    fn test_kdb_sql_unsupported_statement() {
        let path = tmp_kdb("sqlbad");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());
        let r = read_free(kdb_sql(sess, cstr("DROP TABLE race").as_ptr()));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("SELECT"));
        kdb_close(sess);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_get_by_label_and_ndjson() {
        let path = tmp_kdb("ndjson");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());

        for i in 1..=5 {
            kdb_insert(
                sess,
                cstr(&format!(
                    r#"{{"id":0,"columns":{{"race_no":{{"type":"integer","value":{i}}},"venue":{{"type":"text","value":"BocaRace"}}}},"labels":["y2024"]}}"#
                ))
                .as_ptr(),
            );
        }
        kdb_insert(
            sess,
            cstr(r#"{"id":0,"columns":{"race_no":{"type":"integer","value":9}},"labels":["y2025"]}"#)
                .as_ptr(),
        );

        // 型タグ付き JSON 配列
        let r = read_free(kdb_get_by_label(sess, cstr("y2024").as_ptr()));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 5);
        assert_eq!(v[0]["columns"]["race_no"]["type"], "integer");

        // フラット NDJSON をファイルへ
        let out = std::env::temp_dir().join(format!("db_ffi_ndjson_{}.ndjson", std::process::id()));
        let out_s = out.to_str().unwrap();
        let n = kdb_get_by_label_ndjson(sess, cstr("y2024").as_ptr(), cstr(out_s).as_ptr());
        assert_eq!(n, 5);

        let body = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 5);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["race_no"], 1);
        assert_eq!(first["venue"], "BocaRace");
        assert_eq!(first["labels"][0], "y2024");

        kdb_close(sess);
        let _ = std::fs::remove_file(&out);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_save_returns_ok() {
        let path = tmp_kdb("save");
        let cp = cstr(&path);
        let sess = kdb_open(cp.as_ptr());
        kdb_insert(
            sess,
            cstr(r#"{"id":0,"columns":{"x":{"type":"integer","value":42}},"labels":["s"]}"#).as_ptr(),
        );
        assert_eq!(kdb_save(sess), 1);
        kdb_close(sess);

        let sess2 = kdb_open(cp.as_ptr());
        assert_eq!(kdb_count(sess2), 1);
        kdb_close(sess2);
        cleanup(&path);
    }

    #[test]
    fn test_kdb_null_pointer_safety() {
        assert_eq!(kdb_count(std::ptr::null_mut()), 0);
        assert_eq!(kdb_save(std::ptr::null_mut()), 0);
        assert_eq!(kdb_insert(std::ptr::null_mut(), cstr("{}").as_ptr()), 0);
        assert_eq!(kdb_insert_many(std::ptr::null_mut(), cstr("[]").as_ptr()), -1);
        kdb_close(std::ptr::null_mut()); // パニックしないこと
        let r = read_free(kdb_sql(std::ptr::null_mut(), cstr("SELECT 1").as_ptr()));
        assert!(r.contains("\"ok\":false"));
    }
}
