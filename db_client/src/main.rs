// src/main.rs
// axum HTTP API サーバー（永続化対応）
//
// 環境変数:
//   DB_FILE  保存先JSONファイルパス（デフォルト: db_data.json）
//   DB_ADDR  バインドアドレス（デフォルト: 0.0.0.0:3000）

mod app;
mod handlers;
mod label_handlers;
mod models;
mod ui;

use db_engine::Database;
use dynamic_label_management::LabelManager;

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DB_FILE").unwrap_or_else(|_| "db_data.json".to_string());
    let addr    = std::env::var("DB_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());

    let db = match Database::load_or_new(&db_path) {
        Ok(db) => {
            println!("[INFO] Loaded {} record(s) from '{}'", db.count(), db_path);
            db
        }
        Err(e) => {
            eprintln!("[ERROR] Failed to load DB from '{}': {}", db_path, e);
            std::process::exit(1);
        }
    };

    let (router, _state) = app::build_app(LabelManager::from_db(db), db_path.clone());

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[ERROR] Failed to bind {}: {}", addr, e);
            std::process::exit(1);
        });

    println!("DB API server listening on http://{}", addr);
    println!("Persistence file: {}", db_path);
    println!("Endpoints:");
    println!("  [Records]");
    println!("  GET    /records");
    println!("  GET    /records/{{id}}");
    println!("  GET    /records/label/{{label}}");
    println!("  POST   /records");
    println!("  PUT    /records/{{id}}");
    println!("  DELETE /records/{{id}}");
    println!("  [Labels per record]");
    println!("  GET    /records/{{id}}/labels");
    println!("  POST   /records/{{id}}/labels");
    println!("  DELETE /records/{{id}}/labels/{{label}}");
    println!("  [Labels global]");
    println!("  GET    /labels");
    println!("  POST   /labels/search");
    println!("  PUT    /labels/rename");

    axum::serve(listener, router)
        .await
        .expect("Server failed");
}


