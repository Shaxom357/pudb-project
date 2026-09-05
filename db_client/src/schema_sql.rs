// src/schema_sql.rs
// 任意スキーマ層（ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA / DESCRIBE /
// SHOW SCHEMAS / VALIDATE LABEL）の専用パーサーと実行ロジック、および定義の永続化。
//
// スキーマは db_engine::Database 上に構築されるが、`.kdb`/JSON の永続化フォーマットには
// 一切手を入れない（既存ファイルとの後方互換を保つため）。index_sql.rs と同じ発想で、
// 定義（ラベルごとのカラム定義・強制の有効/無効）だけを小さなサイドカーファイル
// `<DB_FILE>.schema.json` へ保存し、起動時にそれを読んで db_engine::Database::define_column()/
// enable_schema() で中身を再構築する。
//
// `DEFINE COLUMN` しただけでは強制されない（既定で無効）。`ENABLE SCHEMA` して初めて
// そのラベルの INSERT/UPDATE が検証され始める。これとは別に、DB全体のスキーマ強制を
// 一時停止する `schema_enforcement_enabled` 設定（`GET`/`PUT /settings`、既定 `true`）があり、
// 大量バックフィル時などに個々のラベル設定を変えずに全体を退避できる。
//
// これらの文はレコードストアの構造（スキーマ）を操作する管理操作のため、
// user_sql.rs / index_sql.rs と同様に sql_engine には手を入れず db_client 側で処理する。

use db_engine::{ColumnSchema, DataType, Database, SchemaType};

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaStatement {
    DefineColumn {
        label: String,
        column: String,
        ty: SchemaType,
        not_null: bool,
        default: Option<DataType>,
        unique: bool,
    },
    DropColumn { label: String, column: String },
    EnableSchema { label: String },
    DisableSchema { label: String },
    Describe { label: String },
    Show,
    Validate { label: String },
}

/// execute_schema_statement の結果
pub enum SchemaSqlOutcome {
    /// 表形式の結果（DESCRIBE / SHOW SCHEMAS / VALIDATE LABEL）
    Rows { columns: Vec<String>, rows: Vec<Vec<serde_json::Value>> },
    /// 成功したが返す行はない（DEFINE/DROP COLUMN・ENABLE/DISABLE SCHEMA）
    Ok { message: String },
    /// 失敗。status は HTTP ステータスコード（400 / 404 / 409）。
    Err { status: u16, message: String },
}

// ---------------------------------------------------------------------------
// トークナイザ（user_sql.rs / index_sql.rs とは別に持つ）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Int(i64),
    Float(f64),
    Dot,
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
        if c == '\'' {
            i += 1;
            let mut s = String::new();
            loop {
                if i >= chars.len() { return Err("クォートが閉じられていません".to_string()); }
                if chars[i] == '\'' {
                    if i + 1 < chars.len() && chars[i + 1] == '\'' { s.push('\''); i += 2; continue; }
                    i += 1;
                    break;
                }
                s.push(chars[i]);
                i += 1;
            }
            tokens.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit())) {
            let start = i;
            if c == '-' { i += 1; }
            let mut is_float = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && !is_float)) {
                if chars[i] == '.' { is_float = true; }
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            if is_float {
                tokens.push(Tok::Float(s.parse().map_err(|_| format!("不正な数値です: '{}'", s))?));
            } else {
                tokens.push(Tok::Int(s.parse().map_err(|_| format!("不正な数値です: '{}'", s))?));
            }
            continue;
        }
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
        Tok::Str(s) => format!("'{}'", s),
        Tok::Int(n) => n.to_string(),
        Tok::Float(f) => f.to_string(),
        Tok::Dot => ".".to_string(),
    }
}

// ---------------------------------------------------------------------------
// パース
// ---------------------------------------------------------------------------

