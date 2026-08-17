// src/settings_handlers.rs
// アプリケーション設定 API ハンドラー
// 現在の設定項目:
//   - HTTPリクエスト(REST API)の受付オンオフ
//   - メモリ使用量の上限(割合 or 絶対値)。既定は OS搭載メモリの60%

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::handlers::AppState;
use crate::handlers::AppStateInner;
use crate::memory::{self, MemoryLimitConfig, MemoryLimitMode};
use crate::models::ErrorResponse;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    /// /records /labels 系のREST APIを受け付けるかどうか。
    /// false の場合、データ操作はSQL経由(/sql)のみ許可される。
    pub http_api_enabled: bool,
    /// OS搭載メモリ量(バイト)。取得できない環境では null
    pub os_total_memory_bytes: Option<u64>,
    /// メモリ上限の指定方式: "percent" | "absolute"
    pub memory_limit_mode: String,
    /// mode = "percent" の場合に使用する割合(1〜100)
    pub memory_limit_percent: u32,
    /// mode = "absolute" の場合に使用するバイト数
    pub memory_limit_absolute_bytes: u64,
    /// 現在の設定から計算された実効上限(バイト)。Percentモードで OS搭載メモリ量が
    /// 不明な場合は null
    pub memory_limit_effective_bytes: Option<u64>,
    /// 現在のプロセスメモリ使用量(RSS, バイト)。取得できない環境では null
    pub memory_usage_bytes: Option<u64>,
    /// 実効上限に対する現在の消費率(0.0〜)。上限・使用量のいずれかが不明な場合は null
    pub memory_usage_ratio: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub http_api_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemoryLimitRequest {
    /// "percent" | "absolute"
    pub mode: String,
    /// mode = "percent" の場合に指定(1〜100)
    pub percent: Option<u32>,
    /// mode = "absolute" の場合に指定(バイト数、1以上)
    pub absolute_bytes: Option<u64>,
}

fn build_settings_response(inner: &AppStateInner) -> SettingsResponse {
    let effective = inner.memory_limit.effective_limit_bytes(inner.os_total_memory_bytes);
    let usage = memory::process_rss_bytes();
    let ratio = match (effective, usage) {
        (Some(limit), Some(usage)) if limit > 0 => Some(usage as f64 / limit as f64),
        _ => None,
    };
    SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
        os_total_memory_bytes: inner.os_total_memory_bytes,
        memory_limit_mode: match inner.memory_limit.mode {
            MemoryLimitMode::Percent => "percent".to_string(),
            MemoryLimitMode::Absolute => "absolute".to_string(),
        },
        memory_limit_percent: inner.memory_limit.percent,
        memory_limit_absolute_bytes: inner.memory_limit.absolute_bytes,
        memory_limit_effective_bytes: effective,
        memory_usage_bytes: usage,
        memory_usage_ratio: ratio,
    }
}

/// GET /settings -- 現在の設定を取得
pub async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    (StatusCode::OK, Json(build_settings_response(&inner)))
}

/// PUT /settings -- 設定を更新(HTTPリクエスト受付オンオフ)
pub async fn update_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    inner.http_api_enabled = payload.http_api_enabled;
    (StatusCode::OK, Json(build_settings_response(&inner)))
}

/// PUT /settings/memory -- メモリ使用量の上限設定を更新
pub async fn update_memory_limit(
    State(state): State<AppState>,
    Json(payload): Json<UpdateMemoryLimitRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;

    let new_limit = match payload.mode.as_str() {
        "percent" => {
            let percent = match payload.percent {
                Some(p) => p,
                None => return (StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("mode=percentの場合、percentの指定が必要です"))).into_response(),
            };
            if percent < 1 || percent > 100 {
                return (StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("percentは1〜100の範囲で指定してください"))).into_response();
            }
            MemoryLimitConfig { mode: MemoryLimitMode::Percent, percent, absolute_bytes: inner.memory_limit.absolute_bytes }
        }
        "absolute" => {
            let absolute_bytes = match payload.absolute_bytes {
                Some(b) => b,
                None => return (StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("mode=absoluteの場合、absolute_bytesの指定が必要です"))).into_response(),
            };
            if absolute_bytes == 0 {
                return (StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new("absolute_bytesは1以上を指定してください"))).into_response();
            }
            MemoryLimitConfig { mode: MemoryLimitMode::Absolute, percent: inner.memory_limit.percent, absolute_bytes }
        }
        _ => return (StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("modeは'percent'または'absolute'を指定してください"))).into_response(),
    };

    inner.memory_limit = new_limit;
    if let Err(e) = inner.memory_limit.save(&inner.memory_settings_path) {
        inner.logger.db_warn(format!("メモリ上限設定の保存に失敗しました: {}", e));
    }
    inner.logger.db_info(format!(
        "メモリ上限設定を更新しました: mode={:?} percent={} absolute_bytes={}",
        inner.memory_limit.mode, inner.memory_limit.percent, inner.memory_limit.absolute_bytes
    ));

    (StatusCode::OK, Json(build_settings_response(&inner))).into_response()
}
