// src/sql_handlers.rs
// SQL クエリ実行 API ハンドラー

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::handlers::{auto_save, AppState};
use sql_engine::{run_select, run_insert, CellValue};

// ---------------------------------------------------------------------------
// リクエスト / レスポンス DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SqlQueryRequest {
    /// 実行するSQL文
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SqlQueryResponse {
    /// 成功・失敗
    pub ok: bool,
    /// カラム名リスト（成功時）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// 行データ（成功時）各行は columns と同順の値リスト
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<serde_json::Value>>,
    /// LIMIT前のヒット件数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_matched: Option<usize>,
    /// 返却行数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned: Option<usize>,
    /// INSERTで発行されたレコードID（成功時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted_id: Option<u64>,
    /// INSERTで付与されたラベル（成功時）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// エラーメッセージ（失敗時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 実行したSQL（エコーバック）
    pub query: String,
}

impl SqlQueryResponse {
    fn error(query: String, message: impl Into<String>) -> Self {
        SqlQueryResponse {
            ok: false, columns: vec![], rows: vec![], total_matched: None, returned: None,
            inserted_id: None, labels: vec![], error: Some(message.into()), query,
        }
    }
}

fn cell_to_json(cell: &CellValue) -> serde_json::Value {
    match cell {
        CellValue::Text(s)    => serde_json::Value::String(s.clone()),
        CellValue::Integer(n) => serde_json::Value::Number((*n).into()),
        CellValue::Float(f)   => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CellValue::Boolean(b) => serde_json::Value::Bool(*b),
        CellValue::Null       => serde_json::Value::Null,
    }
}

// ---------------------------------------------------------------------------
// POST /sql  -- SQL クエリを実行する
// ---------------------------------------------------------------------------

pub async fn execute_sql(
    State(state): State<AppState>,
    Json(payload): Json<SqlQueryRequest>,
) -> impl IntoResponse {
    let query = payload.query.trim().to_string();

    // 現時点では SELECT / INSERT のみサポート
    let upper = query.to_uppercase();
    let trimmed = upper.trim_start();

    if trimmed.starts_with("SELECT") {
        let inner = state.read().await;
        let db = inner.mgr.db();

        return match run_select(db, &query) {
            Ok(result) => {
                let returned = result.rows.len();
                let rows: Vec<Vec<serde_json::Value>> = result.rows.iter()
                    .map(|row| row.iter().map(cell_to_json).collect())
                    .collect();
                (StatusCode::OK, Json(SqlQueryResponse {
                    ok: true,
                    columns: result.columns,
                    rows,
                    total_matched: Some(result.total_matched),
                    returned: Some(returned),
                    inserted_id: None,
                    labels: vec![],
                    error: None,
                    query,
                }))
            }
            Err(e) => (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string()))),
        };
    }

    if trimmed.starts_with("INSERT") {
        let mut inner = state.write().await;
        let insert_result = run_insert(inner.mgr.db_mut(), &query);

        return match insert_result {
            Ok(result) => {
                auto_save(&mut inner);
                // 挿入後のレコードを読み直し、使用したカラム順に値を並べて返す
                let record = inner.mgr.db().get(result.id).unwrap();
                let row: Vec<serde_json::Value> = result.columns.iter()
                    .map(|col| record.columns.get(col)
                        .map(CellValue::from_data_type)
                        .map(|c| cell_to_json(&c))
                        .unwrap_or(serde_json::Value::Null))
                    .collect();
                (StatusCode::CREATED, Json(SqlQueryResponse {
                    ok: true,
                    columns: result.columns,
                    rows: vec![row],
                    total_matched: None,
                    returned: Some(1),
                    inserted_id: Some(result.id),
                    labels: result.labels,
                    error: None,
                    query,
                }))
            }
            Err(e) => (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string()))),
        };
    }

    (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(
        query, "Only SELECT and INSERT statements are supported in this version.",
    )))
}
