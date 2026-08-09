// db_ffi/src/lib.rs
// Python (ctypes) から呼び出せる C FFI インターフェース
//
// メモリ管理の帰則:
//   - Rust 側で确保した文字列は db_string_free() で解放する
//   - Database ポインタは db_free() で解放する

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use db_engine::{Database, DataType, Record};

// ---------------------------------------------------------------------------
// 内部ヘルパー
// ---------------------------------------------------------------------------

/// C 文字列を Rust &str に変換するヘルパー
/// NULL または無効な UTF-8 の場合は None を返す
fn c_str_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() { return None; }
    unsafe { CStr::from_ptr(ptr).to_str().ok() }
}

/// Rust String をヒープ確保した C 文字列に変換する
/// 呼び出し元は db_string_free() で解放する必要がある
fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// JSON 文字列から Record を構築する
/// JSON 形式: {"id":0, "columns":{"name":{"type":"text","value":"Alice"}}, "labels":["tag1"]}
fn json_to_record(json: &str) -> Option<Record> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let id = v["id"].as_u64().unwrap_or(0);
    let mut record = Record::new(id);

    if let Some(cols) = v["columns"].as_object() {
        for (col_name, col_val) in cols {
            let dtype = match col_val["type"].as_str()? {
                "text"    => DataType::Text(col_val["value"].as_str()?.to_string()),
                "integer" => DataType::Integer(col_val["value"].as_i64()?),
                "float"   => DataType::Float(col_val["value"].as_f64()?),
                "boolean" => DataType::Boolean(col_val["value"].as_bool()?),
                "null"    => DataType::Null,
                _         => return None,
            };
            record.set(col_name.clone(), dtype);
        }
    }
    if let Some(labels) = v["labels"].as_array() {
        for label in labels {
            if let Some(l) = label.as_str() {
                record.add_label(l);
            }
        }
    }
    Some(record)
}

/// Record を JSON 値に変換する
fn record_to_json(record: &Record) -> serde_json::Value {
    let mut cols = serde_json::Map::new();
    for (k, v) in &record.columns {
        let val = match v {
            DataType::Text(s)    => serde_json::json!({"type":"text",    "value": s}),
            DataType::Integer(n) => serde_json::json!({"type":"integer", "value": n}),
            DataType::Float(f)   => serde_json::json!({"type":"float",   "value": f}),
            DataType::Boolean(b) => serde_json::json!({"type":"boolean", "value": b}),
            DataType::Null       => serde_json::json!({"type":"null",    "value": null}),
        };
        cols.insert(k.clone(), val);
    }
    serde_json::json!({
        "id":      record.id,
        "columns": cols,
        "labels":  record.labels,
    })
}

// ---------------------------------------------------------------------------
// 公開 FFI 関数
// ---------------------------------------------------------------------------

/// Database を新規作成し、ポインタを返す
/// Python: db = lib.db_new()
#[unsafe(no_mangle)]
pub extern "C" fn db_new() -> *mut Database {
    Box::into_raw(Box::new(Database::new()))
}

/// Database ポインタを解放する
/// Python: lib.db_free(db)
#[unsafe(no_mangle)]
pub extern "C" fn db_free(ptr: *mut Database) {
    if !ptr.is_null() {
        unsafe { drop(Box::from_raw(ptr)); }
    }
}

/// Rust 側で確保した文字列を解放する
/// db_get / db_get_by_label / db_list_all の戻り値を使用後に必ず呼ぶこと
/// Python: lib.db_string_free(result)
#[unsafe(no_mangle)]
pub extern "C" fn db_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)); }
    }
}

/// レコードを挿入する
/// json: {"id":0, "columns":{...}, "labels":[...]}
/// 戻り値: 発行されたレコードID（0=失敗）
/// Python: record_id = lib.db_insert(db, json_bytes)
#[unsafe(no_mangle)]
pub extern "C" fn db_insert(ptr: *mut Database, json: *const c_char) -> u64 {
    if ptr.is_null() { return 0; }
    let json_str = match c_str_to_str(json) { Some(s) => s, None => return 0 };
    let record   = match json_to_record(json_str) { Some(r) => r, None => return 0 };
    unsafe { (*ptr).insert(record).unwrap_or(0) }
}

