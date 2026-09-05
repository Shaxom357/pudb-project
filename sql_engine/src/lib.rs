// sql_engine/src/lib.rs

pub mod ast;
pub mod executor;
pub mod parser;

pub use executor::{
    execute_select, execute_insert, execute_insert_fast, execute_update, execute_delete,
    explain_select, CellValue, QueryResult, InsertResult, InsertExecError, UpdateResult,
    UpdateExecError, DeleteResult, DeleteExecError, PlanDescription,
};
pub use parser::{parse_select, parse_insert, parse_update, parse_delete, parse_explain, ParseError};
pub use ast::{SelectStatement, InsertStatement, UpdateStatement, DeleteStatement};

/// SQL文字列を受け取り、クエリを実行して結果を返す（SELECT専用）
pub fn run_select(db: &db_engine::Database, sql: &str) -> Result<QueryResult, ParseError> {
    let stmt = parse_select(sql)?;
    Ok(execute_select(db, &stmt))
}

/// `EXPLAIN <SELECT文>` を受け取り、クエリは実行せず実行計画（二次インデックスを
/// 使ったかどうか）だけを返す。
pub fn run_explain(db: &db_engine::Database, sql: &str) -> Result<QueryResult, ParseError> {
    let stmt = parse_explain(sql)?;
    Ok(explain_select(db, &stmt))
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

/// UPDATE実行時の統合エラー（パースエラー or 実行エラー）
#[derive(Debug, PartialEq)]
pub enum UpdateError {
    Parse(ParseError),
    Exec(UpdateExecError),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Parse(e) => write!(f, "{}", e),
            UpdateError::Exec(e)  => write!(f, "{}", e),
        }
    }
}
impl From<ParseError> for UpdateError { fn from(e: ParseError) -> Self { UpdateError::Parse(e) } }
impl From<UpdateExecError> for UpdateError { fn from(e: UpdateExecError) -> Self { UpdateError::Exec(e) } }

/// SQL文字列を受け取り、UPDATEを実行する
pub fn run_update(db: &mut db_engine::Database, sql: &str) -> Result<UpdateResult, UpdateError> {
    let stmt = parse_update(sql)?;
    Ok(execute_update(db, &stmt)?)
}

/// DELETE実行時の統合エラー（パースエラー or 実行エラー）
#[derive(Debug, PartialEq)]
pub enum DeleteError {
    Parse(ParseError),
    Exec(DeleteExecError),
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteError::Parse(e) => write!(f, "{}", e),
            DeleteError::Exec(e)  => write!(f, "{}", e),
        }
    }
}
impl From<ParseError> for DeleteError { fn from(e: ParseError) -> Self { DeleteError::Parse(e) } }
impl From<DeleteExecError> for DeleteError { fn from(e: DeleteExecError) -> Self { DeleteError::Exec(e) } }

/// SQL文字列を受け取り、DELETEを実行する
pub fn run_delete(db: &mut db_engine::Database, sql: &str) -> Result<DeleteResult, DeleteError> {
    let stmt = parse_delete(sql)?;
    Ok(execute_delete(db, &stmt)?)
}
