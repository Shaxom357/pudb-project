// src/label_handlers.rs
// ラベル管理 API ハンドラー

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use crate::handlers::AppState;
use crate::models::{
    ErrorResponse, RecordResponse,
    AddLabelRequest, LabelSearchRequest, RenameLabelRequest,
    LabelListResponse, LabelSearchResponse, LabelStatDto,
};

// GET /records/{id}/labels
pub async fn list_record_labels(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    let inner = state.read().await;
    match inner.mgr.list_labels(id) {
        Ok(labels) => (StatusCode::OK, Json(serde_json::to_value(labels).unwrap())),
        Err(e) => (StatusCode::NOT_FOUND,
            Json(serde_json::to_value(ErrorResponse::new(e.to_string())).unwrap())),
    }
}

// POST /records/{id}/labels  -- ラベル追加
 pub async fn add_label(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(payload): Json<AddLabelRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    match inner.mgr.add_label(id, &payload.label) {
        Ok(()) => {
            let labels = inner.mgr.list_labels(id).unwrap_or_default();
            if let Err(e) = inner.mgr.db().save(&inner.db_path) {
                eprintln!("[WARN] Failed to save: {}", e);
                inner.logger.db_warn(format!("label save failed: {}", e));
            }
            inner.logger.db_info(format!("label '{}' added to id={}", payload.label, id));
            (StatusCode::OK, Json(serde_json::to_value(labels).unwrap()))
        }
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND
            } else if e.to_string().contains("already attached") {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(serde_json::to_value(ErrorResponse::new(e.to_string())).unwrap()))
        }
    }
}

// DELETE /records/{id}/labels/{label}
pub async fn remove_label(
    State(state): State<AppState>,
    Path((id, label)): Path<(u64, String)>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    match inner.mgr.remove_label(id, &label) {
        Ok(true) => {
            if let Err(e) = inner.mgr.db().save(&inner.db_path) {
                eprintln!("[WARN] Failed to save: {}", e);
                inner.logger.db_warn(format!("label save failed: {}", e));
            }
            inner.logger.db_info(format!("label '{}' removed from id={}", label, id));
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(format!("Label '{}' not found on record {}", label, id)))
        ).into_response(),
        Err(e) => {
            let status = if e.to_string().contains("not found") {
                StatusCode::NOT_FOUND } else { StatusCode::UNPROCESSABLE_ENTITY };
            (status, Json(ErrorResponse::new(e.to_string()))).into_response()
        }
    }
}

// GET /labels  -- 全ラベル一覧＋統計
pub async fn list_all_labels(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    let response = LabelListResponse {
        labels: inner.mgr.list_all_labels(),
        stats: inner.mgr.label_stats().into_iter()
            .map(|s| LabelStatDto { label: s.label, record_count: s.record_count })
            .collect(),
    };
    (StatusCode::OK, Json(response))
}

// POST /labels/search  -- AND/OR検索
pub async fn search_by_labels(
    State(state): State<AppState>,
    Json(payload): Json<LabelSearchRequest>,
) -> impl IntoResponse {
    let inner = state.read().await;
    let label_refs: Vec<&str> = payload.labels.iter().map(|s| s.as_str()).collect();
    let ids = match payload.mode.as_deref().unwrap_or("and") {
        "or" => inner.mgr.find_by_labels_any(&label_refs),
        _    => inner.mgr.find_by_labels_all(&label_refs),
    };
    let records: Vec<RecordResponse> = ids.iter()
        .filter_map(|&id| inner.mgr.db().get(id).ok())
        .map(RecordResponse::from_record)
        .collect();
    let response = LabelSearchResponse {
        mode: payload.mode.unwrap_or_else(|| "and".to_string()),
        matched_count: records.len(),
        records,
    };
    (StatusCode::OK, Json(response))
}

// PUT /labels/rename  -- ラベルリネーム
pub async fn rename_label(
    State(state): State<AppState>,
    Json(payload): Json<RenameLabelRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    match inner.mgr.rename_label(&payload.old_label, &payload.new_label) {
        Ok(count) => {
            if count > 0 {
                if let Err(e) = inner.mgr.db().save(&inner.db_path) {
                    eprintln!("[WARN] Failed to save: {}", e);
                    inner.logger.db_warn(format!("label save failed: {}", e));
                }
            }
            inner.logger.db_info(format!(
                "label renamed '{}' -> '{}' ({} record(s))",
                payload.old_label, payload.new_label, count
            ));
            (StatusCode::OK, Json(serde_json::json!({
                "old_label": payload.old_label,
                "new_label": payload.new_label,
                "updated_count": count
            })))
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": e.to_string() }))),
    }
}
