// sql_engine/src/lib.rs

pub mod ast;
pub mod executor;
pub mod parser;

pub use executor::{execute_select, CellValue, QueryResult};
pub use parser::{parse_select, ParseError};
pub use ast::SelectStatement;

/// SQL文字列を受け取り、クエリを実行して結果を返す（SELECT専用）
pub fn run_select(db: &db_engine::Database, sql: &str) -> Result<QueryResult, ParseError> {
    let stmt = parse_select(sql)?;
    Ok(execute_select(db, &stmt))
}
