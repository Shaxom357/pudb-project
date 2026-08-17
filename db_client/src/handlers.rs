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

use crate::logging::Logger;
use crate::memory::{self, MemoryAlertLevel, MemoryLimitConfig};
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
    /// メモリ使用量の上限設定(割合 or 絶対値)。既定は OS搭載メモリの60%。
    pub memory_limit: MemoryLimitConfig,
    /// OS搭載メモリ量(バイト)。起動時に一度だけ検出してキャッシュする。取得できない環境では None。
    pub os_total_memory_bytes: Option<u64>,
    /// メモリ上限設定の永続化先ファイルパス
    pub memory_settings_path: String,
    /// バックグラウンド監視タスクが直近に記録したアラート段階(ログの多重出力防止用)
    pub memory_alert_level: MemoryAlertLevel,
}

/// 現在のメモリ使用量(RSS)が設定上限に達しているかどうかを判定する。
/// 上限到達時は新規書き込み(レコード作成)を拒否するためのゲートとして使う。
pub(crate) fn memory_limit_exceeded(state: &AppStateInner) -> bool {
    let limit = state.memory_limit.effective_limit_bytes(state.os_total_memory_bytes);
    memory::is_over_limit(limit, memory::process_rss_bytes())
}

pub type AppState = Arc<RwLock<AppStateInner>>;

pub(crate) fn auto_save(state: &mut AppStateInner) {
    if let Some(kdb) = state.kdb.as_mut() {
        let records: Vec<Record> = state.mgr.db().list_all().into_iter().cloned().collect();
        let next_id = state.mgr.db().next_id();
        if let Err(e) = kdb.compact(&records, next_id) {
            eprintln!("[WARN] KDB compact failed: {}", e);
            if let KdbError::Io(io_err) = &e {
                state.logger.db_io_error("KDB compact failed", io_err);
            } else {
                state.logger.db_warn(format!("KDB compact failed: {}", e));
            }
        } else {
            state.logger.db_info(format!("KDB saved to '{}' ({} record(s))", state.db_path, records.len()));
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
            Json(serde_json::to_value(RecordResponse::from_record(record)).unwrap())),
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

    if memory_limit_exceeded(&inner) {
        inner.logger.memory_alert("メモリ使用量が設定上限に達したため、新規レコード作成(POST /records)を拒否しました".to_string());
        return (StatusCode::INSUFFICIENT_STORAGE,
            Json(serde_json::to_value(ErrorResponse::new(
                "メモリ使用量が設定上限に達しているため、新規データの書き込みを拒否しました。設定画面でメモリ上限を確認してください。"
            )).unwrap()));
    }

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
            let response = RecordResponse::from_record(inner.mgr.db().get(id).unwrap());
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
            let response = RecordResponse::from_record(inner.mgr.db().get(id).unwrap());
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
