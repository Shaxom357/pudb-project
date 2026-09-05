// src/handlers.rs
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use db_engine::{Record, DataType};
use db_engine::kdb_store::{KdbError, KdbFile};
use db_engine::PersistError;
use dynamic_label_management::LabelManager;

use crate::auth::AuthState;
use crate::logging::Logger;
use crate::models::{
    CreateRecordRequest, UpdateRecordRequest,
    RecordResponse, ErrorResponse,
};

pub struct AppStateInner {
    pub mgr:     LabelManager,
    pub db_path: String,
    pub kdb:     Option<KdbFile>,
    /// HTTPリクエスト（/records, /labels 系のREST API）を受け付けるかどうか。
    /// false の場合、データ操作はSQL経由(/sql)のみ許可される。
    pub http_api_enabled: bool,
    pub logger: Arc<Logger>,
    pub auth: AuthState,
}

pub type AppState = Arc<RwLock<AppStateInner>>;

pub(crate) fn auto_save(state: &mut AppStateInner) {
    if let Some(kdb) = state.kdb.as_mut() {
        let records: Vec<Record> = state.mgr.db().list_all();
        let next_id = state.mgr.db().next_id();
        match kdb.compact(&records, next_id) {
            Ok(offsets) => {
                state.mgr.db_mut().sync_record_offsets(&records, &offsets);
                state.logger.db_info(format!("KDB saved to '{}' ({} record(s))", state.db_path, records.len()));
            }
            Err(e) => {
                eprintln!("[WARN] KDB compact failed: {}", e);
                if let KdbError::Io(io_err) = &e {
                    state.logger.db_io_error("KDB compact failed", io_err);
                } else {
                    state.logger.db_warn(format!("KDB compact failed: {}", e));
                }
            }
        }
    } else if let Err(e) = state.mgr.db().save(&state.db_path) {
        eprintln!("[WARN] Failed to save DB: {}", e);
        if let PersistError::Io(io_err) = &e {
            state.logger.db_io_error("DB save failed", io_err);
        } else {
            state.logger.db_warn(format!("DB save failed: {}", e));
        }
    } else {
        state.logger.db_info(format!("DB saved to '{}'", state.db_path));
    }
}

pub async fn list_records(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.read().await;
    let records: Vec<RecordResponse> = inner.mgr.db()
        .list_all().iter().map(|r| RecordResponse::from_record(r)).collect();
    (StatusCode::OK, Json(records))
}

pub async fn get_record(State(state): State<AppState>, Path(id): Path<u64>) -> impl IntoResponse {
    let inner = state.read().await;
    match inner.mgr.db().get(id) {
        Ok(record) => (StatusCode::OK,
            Json(serde_json::to_value(RecordResponse::from_record(&record)).unwrap())),
        Err(e) => (StatusCode::NOT_FOUND,
            Json(serde_json::to_value(ErrorResponse::new(e.to_string())).unwrap())),
    }
}

pub async fn get_records_by_label(State(state): State<AppState>, Path(label): Path<String>) -> impl IntoResponse {
    let inner = state.read().await;
    let records: Vec<RecordResponse> = inner.mgr.db()
        .get_by_label(&label).iter().map(|r| RecordResponse::from_record(r)).collect();
    (StatusCode::OK, Json(records))
}

pub async fn create_record(
    State(state): State<AppState>,
    Json(payload): Json<CreateRecordRequest>,
) -> impl IntoResponse {
    let mut record = Record::new(payload.id);
    for (col, val) in payload.columns { record.set(col, DataType::from(val)); }
    for label in payload.labels { record.add_label(label); }
    let mut inner = state.write().await;
    // kdb モード: WAL 追記（高速 O(1)）/ JSON モード: 通常保存
    // Rustの借用チェッカー対策: kdb と mgr を別々に取り出す
    let id_result = {
        // kdb フィールドを一時的に取り出してから返す
        let mut kdb_taken = inner.kdb.take();
        let result = inner.mgr.db_mut().insert_fast(record, kdb_taken.as_mut());
        inner.kdb = kdb_taken;
        result
    };
    match id_result {
        Ok(id) => {
            let response = RecordResponse::from_record(&inner.mgr.db().get(id).unwrap());
            if inner.kdb.is_none() { auto_save(&mut inner); }
            inner.logger.db_info(format!("record created id={}", id));
            (StatusCode::CREATED, Json(serde_json::to_value(response).unwrap()))
        }
        Err(e) => {
            inner.logger.db_warn(format!("record create failed: {}", e));
            (StatusCode::CONFLICT,
                Json(serde_json::to_value(ErrorResponse::new(e.to_string())).unwrap()))
        }
    }
}

pub async fn update_record(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(payload): Json<UpdateRecordRequest>,
) -> impl IntoResponse {
    let mut new_record = Record::new(0);
    for (col, val) in payload.columns { new_record.set(col, DataType::from(val)); }
    for label in payload.labels { new_record.add_label(label); }
    let mut inner = state.write().await;
    match inner.mgr.db_mut().update(id, new_record) {
        Ok(()) => {
            let response = RecordResponse::from_record(&inner.mgr.db().get(id).unwrap());
            auto_save(&mut inner);
            inner.logger.db_info(format!("record updated id={}", id));
            (StatusCode::OK, Json(serde_json::to_value(response).unwrap()))
        }
        Err(e) => {
            inner.logger.db_warn(format!("record update failed id={}: {}", id, e));
            (StatusCode::NOT_FOUND,
                Json(serde_json::to_value(ErrorResponse::new(e.to_string())).unwrap()))
        }
    }
}

pub async fn delete_record(State(state): State<AppState>, Path(id): Path<u64>) -> impl IntoResponse {
    let mut inner = state.write().await;
    match inner.mgr.db_mut().delete(id) {
        Ok(()) => {
            auto_save(&mut inner);
            inner.logger.db_info(format!("record deleted id={}", id));
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            inner.logger.db_warn(format!("record delete failed id={}: {}", id, e));
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new(e.to_string()))).into_response()
        }
    }
}
