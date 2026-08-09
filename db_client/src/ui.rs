// src/ui.rs
use axum::{response::Html, response::IntoResponse};

pub async fn ui_handler() -> impl IntoResponse {
    Html(HTML)
}

const HTML: &str = include_str!("ui.html");