/// ID でレコードを取得する
/// 戻り値: JSON 文字列（見つからなければ NULL）→ db_string_free() で解放すること
/// Python: ptr = lib.db_get(db, record_id); s = ctypes.string_at(ptr).decode()
#[unsafe(no_mangle)]
pub extern "C" fn db_get(ptr: *mut Database, id: u64) -> *mut c_char {
    if ptr.is_null() { return std::ptr::null_mut(); }
    match unsafe { (*ptr).get(id) } {
        Ok(record) => to_c_string(record_to_json(record).to_string()),
        Err(_)     => std::ptr::null_mut(),
    }
}

/// レコードを削除する
/// 戻り値: 1=成功, 0=失敗
/// Python: ok = lib.db_delete(db, record_id)
#[unsafe(no_mangle)]
pub extern "C" fn db_delete(ptr: *mut Database, id: u64) -> i32 {
    if ptr.is_null() { return 0; }
    match unsafe { (*ptr).delete(id) } {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// レコードを更新する
/// json: {"columns":{...}, "labels":[...]}
/// 戻り値: 1=成功, 0=失敗
/// Python: ok = lib.db_update(db, record_id, json_bytes)
#[unsafe(no_mangle)]
pub extern "C" fn db_update(ptr: *mut Database, id: u64, json: *const c_char) -> i32 {
    if ptr.is_null() { return 0; }
    let json_str = match c_str_to_str(json) { Some(s) => s, None => return 0 };
    let record   = match json_to_record(json_str) { Some(r) => r, None => return 0 };
    match unsafe { (*ptr).update(id, record) } {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// ラベルでレコードを検索する
/// 戻り値: JSON 配列文字列 → db_string_free() で解放すること
/// Python: ptr = lib.db_get_by_label(db, b"dept:eng")
#[unsafe(no_mangle)]
pub extern "C" fn db_get_by_label(ptr: *mut Database, label: *const c_char) -> *mut c_char {
    if ptr.is_null() { return std::ptr::null_mut(); }
    let label_str = match c_str_to_str(label) { Some(s) => s, None => return std::ptr::null_mut() };
    let records: Vec<serde_json::Value> = unsafe {
        (*ptr).get_by_label(label_str).iter().map(|r| record_to_json(r)).collect()
    };
    to_c_string(serde_json::Value::Array(records).to_string())
}

/// 全レコードを取得する
/// 戻り値: JSON 配列文字列 → db_string_free() で解放すること
/// Python: ptr = lib.db_list_all(db)
#[unsafe(no_mangle)]
pub extern "C" fn db_list_all(ptr: *mut Database) -> *mut c_char {
    if ptr.is_null() { return std::ptr::null_mut(); }
    let records: Vec<serde_json::Value> = unsafe {
        (*ptr).list_all().iter().map(|r| record_to_json(r)).collect()
    };
    to_c_string(serde_json::Value::Array(records).to_string())
}

/// 有効レコード数を返す
/// Python: n = lib.db_count(db)
#[unsafe(no_mangle)]
pub extern "C" fn db_count(ptr: *mut Database) -> u64 {
    if ptr.is_null() { return 0; }
    unsafe { (*ptr).count() as u64 }
}

/// データベースを JSON ファイルに保存する
/// 戻り値: 1=成功, 0=失敗
/// Python: ok = lib.db_save(db, b"/path/to/db.json")
#[unsafe(no_mangle)]
pub extern "C" fn db_save(ptr: *mut Database, path: *const c_char) -> i32 {
    if ptr.is_null() { return 0; }
    let path_str = match c_str_to_str(path) { Some(s) => s, None => return 0 };
    match unsafe { (*ptr).save(path_str) } {
        Ok(()) => 1,
        Err(_) => 0,
    }
}

/// JSON ファイルから Database をロードし、ポインタを返す
/// 失敗時は NULL を返す
/// Python: db = lib.db_load(b"/path/to/db.json")
#[unsafe(no_mangle)]
pub extern "C" fn db_load(path: *const c_char) -> *mut Database {
    let path_str = match c_str_to_str(path) { Some(s) => s, None => return std::ptr::null_mut() };
    match Database::load(path_str) {
        Ok(db) => Box::into_raw(Box::new(db)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 全ラベル一覧を取得する
/// 戻り値: JSON 配列文字列 ["label1", "label2", ...] → db_string_free() で解放
/// Python: ptr = lib.db_list_labels(db)
#[unsafe(no_mangle)]
pub extern "C" fn db_list_labels(ptr: *mut Database) -> *mut c_char {
    if ptr.is_null() { return std::ptr::null_mut(); }
    let labels: Vec<serde_json::Value> = unsafe {
        (*ptr).list_all_labels().into_iter().map(serde_json::Value::String).collect()
    };
    to_c_string(serde_json::Value::Array(labels).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn make_cstr(s: &str) -> CString { CString::new(s).unwrap() }

    #[test]
    fn test_db_new_and_free() {
        let db = db_new();
        assert!(!db.is_null());
        db_free(db);
    }

    #[test]
    fn test_db_insert_and_count() {
        let db = db_new();
        let json = make_cstr(r#"{"id":0,"columns":{"name":{"type":"text","value":"Alice"}},"labels":["dept:eng"]}"#);
        let id = db_insert(db, json.as_ptr());
        assert_eq!(id, 1);
        assert_eq!(db_count(db), 1);
        db_free(db);
    }

    #[test]
    fn test_db_get() {
        let db = db_new();
        let json = make_cstr(r#"{"id":0,"columns":{"name":{"type":"text","value":"Bob"}},"labels":[]}"#);
        let id = db_insert(db, json.as_ptr());
        let result = db_get(db, id);
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["columns"]["name"]["value"], "Bob");
        db_string_free(result);
        db_free(db);
    }

    #[test]
    fn test_db_get_not_found_returns_null() {
        let db = db_new();
        let result = db_get(db, 9999);
        assert!(result.is_null());
        db_free(db);
    }

    #[test]
    fn test_db_delete() {
        let db = db_new();
        let json = make_cstr(r#"{"id":0,"columns":{},"labels":[]}"#);
        let id = db_insert(db, json.as_ptr());
        assert_eq!(db_delete(db, id), 1);
        assert_eq!(db_count(db), 0);
        assert_eq!(db_delete(db, id), 0); // 再削除は失敗
        db_free(db);
    }

    #[test]
    fn test_db_update() {
        let db = db_new();
        let json = make_cstr(r#"{"id":0,"columns":{"score":{"type":"integer","value":50}},"labels":["old"]}"#);
        let id = db_insert(db, json.as_ptr());
        let upd = make_cstr(r#"{"id":0,"columns":{"score":{"type":"integer","value":99}},"labels":["new"]}"#);
        assert_eq!(db_update(db, id, upd.as_ptr()), 1);
        let result = db_get(db, id);
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["columns"]["score"]["value"], 99);
        db_string_free(result);
        db_free(db);
    }

    #[test]
    fn test_db_get_by_label() {
        let db = db_new();
        for name in ["Alice", "Bob"] {
            let json = CString::new(format!(
                r#"{{"id":0,"columns":{{"name":{{"type":"text","value":"{}"}}}},"labels":["team:a"]}}"#, name
            )).unwrap();
            db_insert(db, json.as_ptr());
        }
        let label = make_cstr("team:a");
        let result = db_get_by_label(db, label.as_ptr());
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        db_string_free(result);
        db_free(db);
    }

    #[test]
    fn test_db_save_and_load() {
        let tmp = std::env::temp_dir().join("db_ffi_test.json");
        let path = make_cstr(tmp.to_str().unwrap());

        let db = db_new();
        let json = make_cstr(r#"{"id":0,"columns":{"x":{"type":"integer","value":42}},"labels":["saved"]}"#);
        let id = db_insert(db, json.as_ptr());
        assert_eq!(db_save(db, path.as_ptr()), 1);
        db_free(db);

        let db2 = db_load(path.as_ptr());
        assert!(!db2.is_null());
        assert_eq!(db_count(db2), 1);
        let result = db_get(db2, id);
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v["columns"]["x"]["value"], 42);
        db_string_free(result);
        db_free(db2);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_db_list_all() {
        let db = db_new();
        db_insert(db, make_cstr(r#"{"id":0,"columns":{},"labels":[]}"#).as_ptr());
        db_insert(db, make_cstr(r#"{"id":0,"columns":{},"labels":[]}"#).as_ptr());
        let result = db_list_all(db);
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        db_string_free(result);
        db_free(db);
    }

    #[test]
    fn test_db_list_labels() {
        let db = db_new();
        db_insert(db, make_cstr(r#"{"id":0,"columns":{},"labels":["env:prod","region:ap"]}"#).as_ptr());
        let result = db_list_labels(db);
        let s = unsafe { CStr::from_ptr(result).to_str().unwrap() };
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        let labels: Vec<&str> = v.as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        assert!(labels.contains(&"env:prod"));
        assert!(labels.contains(&"region:ap"));
        db_string_free(result);
        db_free(db);
    }
}
