// src/settings_handlers.rs
// アプリケーション設定 API ハンドラー
// 現在の設定項目: HTTPリクエスト(REST API)の受付オンオフ

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::handlers::AppState;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    /// /records /labels 系のREST APIを受け付けるかどうか。
    /// false の場合、データ操作はSQL経由(/sql)のみ許可される。
    pub http_api_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub http_api_enabled: bool,
}

/// GET /settings -- 現在の設定を取得
pub async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    (StatusCode::OK, Json(SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
    }))
}

/// PUT /settings -- 設定を更新
pub async fn update_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    inner.http_api_enabled = payload.http_api_enabled;
    (StatusCode::OK, Json(SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
    }))
}
