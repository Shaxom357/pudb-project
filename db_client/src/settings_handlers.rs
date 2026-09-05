// src/settings_handlers.rs
// アプリケーション設定 API ハンドラー
// 設定項目: HTTPリクエスト(REST API)の受付オンオフ、スキーマ強制の全体スイッチ、
//           「全データオンメモリ」設定（ON/OFF・容量上限）

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use db_engine::{MemoryPolicy, MemorySizeSpec};
use crate::auth_handlers::resolve_actor;
use crate::handlers::AppState;
use crate::memory_settings;

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
    /// 「全データオンメモリ」が有効か（既定 `true` ＝ 無制限）。
    /// `false` にすると `memory_limit` の範囲でオンメモリ量を制限し、
    /// あふれた分は直近未使用の行から `.kdb` へ退避する（アクセス時に読み直す）。
    pub all_in_memory: bool,
    /// 容量上限の指定（`{"kind":"bytes","value":<バイト数>}` または
    /// `{"kind":"percent","value":<%>}`）。`all_in_memory=false` のときだけ意味を持つ。
    pub memory_limit: MemorySizeSpec,
    /// 解決後の上限バイト数（`percent` 指定で搭載メモリ量が取得できなければ `null`）
    pub memory_limit_bytes: Option<u64>,
    /// 現在メモリに載せている列データの推定バイト数
    pub memory_resident_bytes: u64,
    /// メモリに実体を載せている行数 / 有効な行数
    pub memory_resident_rows: usize,
    pub memory_total_rows: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    #[serde(default)]
    pub http_api_enabled: Option<bool>,
    #[serde(default)]
    pub schema_enforcement_enabled: Option<bool>,
    /// 「全データオンメモリ」の有効/無効
    #[serde(default)]
    pub all_in_memory: Option<bool>,
    /// 容量上限（構造化形式）。`{"kind":"bytes","value":n}` / `{"kind":"percent","value":n}`。
    #[serde(default)]
    pub memory_limit: Option<MemorySizeSpec>,
    /// 容量上限（文字列形式。`kdb in-memory-size` から使う）。例: `"500MB"` / `"2GB"` / `"20%"`。
    #[serde(default)]
    pub memory_limit_spec: Option<String>,
}

fn build_response(inner: &crate::handlers::AppStateInner) -> SettingsResponse {
    let policy = inner.mgr.db().memory_policy();
    let stats = inner.mgr.db().memory_stats();
    SettingsResponse {
        http_api_enabled: inner.http_api_enabled,
        schema_enforcement_enabled: inner.mgr.db().is_schema_enforcement_enabled(),
        all_in_memory: policy.all_in_memory,
        memory_limit: policy.limit,
        memory_limit_bytes: stats.limit_bytes,
        memory_resident_bytes: stats.resident_bytes,
        memory_resident_rows: stats.resident_rows,
        memory_total_rows: stats.total_rows,
    }
}

/// GET /settings -- 現在の設定を取得
pub async fn get_settings(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    (StatusCode::OK, Json(serde_json::to_value(build_response(&inner)).unwrap()))
}

/// PUT /settings -- 設定を更新（指定したフィールドだけを変更する。省略したフィールドは
/// 現在値を維持する）。
/// 「全データオンメモリ」設定（`all_in_memory` / `memory_limit`）の変更は管理者ロール
/// （`kagura`）のみ許可し、変更内容はサイドカーファイルへ永続化する。
pub async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;

    if let Some(v) = payload.http_api_enabled {
        inner.http_api_enabled = v;
    }
    if let Some(v) = payload.schema_enforcement_enabled {
        inner.mgr.db_mut().set_schema_enforcement_enabled(v);
    }

    // 文字列形式の上限指定をパースする（不正なら 400）
    let parsed_spec = match &payload.memory_limit_spec {
        Some(s) => match db_engine::memory::parse_size_spec(s) {
            Ok(spec) => Some(spec),
            Err(e) => {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": e })));
            }
        },
        None => None,
    };

    let touches_memory = payload.all_in_memory.is_some()
        || payload.memory_limit.is_some()
        || parsed_spec.is_some();
    if touches_memory {
        let actor = resolve_actor(&mut inner, &headers);
        let is_admin = actor.as_deref().map(|a| inner.auth.is_admin(a)).unwrap_or(false);
        if !is_admin {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "「全データオンメモリ」設定の変更は管理者ロール(kagura)のみ可能です" })),
            );
        }
        if !inner.db_path.ends_with(".kdb") {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "「全データオンメモリ」設定は .kdb モードでのみ変更できます" })),
            );
        }

        let current = inner.mgr.db().memory_policy();
        let new_policy = MemoryPolicy {
            all_in_memory: payload.all_in_memory.unwrap_or(current.all_in_memory),
            limit: parsed_spec.or(payload.memory_limit).unwrap_or(current.limit),
        };
        inner.mgr.db_mut().set_memory_policy(new_policy);

        let db_path = inner.db_path.clone();
        if let Err(e) = memory_settings::save(&db_path, &new_policy) {
            inner.logger.db_warn(format!("memory settings save failed: {}", e));
        }
        inner.logger.db_info(format!(
            "memory policy updated: all_in_memory={}, limit={:?}",
            new_policy.all_in_memory, new_policy.limit
        ));
    }

    (StatusCode::OK, Json(serde_json::to_value(build_response(&inner)).unwrap()))
}
