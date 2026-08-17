// sql_engine/src/lib.rs

pub mod ast;
pub mod executor;
pub mod parser;

pub use executor::{execute_select, execute_insert, execute_insert_fast, CellValue, QueryResult, InsertResult, InsertExecError};
pub use parser::{parse_select, parse_insert, ParseError};
pub use ast::{SelectStatement, InsertStatement};

/// SQL文字列を受け取り、クエリを実行して結果を返す（SELECT専用）
pub fn run_select(db: &db_engine::Database, sql: &str) -> Result<QueryResult, ParseError> {
    let stmt = parse_select(sql)?;
    Ok(execute_select(db, &stmt))
}

/// INSERT実行時の統合エラー（パースエラー or 実行エラー）
#[derive(Debug, PartialEq)]
pub enum InsertError {
    Parse(ParseError),
    Exec(InsertExecError),
}

impl std::fmt::Display for InsertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsertError::Parse(e) => write!(f, "{}", e),
            InsertError::Exec(e)  => write!(f, "{}", e),
        }
    }
}
impl From<ParseError> for InsertError { fn from(e: ParseError) -> Self { InsertError::Parse(e) } }
impl From<InsertExecError> for InsertError { fn from(e: InsertExecError) -> Self { InsertError::Exec(e) } }

/// SQL文字列を受け取り、INSERTを実行する
pub fn run_insert(db: &mut db_engine::Database, sql: &str) -> Result<InsertResult, InsertError> {
    let stmt = parse_insert(sql)?;
    Ok(execute_insert(db, &stmt)?)
}

/// SQL文字列を受け取り、INSERTをWAL追記(insert_fast)で実行する。.kdbモード用の高速パス。
pub fn run_insert_fast(
    db: &mut db_engine::Database,
    kdb: Option<&mut db_engine::kdb_store::KdbFile>,
    sql: &str,
) -> Result<InsertResult, InsertError> {
    let stmt = parse_insert(sql)?;
    Ok(execute_insert_fast(db, kdb, &stmt)?)
}