/// 先頭のキーワード列を見て、スキーマ管理文かどうかを判定する。
/// 該当しなければ `None`（呼び出し側は通常の SELECT/INSERT/... ディスパッチへ進む）。
pub fn parse_schema_statement(sql: &str) -> Option<Result<SchemaStatement, String>> {
    let trimmed = sql.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    let head_words = head.split_whitespace().collect::<Vec<_>>();

    let is_schema_stmt = matches!(
        head_words.as_slice(),
        ["ALTER", "LABEL", ..]
            | ["DESCRIBE", ..]
            | ["SHOW", "SCHEMAS", ..]
            | ["SHOW", "SCHEMA", ..]
            | ["VALIDATE", "LABEL", ..]
    );
    if !is_schema_stmt {
        return None;
    }
    Some(parse_inner(sql))
}

fn parse_inner(sql: &str) -> Result<SchemaStatement, String> {
    let toks = tokenize(sql)?;
    if toks.is_empty() {
        return Err("空の文です".to_string());
    }

    if is_word(&toks[0], "SHOW") {
        if toks.len() != 2 || !(is_word(&toks[1], "SCHEMAS") || is_word(&toks[1], "SCHEMA")) {
            return Err("構文エラー: 'SHOW SCHEMAS' と記述してください".to_string());
        }
        return Ok(SchemaStatement::Show);
    }

    if is_word(&toks[0], "DESCRIBE") {
        let mut idx = 1;
        let label = parse_label_name(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(SchemaStatement::Describe { label });
    }

    if is_word(&toks[0], "VALIDATE") {
        let mut idx = 1;
        expect_word(&toks, &mut idx, "LABEL")?;
        let label = parse_label_name(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(SchemaStatement::Validate { label });
    }

    if is_word(&toks[0], "ALTER") {
        let mut idx = 1;
        expect_word(&toks, &mut idx, "LABEL")?;
        let label = parse_label_name(&toks, &mut idx)?;

        match toks.get(idx) {
            Some(t) if is_word(t, "DEFINE") => {
                idx += 1;
                expect_word(&toks, &mut idx, "COLUMN")?;
                let column = take_word(&toks, &mut idx)?;
                expect_word(&toks, &mut idx, "TYPE")?;
                let ty_name = take_word(&toks, &mut idx)?;
                let ty = schema_type_from_name(&ty_name)?;

                let mut not_null = false;
                let mut default = None;
                let mut unique = false;
                loop {
                    match toks.get(idx) {
                        Some(t) if is_word(t, "NOT") => {
                            idx += 1;
                            expect_word(&toks, &mut idx, "NULL")?;
                            not_null = true;
                        }
                        Some(t) if is_word(t, "DEFAULT") => {
                            idx += 1;
                            default = Some(parse_default_value(&toks, &mut idx, ty)?);
                        }
                        Some(t) if is_word(t, "UNIQUE") => {
                            idx += 1;
                            unique = true;
                        }
                        _ => break,
                    }
                }
                expect_end(&toks, idx)?;
                return Ok(SchemaStatement::DefineColumn { label, column, ty, not_null, default, unique });
            }
            Some(t) if is_word(t, "DROP") => {
                idx += 1;
                expect_word(&toks, &mut idx, "COLUMN")?;
                let column = take_word(&toks, &mut idx)?;
                expect_end(&toks, idx)?;
                return Ok(SchemaStatement::DropColumn { label, column });
            }
            Some(t) if is_word(t, "ENABLE") => {
                idx += 1;
                expect_word(&toks, &mut idx, "SCHEMA")?;
                expect_end(&toks, idx)?;
                return Ok(SchemaStatement::EnableSchema { label });
            }
            Some(t) if is_word(t, "DISABLE") => {
                idx += 1;
                expect_word(&toks, &mut idx, "SCHEMA")?;
                expect_end(&toks, idx)?;
                return Ok(SchemaStatement::DisableSchema { label });
            }
            Some(t) => return Err(format!(
                "構文エラー: 'DEFINE COLUMN' / 'DROP COLUMN' / 'ENABLE SCHEMA' / 'DISABLE SCHEMA' が必要です（'{}' が見つかりました）",
                tok_repr(t)
            )),
            None => return Err("構文エラー: 'DEFINE COLUMN' / 'DROP COLUMN' / 'ENABLE SCHEMA' / 'DISABLE SCHEMA' が必要です".to_string()),
        }
    }

    Err("サポートされていないスキーマ管理文です".to_string())
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

/// `label.<name>` または `<name>`（`label.` 省略の糖衣構文。他の SQL 構文と同じ書き味にする
/// ためだけの表記）からラベル名を読む。
fn parse_label_name(toks: &[Tok], idx: &mut usize) -> Result<String, String> {
    if let (Some(t), Some(Tok::Dot)) = (toks.get(*idx), toks.get(*idx + 1)) {
        if is_word(t, "label") { *idx += 2; }
    }
    take_word(toks, idx)
}

fn schema_type_from_name(name: &str) -> Result<SchemaType, String> {
    match name.to_uppercase().as_str() {
        "TEXT" | "STRING" | "VARCHAR" => Ok(SchemaType::Text),
        "INTEGER" | "INT"             => Ok(SchemaType::Integer),
        "FLOAT" | "DOUBLE" | "REAL"   => Ok(SchemaType::Float),
        "BOOLEAN" | "BOOL"            => Ok(SchemaType::Boolean),
        other => Err(format!("不明な型です: '{}'（TEXT/INTEGER/FLOAT/BOOLEAN のいずれか）", other)),
    }
}

fn schema_type_name(ty: SchemaType) -> &'static str {
    match ty {
        SchemaType::Text    => "text",
        SchemaType::Integer => "integer",
        SchemaType::Float   => "float",
        SchemaType::Boolean => "boolean",
    }
}

fn parse_literal(toks: &[Tok], idx: &mut usize) -> Result<DataType, String> {
    match toks.get(*idx) {
        Some(Tok::Str(s))   => { *idx += 1; Ok(DataType::Text(s.clone())) }
        Some(Tok::Int(n))   => { *idx += 1; Ok(DataType::Integer(*n)) }
        Some(Tok::Float(f)) => { *idx += 1; Ok(DataType::Float(*f)) }
        Some(t) if is_word(t, "TRUE")  => { *idx += 1; Ok(DataType::Boolean(true)) }
        Some(t) if is_word(t, "FALSE") => { *idx += 1; Ok(DataType::Boolean(false)) }
        Some(t) => Err(format!("DEFAULT の値が不正です（'{}'）", tok_repr(t))),
        None => Err("DEFAULT の値が指定されていません".to_string()),
    }
}

/// `DEFAULT` の後ろの値を読み、宣言した `TYPE` と矛盾しないか確認する
/// （`Integer` の値は `Float` 宣言にも適合させる。db_engine 側の `value_matches_type` と
/// 同じ判定基準だが、クレートが違うため個別に実装している）。
fn parse_default_value(toks: &[Tok], idx: &mut usize, ty: SchemaType) -> Result<DataType, String> {
    let lit = parse_literal(toks, idx)?;
    let matches_type = matches!((&lit, ty),
        (DataType::Text(_), SchemaType::Text)
        | (DataType::Integer(_), SchemaType::Integer)
        | (DataType::Float(_), SchemaType::Float)
        | (DataType::Integer(_), SchemaType::Float)
        | (DataType::Boolean(_), SchemaType::Boolean)
    );
    if !matches_type {
        return Err(format!("DEFAULT の値の型が TYPE {} と一致しません", schema_type_name(ty)));
    }
    Ok(lit)
}

fn data_type_to_json(v: &DataType) -> serde_json::Value {
    match v {
        DataType::Text(s)    => serde_json::Value::String(s.clone()),
        DataType::Integer(n) => serde_json::Value::Number((*n).into()),
        DataType::Float(f)   => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        DataType::Boolean(b) => serde_json::Value::Bool(*b),
        DataType::Null       => serde_json::Value::Null,
    }
}

fn json_to_default(ty: SchemaType, v: &serde_json::Value) -> Option<DataType> {
    if v.is_null() { return None; }
    match ty {
        SchemaType::Text    => v.as_str().map(|s| DataType::Text(s.to_string())),
        SchemaType::Integer => v.as_i64().map(DataType::Integer),
        SchemaType::Float   => v.as_f64().map(DataType::Float),
        SchemaType::Boolean => v.as_bool().map(DataType::Boolean),
    }
}

// ---------------------------------------------------------------------------
// 実行
// ---------------------------------------------------------------------------

/// スキーマ管理文を実行する。呼び出し側（sql_handlers.rs）で権限
/// （MANAGE_USERS 相当。CREATE USER / CREATE INDEX 等と同じ管理者専用の操作として扱う）を
/// 確認済みの前提。
pub fn execute_schema_statement(db: &mut Database, stmt: SchemaStatement) -> SchemaSqlOutcome {
    match stmt {
        SchemaStatement::Show => {
            let columns = vec!["label".to_string(), "enabled".to_string(), "column_count".to_string()];
            let rows = db.list_schema_labels().into_iter().map(|label| {
                let (enabled, cols) = db.describe_label(&label).unwrap_or((false, Vec::new()));
                vec![
                    serde_json::Value::String(label),
                    serde_json::Value::Bool(enabled),
                    serde_json::Value::Number(cols.len().into()),
                ]
            }).collect();
            SchemaSqlOutcome::Rows { columns, rows }
        }

        SchemaStatement::Describe { label } => match db.describe_label(&label) {
            None => SchemaSqlOutcome::Err {
                status: 404,
                message: format!("label '{}' has no schema defined", label),
            },
            Some((enabled, cols)) => {
                let columns = vec![
                    "column".to_string(), "type".to_string(), "not_null".to_string(),
                    "default".to_string(), "unique".to_string(), "schema_enabled".to_string(),
                ];
                let rows = cols.into_iter().map(|(name, def)| vec![
                    serde_json::Value::String(name),
                    serde_json::Value::String(schema_type_name(def.ty).to_string()),
                    serde_json::Value::Bool(def.not_null),
                    def.default.as_ref().map(data_type_to_json).unwrap_or(serde_json::Value::Null),
                    serde_json::Value::Bool(def.unique),
                    serde_json::Value::Bool(enabled),
                ]).collect();
                SchemaSqlOutcome::Rows { columns, rows }
            }
        },

        SchemaStatement::Validate { label } => {
            let columns = vec!["violation".to_string()];
            let rows = db.validate_label(&label).into_iter()
                .map(|v| vec![serde_json::Value::String(v)])
                .collect();
            SchemaSqlOutcome::Rows { columns, rows }
        }

        SchemaStatement::DefineColumn { label, column, ty, not_null, default, unique } => {
            match db.define_column(&label, &column, ColumnSchema { ty, not_null, default, unique }) {
                Ok(()) => SchemaSqlOutcome::Ok {
                    message: format!("ラベル '{}' のカラム '{}' にスキーマを定義しました（ENABLE SCHEMA するまで強制されません）", label, column),
                },
                Err(e) => SchemaSqlOutcome::Err { status: 409, message: e.to_string() },
            }
        }

        SchemaStatement::DropColumn { label, column } => match db.drop_column_schema(&label, &column) {
            Ok(()) => SchemaSqlOutcome::Ok {
                message: format!("ラベル '{}' のカラム '{}' のスキーマ定義を削除しました", label, column),
            },
            Err(e) => SchemaSqlOutcome::Err { status: 404, message: e.to_string() },
        },

        SchemaStatement::EnableSchema { label } => {
            db.enable_schema(&label);
            SchemaSqlOutcome::Ok { message: format!("ラベル '{}' のスキーマ強制を有効化しました", label) }
        }

        SchemaStatement::DisableSchema { label } => {
            db.disable_schema(&label);
            SchemaSqlOutcome::Ok { message: format!("ラベル '{}' のスキーマ強制を無効化しました", label) }
        }
    }
}

// ---------------------------------------------------------------------------
// 定義の永続化（サイドカー JSON。.kdb/JSON 本体のフォーマットには手を入れない）
// ---------------------------------------------------------------------------

/// スキーマ定義の保存先パス（`<DB_FILE>.schema.json`）
pub fn schema_sidecar_path(db_path: &str) -> String {
    format!("{}.schema.json", db_path)
}

/// 現在のスキーマ定義（ラベルごとのカラム定義・有効/無効）をサイドカーファイルへ保存する。
/// `DEFINE`/`DROP COLUMN`・`ENABLE`/`DISABLE SCHEMA` が成功するたびに呼ぶ想定。
/// DB全体の一時停止スイッチ（`schema_enforcement_enabled`）はここには含めない
/// （サーバー再起動のたびに `true` へ戻す設計のため、意図的に永続化しない）。
pub fn save_schema_definitions(db_path: &str, db: &Database) -> std::io::Result<()> {
    let mut labels_json = serde_json::Map::new();
    for label in db.list_schema_labels() {
        let Some((enabled, cols)) = db.describe_label(&label) else { continue; };
        let mut columns_json = serde_json::Map::new();
        for (name, def) in cols {
            columns_json.insert(name, serde_json::json!({
                "type": schema_type_name(def.ty),
                "not_null": def.not_null,
                "default": def.default.as_ref().map(data_type_to_json).unwrap_or(serde_json::Value::Null),
                "unique": def.unique,
            }));
        }
        labels_json.insert(label, serde_json::json!({ "enabled": enabled, "columns": columns_json }));
    }
    let body = serde_json::json!({ "labels": serde_json::Value::Object(labels_json) });
    std::fs::write(schema_sidecar_path(db_path), serde_json::to_string_pretty(&body)?)
}

/// 起動時に呼ぶ: サイドカーファイルの定義に従ってスキーマを再構築する
/// （二次インデックスと同じく、実データは永続化せず定義から作り直す）。
pub fn rebuild_schemas_from_sidecar(db_path: &str, db: &mut Database) {
    let path = schema_sidecar_path(db_path);
    let Ok(content) = std::fs::read_to_string(&path) else { return; };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else { return; };
    let Some(labels) = json.get("labels").and_then(|v| v.as_object()) else { return; };

    for (label, label_val) in labels {
        if let Some(columns) = label_val.get("columns").and_then(|v| v.as_object()) {
            for (col, col_val) in columns {
                let Some(ty_name) = col_val.get("type").and_then(|v| v.as_str()) else { continue; };
                let Ok(ty) = schema_type_from_name(ty_name) else { continue; };
                let not_null = col_val.get("not_null").and_then(|v| v.as_bool()).unwrap_or(false);
                let unique = col_val.get("unique").and_then(|v| v.as_bool()).unwrap_or(false);
                let default = col_val.get("default").and_then(|v| json_to_default(ty, v));
                let _ = db.define_column(label, col, ColumnSchema { ty, not_null, default, unique });
            }
        }
        if label_val.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
            db.enable_schema(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_engine::Record;

    #[test]
    fn test_parse_define_column_minimal() {
        let stmt = parse_schema_statement("ALTER LABEL race DEFINE COLUMN date TYPE text").unwrap().unwrap();
        assert_eq!(stmt, SchemaStatement::DefineColumn {
            label: "race".to_string(), column: "date".to_string(), ty: SchemaType::Text,
            not_null: false, default: None, unique: false,
        });
    }

    #[test]
    fn test_parse_define_column_all_modifiers() {
        let stmt = parse_schema_statement(
            "ALTER LABEL race DEFINE COLUMN odds TYPE float NOT NULL DEFAULT 0.0 UNIQUE"
        ).unwrap().unwrap();
        assert_eq!(stmt, SchemaStatement::DefineColumn {
            label: "race".to_string(), column: "odds".to_string(), ty: SchemaType::Float,
            not_null: true, default: Some(DataType::Float(0.0)), unique: true,
        });
    }

    #[test]
    fn test_parse_define_column_bare_label_sugar() {
        let stmt = parse_schema_statement("ALTER LABEL label.race DEFINE COLUMN date TYPE text").unwrap().unwrap();
        assert_eq!(stmt, SchemaStatement::DefineColumn {
            label: "race".to_string(), column: "date".to_string(), ty: SchemaType::Text,
            not_null: false, default: None, unique: false,
        });
    }

    #[test]
    fn test_parse_default_type_mismatch_is_error() {
        assert!(parse_schema_statement("ALTER LABEL race DEFINE COLUMN age TYPE integer DEFAULT 'abc'").unwrap().is_err());
    }

    #[test]
    fn test_parse_default_integer_for_float_column_is_ok() {
        let stmt = parse_schema_statement("ALTER LABEL race DEFINE COLUMN odds TYPE float DEFAULT 0").unwrap().unwrap();
        assert_eq!(stmt, SchemaStatement::DefineColumn {
            label: "race".to_string(), column: "odds".to_string(), ty: SchemaType::Float,
            not_null: false, default: Some(DataType::Integer(0)), unique: false,
        });
    }

    #[test]
    fn test_parse_drop_column() {
        let stmt = parse_schema_statement("ALTER LABEL race DROP COLUMN date").unwrap().unwrap();
        assert_eq!(stmt, SchemaStatement::DropColumn { label: "race".to_string(), column: "date".to_string() });
    }

    #[test]
    fn test_parse_enable_disable_schema() {
        assert_eq!(
            parse_schema_statement("ALTER LABEL race ENABLE SCHEMA").unwrap().unwrap(),
            SchemaStatement::EnableSchema { label: "race".to_string() }
        );
        assert_eq!(
            parse_schema_statement("ALTER LABEL race DISABLE SCHEMA").unwrap().unwrap(),
            SchemaStatement::DisableSchema { label: "race".to_string() }
        );
    }

    #[test]
    fn test_parse_describe() {
        assert_eq!(
            parse_schema_statement("DESCRIBE label.race").unwrap().unwrap(),
            SchemaStatement::Describe { label: "race".to_string() }
        );
        assert_eq!(
            parse_schema_statement("DESCRIBE race").unwrap().unwrap(),
            SchemaStatement::Describe { label: "race".to_string() }
        );
    }

    #[test]
    fn test_parse_show_schemas() {
        assert_eq!(parse_schema_statement("SHOW SCHEMAS").unwrap().unwrap(), SchemaStatement::Show);
    }

    #[test]
    fn test_parse_validate_label() {
        assert_eq!(
            parse_schema_statement("VALIDATE LABEL race").unwrap().unwrap(),
            SchemaStatement::Validate { label: "race".to_string() }
        );
    }

    #[test]
    fn test_parse_schema_statement_returns_none_for_unrelated_sql() {
        assert!(parse_schema_statement("SELECT * FROM label.employee").is_none());
        assert!(parse_schema_statement("ALTER USER 'x' IDENTIFIED BY 'y'").is_none());
        assert!(parse_schema_statement("CREATE INDEX ON label.race (venue)").is_none());
    }

    #[test]
    fn test_execute_define_then_enable_then_insert_is_validated() {
        let mut db = Database::new();
        let outcome = execute_schema_statement(&mut db, SchemaStatement::DefineColumn {
            label: "race".to_string(), column: "date".to_string(), ty: SchemaType::Text,
            not_null: true, default: None, unique: false,
        });
        assert!(matches!(outcome, SchemaSqlOutcome::Ok { .. }));

        // 未 ENABLE のうちは強制されない
        let mut r = Record::new(0);
        r.add_label("race");
        assert!(db.insert(r).is_ok());

        execute_schema_statement(&mut db, SchemaStatement::EnableSchema { label: "race".to_string() });
        let mut r2 = Record::new(0);
        r2.add_label("race");
        assert!(db.insert(r2).is_err());
    }

    #[test]
    fn test_execute_describe_unknown_label_errors() {
        let mut db = Database::new();
        let outcome = execute_schema_statement(&mut db, SchemaStatement::Describe { label: "race".to_string() });
        assert!(matches!(outcome, SchemaSqlOutcome::Err { status: 404, .. }));
    }

    #[test]
    fn test_execute_show_schemas_lists_labels() {
        let mut db = Database::new();
        db.define_column("race", "date", ColumnSchema {
            ty: SchemaType::Text, not_null: true, default: None, unique: false,
        }).unwrap();
        let outcome = execute_schema_statement(&mut db, SchemaStatement::Show);
        match outcome {
            SchemaSqlOutcome::Rows { columns, rows } => {
                assert_eq!(columns, vec!["label".to_string(), "enabled".to_string(), "column_count".to_string()]);
                assert_eq!(rows, vec![vec![
                    serde_json::Value::String("race".to_string()),
                    serde_json::Value::Bool(false),
                    serde_json::Value::Number(1.into()),
                ]]);
            }
            _ => panic!("expected Rows"),
        }
    }

    #[test]
    fn test_save_and_load_schema_definitions_roundtrip() {
        let tmp = std::env::temp_dir().join(format!(
            "schema_sql_test_{}_{}.kdb", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        let db_path = tmp.to_string_lossy().to_string();
        let _ = std::fs::remove_file(schema_sidecar_path(&db_path));

        let mut db = Database::new();
        db.define_column("race", "date", ColumnSchema {
            ty: SchemaType::Text, not_null: true, default: None, unique: false,
        }).unwrap();
        db.define_column("racer", "racer_id", ColumnSchema {
            ty: SchemaType::Integer, not_null: true, default: Some(DataType::Integer(0)), unique: true,
        }).unwrap();
        db.enable_schema("race");
        save_schema_definitions(&db_path, &db).unwrap();

        let mut db2 = Database::new();
        rebuild_schemas_from_sidecar(&db_path, &mut db2);

        assert_eq!(db2.list_schema_labels(), vec!["race".to_string(), "racer".to_string()]);
        let (race_enabled, race_cols) = db2.describe_label("race").unwrap();
        assert!(race_enabled);
        assert_eq!(race_cols, vec![("date".to_string(), ColumnSchema {
            ty: SchemaType::Text, not_null: true, default: None, unique: false,
        })]);
        let (racer_enabled, racer_cols) = db2.describe_label("racer").unwrap();
        assert!(!racer_enabled);
        assert_eq!(racer_cols, vec![("racer_id".to_string(), ColumnSchema {
            ty: SchemaType::Integer, not_null: true, default: Some(DataType::Integer(0)), unique: true,
        })]);
        // UNIQUE 指定だったカラムのインデックスも再構築されている
        assert!(db2.has_index("racer_id"));

        let _ = std::fs::remove_file(schema_sidecar_path(&db_path));
    }

    #[test]
    fn test_load_schema_definitions_missing_file_is_noop() {
        let mut db = Database::new();
        rebuild_schemas_from_sidecar("/nonexistent/dir/does_not_exist.kdb", &mut db);
        assert!(db.list_schema_labels().is_empty());
    }
}
