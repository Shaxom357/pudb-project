// src/settings_handlers.rs
// アプリケーション設定 API ハンドラー
// 現在の設定項目: HTTPリクエスト(REST API)の受付オンオフ、スキーマ強制の全体スイッチ

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
    /// スキーマ強制の全体スイッチ（既定 `true`）。`false` にすると、各ラベルの
    /// `ENABLE SCHEMA` 状態に関わらず一時的に全てのスキーマ検証を止める
    /// （大量バックフィル時の緊急退避用）。サーバー再起動のたびに `true` へ戻る
    /// （永続化しない）。
    pub schema_enforcement_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub http_api_enabled: Option<bool>,
    #[serde(default)]
    pub schema_enforcement_enabled: Option<bool>,
}

/// GET /settings -- 現在の設定を取得
pub async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    (StatusCode::OK, Json(SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
        schema_enforcement_enabled: inner.mgr.db().is_schema_enforcement_enabled(),
    }))
}

/// PUT /settings -- 設定を更新（指定したフィールドだけを変更する。省略したフィールドは
/// 現在値を維持する）
pub async fn update_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    if let Some(v) = payload.http_api_enabled {
        inner.http_api_enabled = v;
    }
    if let Some(v) = payload.schema_enforcement_enabled {
        inner.mgr.db_mut().set_schema_enforcement_enabled(v);
    }
    (StatusCode::OK, Json(SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
        schema_enforcement_enabled: inner.mgr.db().is_schema_enforcement_enabled(),
    }))
}
