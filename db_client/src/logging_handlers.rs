// src/logging_handlers.rs
// 保管されたログの閲覧 API ハンドラー

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::handlers::AppState;

#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    /// 取得する件数（新しい順）。既定 200、上限 2000。
    pub lines: Option<usize>,
}

/// GET /logs -- 保管されたログを新しい順に返す
pub async fn get_logs(
    State(state): State<AppState>,
    Query(q): Query<LogsQuery>,
) -> impl IntoResponse {
    let inner = state.read().await;
    let n = q.lines.unwrap_or(200).min(2000);
    let entries = inner.logger.recent(n);
    (StatusCode::OK, Json(entries))
}
