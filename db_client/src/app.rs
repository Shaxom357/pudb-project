// src/app.rs
// Router の組み立てを main から分離する
// テストからも同じルーター定義を使えるようにする

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use dynamic_label_management::LabelManager;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::handlers::{
    create_record, delete_record, get_record,
    get_records_by_label, list_records, update_record,
    AppState, AppStateInner,
};
use crate::label_handlers::{
    add_label, list_record_labels, remove_label,
    list_all_labels, search_by_labels, rename_label,
};
use crate::info_handlers::get_db_info;
use crate::sql_handlers::execute_sql;

use crate::ui::ui_handler;

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/ui", get(ui_handler))
        .route("/", get(|| async { axum::response::Redirect::temporary("/ui") }))
        .route("/records", get(list_records).post(create_record))
        .route("/records/label/{label}", get(get_records_by_label))
        .route("/records/{id}", get(get_record).put(update_record).delete(delete_record))
        .route("/records/{id}/labels", get(list_record_labels).post(add_label))
        .route("/records/{id}/labels/{label}", delete(remove_label))
        .route("/labels", get(list_all_labels))
        .route("/labels/search", post(search_by_labels))
        .route("/labels/rename", put(rename_label))
        .route("/db/info", get(get_db_info))
        .route("/sql", post(execute_sql))
        .with_state(state)
}

/// テスト用: mgr と db_path から AppState を作って Router を返す
pub fn build_app(mgr: LabelManager, db_path: String) -> (Router, AppState) {
    let state: AppState = Arc::new(RwLock::new(AppStateInner { mgr, db_path, kdb: None }));
    let router = build_router(state.clone());
    (router, state)
}

/// 本番用: 外から組み立てた AppState を受け取って Router を返す
pub fn build_app_with_state(state: AppState) -> (Router, AppState) {
    let router = build_router(state.clone());
    (router, state)
}
