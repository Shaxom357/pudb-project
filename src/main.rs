use axum::{
    extract::{Path, Query, Json},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use db_engine::get_by_label;

#[derive(Serialize, Deserialize)]
struct Data {
    label: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
struct DataResponse {
    data: Vec<Data>,
}

async fn get_data(query: Query<String>) -> impl IntoResponse {
    let label = query.0;
    match get_by_label(&label) {
        Ok(data) => (StatusCode::OK, Json(DataResponse { data })),
        Err(_) => (StatusCode::NOT_FOUND, "Data not found".to_string()),
    }
}

async fn post_data(Json(payload): Json<Data>) -> impl IntoResponse {
    // ここでデータを保存する処理を実装
    (StatusCode::CREATED, Json(payload))
}

#[tokio::main]
async fn main() {
    let app = Router::new()
       .route("/data", get(get_data).post(post_data));

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
       .serve(app.into_make_service())
       .await
       .unwrap();
}