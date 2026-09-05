// src/index_sql.rs
// 二次インデックス管理系 SQL 文（CREATE INDEX / DROP INDEX / SHOW INDEXES）の
// 専用パーサーと実行ロジック、および定義の永続化。
//
// インデックスは db_engine::Database 上に構築されるが、`.kdb`/JSON の永続化フォーマットには
// 一切手を入れない（既存ファイルとの後方互換を保つため）。代わりに `auth.json` と同じ発想で、
// 定義（どのカラムにインデックスを張ったか）だけを小さなサイドカーファイル
// `<DB_FILE>.indexes.json` へ保存し、起動時にそれを読んで db_engine::Database::create_index()
// で中身を再構築する。
//
// これらの文はレコードストアの構造（インデックス）を操作する管理操作のため、
// user_sql.rs（CREATE USER 等）と同様に sql_engine には手を入れず db_client 側で処理する。
// `/sql` ハンドラーの冒頭で parse_index_statement() を呼び、Some が返ったら
// 通常の SELECT/INSERT/... ディスパッチには回さずここで完結させる。

use db_engine::{Database, DatabaseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStatement {
    Create { column: String, if_not_exists: bool },
    Drop { column: String, if_exists: bool },
    Show,
}

/// execute_index_statement の結果
pub enum IndexSqlOutcome {
    /// 表形式の結果（SHOW INDEXES）
    Rows { columns: Vec<String>, rows: Vec<Vec<serde_json::Value>> },
    /// 成功したが返す行はない（CREATE / DROP）
    Ok { message: String },
    /// 失敗。status は HTTP ステータスコード（400 / 403 / 404 / 409）。
    Err { status: u16, message: String },
}

// ---------------------------------------------------------------------------
// トークナイザ（CREATE/DROP INDEX 専用。user_sql.rs のものとは別に持つ）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Dot,
    LParen,
    RParen,
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() { i += 1; continue; }
        if c == ';' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() { i += 1; }
            if i < chars.len() {
                return Err("複数の文をまとめて実行することはできません".to_string());
            }
            break;
        }
        if c == '.' { tokens.push(Tok::Dot); i += 1; continue; }
        if c == '(' { tokens.push(Tok::LParen); i += 1; continue; }
        if c == ')' { tokens.push(Tok::RParen); i += 1; continue; }
        if c.is_alphanumeric() || c == '_' {
            let mut w = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                w.push(chars[i]);
                i += 1;
            }
            tokens.push(Tok::Word(w));
            continue;
        }
        return Err(format!("不正な文字です: '{}'", c));
    }
    Ok(tokens)
}

fn is_word(t: &Tok, w: &str) -> bool {
    matches!(t, Tok::Word(s) if s.eq_ignore_ascii_case(w))
}

fn tok_repr(t: &Tok) -> String {
    match t {
        Tok::Word(w) => w.clone(),
        Tok::Dot => ".".to_string(),
        Tok::LParen => "(".to_string(),
        Tok::RParen => ")".to_string(),
    }
}

// ---------------------------------------------------------------------------
// パース
// ---------------------------------------------------------------------------

/// 先頭のキーワード列を見て、インデックス管理文かどうかを判定する。
/// 該当しなければ `None`（呼び出し側は通常の SELECT/INSERT/... ディスパッチへ進む）。
pub fn parse_index_statement(sql: &str) -> Option<Result<IndexStatement, String>> {
    let trimmed = sql.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    let head_words = head.split_whitespace().collect::<Vec<_>>();

    let is_index_stmt = matches!(
        head_words.as_slice(),
        ["CREATE", "INDEX", ..] | ["DROP", "INDEX", ..] | ["SHOW", "INDEXES", ..] | ["SHOW", "INDEX", ..]
    );
    if !is_index_stmt {
        return None;
    }
    Some(parse_inner(sql))
}

