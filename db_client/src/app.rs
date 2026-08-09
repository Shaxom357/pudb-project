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

use crate::ui::ui_handler;

/// テスト・本番共通のルーター組み立て関数
pub fn build_app(mgr: LabelManager, db_path: String) -> (Router, AppState) {
    let state: AppState = Arc::new(RwLock::new(AppStateInner { mgr, db_path }));

    let app = Router::new()
        // --- 管理UI ---
        .route("/ui", get(ui_handler))
        .route("/", get(|| async { axum::response::Redirect::temporary("/ui") }))
        // --- レコード操作 ---
        .route("/records", get(list_records).post(create_record))
        .route("/records/label/{label}", get(get_records_by_label))
        .route(
            "/records/{id}",
            get(get_record).put(update_record).delete(delete_record),
        )
        // --- ラベル操作（レコード単位） ---
        .route("/records/{id}/labels", get(list_record_labels).post(add_label))
        .route("/records/{id}/labels/{label}", delete(remove_label))
        // --- ラベル操作（DB全体） ---
        .route("/labels", get(list_all_labels))
        .route("/labels/search", post(search_by_labels))
        .route("/labels/rename", put(rename_label))
        .with_state(state.clone());

    (app, state)
}
