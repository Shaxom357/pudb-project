// src/info_handlers.rs
// DB情報確認 API ハンドラー

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use crate::handlers::AppState;
use crate::models::DbInfoResponse;

/// GET /db/info  -- DB バージョン・統計情報を返す
pub async fn get_db_info(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    let db      = inner.mgr.db();
    let db_path = &inner.db_path;

    // ストレージモードを判定（拡張子 .kdb = KDB暗号化モード）
    let is_kdb  = db_path.ends_with(".kdb");
    let storage_mode   = if is_kdb { "kdb".to_string() } else { "json".to_string() };
    let storage_format = if is_kdb {
        "KDB Binary (WAL + XChaCha20-Poly1305 encrypted)".to_string()
    } else {
        "JSON (column-oriented snapshot)".to_string()
    };
    let encrypted = is_kdb;

    // データファイルのサイズを取得（存在しない場合は None）
    let db_file_size_bytes = std::fs::metadata(db_path)
        .map(|m| m.len())
        .ok();

    let record_count  = db.count();
    let deleted_count = db.deleted_count();
    let slot_count    = db.slot_count();
    let columns       = db.list_all_columns();
    let labels        = db.list_all_labels();
    let next_id       = db.next_id();

    let info = DbInfoResponse {
        engine_name:        "KAGURA DB Engine".to_string(),
        app_version:        env!("CARGO_PKG_VERSION").to_string(),
        storage_mode,
        storage_format,
        encrypted,
        db_file_path:       db_path.clone(),
        db_file_size_bytes,
        record_count,
        deleted_count,
        slot_count,
        label_count:        labels.len(),
        column_count:       columns.len(),
        columns,
        next_id,
    };

    (StatusCode::OK, Json(info))
}