fn parse_inner(sql: &str) -> Result<IndexStatement, String> {
    let toks = tokenize(sql)?;
    if toks.is_empty() {
        return Err("空の文です".to_string());
    }

    if is_word(&toks[0], "SHOW") {
        if toks.len() != 2 || !(is_word(&toks[1], "INDEXES") || is_word(&toks[1], "INDEX")) {
            return Err("構文エラー: 'SHOW INDEXES' と記述してください".to_string());
        }
        return Ok(IndexStatement::Show);
    }

    if is_word(&toks[0], "CREATE") {
        // CREATE INDEX [IF NOT EXISTS] ON [label.]<name> (<column>)
        if toks.len() < 2 || !is_word(&toks[1], "INDEX") {
            return Err("構文エラー: 'CREATE INDEX ...' と記述してください".to_string());
        }
        let mut idx = 2;
        let mut if_not_exists = false;
        if toks.len() >= idx + 3 && is_word(&toks[idx], "IF") && is_word(&toks[idx + 1], "NOT") && is_word(&toks[idx + 2], "EXISTS") {
            if_not_exists = true;
            idx += 3;
        }
        expect_word(&toks, &mut idx, "ON")?;
        let column = parse_label_and_column(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(IndexStatement::Create { column, if_not_exists });
    }

    if is_word(&toks[0], "DROP") {
        // DROP INDEX [IF EXISTS] ON [label.]<name> (<column>)
        if toks.len() < 2 || !is_word(&toks[1], "INDEX") {
            return Err("構文エラー: 'DROP INDEX ...' と記述してください".to_string());
        }
        let mut idx = 2;
        let mut if_exists = false;
        if toks.len() >= idx + 2 && is_word(&toks[idx], "IF") && is_word(&toks[idx + 1], "EXISTS") {
            if_exists = true;
            idx += 2;
        }
        expect_word(&toks, &mut idx, "ON")?;
        let column = parse_label_and_column(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(IndexStatement::Drop { column, if_exists });
    }

    Err("サポートされていないインデックス管理文です".to_string())
}

fn expect_word(toks: &[Tok], idx: &mut usize, w: &str) -> Result<(), String> {
    match toks.get(*idx) {
        Some(t) if is_word(t, w) => { *idx += 1; Ok(()) }
        Some(t) => Err(format!("構文エラー: '{}' が必要です（'{}' が見つかりました）", w, tok_repr(t))),
        None => Err(format!("構文エラー: '{}' が必要です", w)),
    }
}

fn expect_end(toks: &[Tok], idx: usize) -> Result<(), String> {
    if idx == toks.len() { Ok(()) } else { Err("構文エラー: 余分なトークンがあります".to_string()) }
}

fn take_word(toks: &[Tok], idx: &mut usize) -> Result<String, String> {
    match toks.get(*idx) {
        Some(Tok::Word(w)) => { *idx += 1; Ok(w.clone()) }
        Some(t) => Err(format!("識別子が必要です（'{}' が見つかりました）", tok_repr(t))),
        None => Err("識別子が必要です".to_string()),
    }
}

/// `label.<name> (<column>)` または `<name> (<column>)`（`label.` 省略の糖衣構文。
/// 他の SQL 構文と同じ書き味にするためだけの表記で、ラベル名自体はインデックスの
/// 実体には使わない。KAGURA DB はスキーマレスで、二次インデックスはラベルをまたいで
/// 「そのカラム名を持つ全レコード」を対象にするカラム単位の構造のため）を読み、
/// カラム名を返す。
fn parse_label_and_column(toks: &[Tok], idx: &mut usize) -> Result<String, String> {
    if let (Some(t), Some(Tok::Dot)) = (toks.get(*idx), toks.get(*idx + 1)) {
        if is_word(t, "label") {
            *idx += 2;
        }
    }
    let _label_name = take_word(toks, idx)?;
    match toks.get(*idx) {
        Some(Tok::LParen) => { *idx += 1; }
        Some(t) => return Err(format!("構文エラー: ラベル名の後に '(column)' が必要です（'{}' が見つかりました）", tok_repr(t))),
        None => return Err("構文エラー: ラベル名の後に '(column)' が必要です".to_string()),
    }
    let column = take_word(toks, idx)?;
    match toks.get(*idx) {
        Some(Tok::RParen) => { *idx += 1; Ok(column) }
        Some(t) => Err(format!("構文エラー: カラム名の後に ')' が必要です（'{}' が見つかりました）", tok_repr(t))),
        None => Err("構文エラー: カラム名の後に ')' が必要です".to_string()),
    }
}

// ---------------------------------------------------------------------------
// 実行
// ---------------------------------------------------------------------------

/// インデックス管理文を実行する。呼び出し側（sql_handlers.rs）で権限
/// （MANAGE_USERS 相当。CREATE USER 等と同じ管理者専用の操作として扱う）を確認済みの前提。
pub fn execute_index_statement(db: &mut Database, stmt: IndexStatement) -> IndexSqlOutcome {
    match stmt {
        IndexStatement::Show => {
            let columns = vec!["column".to_string()];
            let rows = db.list_indexes().into_iter()
                .map(|c| vec![serde_json::Value::String(c)])
                .collect();
            IndexSqlOutcome::Rows { columns, rows }
        }
        IndexStatement::Create { column, if_not_exists } => match db.create_index(&column) {
            Ok(()) => IndexSqlOutcome::Ok {
                message: format!("カラム '{}' にインデックスを作成しました", column),
            },
            Err(DatabaseError::IndexAlreadyExists(_)) if if_not_exists => IndexSqlOutcome::Ok {
                message: format!("カラム '{}' には既にインデックスがあります（IF NOT EXISTS）", column),
            },
            Err(e) => IndexSqlOutcome::Err { status: 409, message: e.to_string() },
        },
        IndexStatement::Drop { column, if_exists } => match db.drop_index(&column) {
            Ok(()) => IndexSqlOutcome::Ok {
                message: format!("カラム '{}' のインデックスを削除しました", column),
            },
            Err(DatabaseError::IndexNotFound(_)) if if_exists => IndexSqlOutcome::Ok {
                message: format!("カラム '{}' にインデックスはありませんでした（IF EXISTS）", column),
            },
            Err(e) => IndexSqlOutcome::Err { status: 404, message: e.to_string() },
        },
    }
}

// ---------------------------------------------------------------------------
// 定義の永続化（サイドカー JSON。.kdb/JSON 本体のフォーマットには手を入れない）
// ---------------------------------------------------------------------------

/// インデックス定義の保存先パス（`<DB_FILE>.indexes.json`）
pub fn indexes_sidecar_path(db_path: &str) -> String {
    format!("{}.indexes.json", db_path)
}

/// 現在のインデックス定義（カラム名一覧）をサイドカーファイルへ保存する。
/// CREATE INDEX / DROP INDEX が成功するたびに呼ぶ想定。
pub fn save_index_definitions(db_path: &str, db: &Database) -> std::io::Result<()> {
    let body = serde_json::json!({ "indexes": db.list_indexes() });
    std::fs::write(indexes_sidecar_path(db_path), serde_json::to_string_pretty(&body)?)
}

/// サイドカーファイルからインデックス定義（カラム名一覧）を読み込む。
/// ファイルが無い・壊れている場合は空を返す（インデックス無しの通常起動として扱う。
/// エラーにはしない）。
pub fn load_index_definitions(db_path: &str) -> Vec<String> {
    let path = indexes_sidecar_path(db_path);
    let Ok(content) = std::fs::read_to_string(&path) else { return Vec::new(); };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else { return Vec::new(); };
    json.get("indexes")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// 起動時に呼ぶ: サイドカーファイルの定義に従ってインデックスの中身を再構築する
/// （`label_index` 等と同じく、実データは永続化せず既存レコードから作り直す）。
pub fn rebuild_indexes_from_sidecar(db_path: &str, db: &mut Database) {
    for column in load_index_definitions(db_path) {
        // 万一定義が重複していても create_index の IndexAlreadyExists は無視してよい
        let _ = db.create_index(&column);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_engine::{DataType, Record};

    #[test]
    fn test_parse_create_index() {
        let stmt = parse_index_statement("CREATE INDEX ON label.race (venue)").unwrap().unwrap();
        assert_eq!(stmt, IndexStatement::Create { column: "venue".to_string(), if_not_exists: false });
    }

    #[test]
    fn test_parse_create_index_if_not_exists() {
        let stmt = parse_index_statement("CREATE INDEX IF NOT EXISTS ON label.race (venue)").unwrap().unwrap();
        assert_eq!(stmt, IndexStatement::Create { column: "venue".to_string(), if_not_exists: true });
    }

    #[test]
    fn test_parse_create_index_bare_label_sugar() {
        // label. を省略した糖衣構文（他のSQL文と同じ）でも書ける
        let stmt = parse_index_statement("CREATE INDEX ON race (venue)").unwrap().unwrap();
        assert_eq!(stmt, IndexStatement::Create { column: "venue".to_string(), if_not_exists: false });
    }

    #[test]
    fn test_parse_drop_index() {
        let stmt = parse_index_statement("DROP INDEX ON label.race (venue)").unwrap().unwrap();
        assert_eq!(stmt, IndexStatement::Drop { column: "venue".to_string(), if_exists: false });
    }

    #[test]
    fn test_parse_drop_index_if_exists() {
        let stmt = parse_index_statement("DROP INDEX IF EXISTS ON label.race (venue)").unwrap().unwrap();
        assert_eq!(stmt, IndexStatement::Drop { column: "venue".to_string(), if_exists: true });
    }

    #[test]
    fn test_parse_show_indexes() {
        assert_eq!(parse_index_statement("SHOW INDEXES").unwrap().unwrap(), IndexStatement::Show);
        assert_eq!(parse_index_statement("SHOW INDEX").unwrap().unwrap(), IndexStatement::Show);
    }

    #[test]
    fn test_parse_index_statement_returns_none_for_unrelated_sql() {
        assert!(parse_index_statement("SELECT * FROM label.employee").is_none());
        assert!(parse_index_statement("CREATE USER 'x' IDENTIFIED BY 'y'").is_none());
    }

    #[test]
    fn test_parse_rejects_missing_parens() {
        assert!(parse_index_statement("CREATE INDEX ON label.race venue").unwrap().is_err());
    }

    #[test]
    fn test_execute_create_and_drop_index() {
        let mut db = Database::new();
        let mut r = Record::new(0);
        r.set("venue", DataType::Text("edogawa".to_string()));
        db.insert(r).unwrap();

        let outcome = execute_index_statement(&mut db, IndexStatement::Create {
            column: "venue".to_string(), if_not_exists: false,
        });
        assert!(matches!(outcome, IndexSqlOutcome::Ok { .. }));
        assert!(db.has_index("venue"));

        let outcome = execute_index_statement(&mut db, IndexStatement::Drop {
            column: "venue".to_string(), if_exists: false,
        });
        assert!(matches!(outcome, IndexSqlOutcome::Ok { .. }));
        assert!(!db.has_index("venue"));
    }

    #[test]
    fn test_execute_create_index_twice_without_if_not_exists_errors() {
        let mut db = Database::new();
        db.create_index("venue").unwrap();
        let outcome = execute_index_statement(&mut db, IndexStatement::Create {
            column: "venue".to_string(), if_not_exists: false,
        });
        assert!(matches!(outcome, IndexSqlOutcome::Err { status: 409, .. }));
    }

    #[test]
    fn test_execute_create_index_twice_with_if_not_exists_succeeds() {
        let mut db = Database::new();
        db.create_index("venue").unwrap();
        let outcome = execute_index_statement(&mut db, IndexStatement::Create {
            column: "venue".to_string(), if_not_exists: true,
        });
        assert!(matches!(outcome, IndexSqlOutcome::Ok { .. }));
    }

    #[test]
    fn test_execute_drop_nonexistent_without_if_exists_errors() {
        let mut db = Database::new();
        let outcome = execute_index_statement(&mut db, IndexStatement::Drop {
            column: "venue".to_string(), if_exists: false,
        });
        assert!(matches!(outcome, IndexSqlOutcome::Err { status: 404, .. }));
    }

    #[test]
    fn test_execute_show_indexes_lists_sorted_columns() {
        let mut db = Database::new();
        db.create_index("venue").unwrap();
        db.create_index("age").unwrap();
        let outcome = execute_index_statement(&mut db, IndexStatement::Show);
        match outcome {
            IndexSqlOutcome::Rows { columns, rows } => {
                assert_eq!(columns, vec!["column".to_string()]);
                assert_eq!(rows, vec![
                    vec![serde_json::Value::String("age".to_string())],
                    vec![serde_json::Value::String("venue".to_string())],
                ]);
            }
            _ => panic!("expected Rows"),
        }
    }

    #[test]
    fn test_save_and_load_index_definitions_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "index_sql_test_{}_{}.kdb", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        let db_path = tmp.to_string_lossy().to_string();
        let _ = std::fs::remove_file(indexes_sidecar_path(&db_path));

        let mut db = Database::new();
        db.create_index("venue").unwrap();
        db.create_index("age").unwrap();
        save_index_definitions(&db_path, &db).unwrap();

        let mut db2 = Database::new();
        rebuild_indexes_from_sidecar(&db_path, &mut db2);
        assert_eq!(db2.list_indexes(), vec!["age".to_string(), "venue".to_string()]);

        let _ = std::fs::remove_file(indexes_sidecar_path(&db_path));
    }

    #[test]
    fn test_load_index_definitions_missing_file_returns_empty() {
        let path = "/nonexistent/dir/does_not_exist.kdb";
        assert!(load_index_definitions(path).is_empty());
    }
}
