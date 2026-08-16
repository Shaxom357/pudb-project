// src/main.rs
// axum HTTP API サーバー（KDB暗号化ストレージ対応）
//
// 環境変数:
//   DB_FILE  保存先ファイルパス（デフォルト: db_data.kdb）
//            拡張子 .kdb = 暗号化KDBモード, それ以外 = JSONモード（後方互換）
//   DB_ADDR  バインドアドレス（デフォルト: 0.0.0.0:3000）
//   KAGURA_MASTER_KEY  暗号化マスターキー(hex64)

mod app;
mod handlers;
mod info_handlers;
mod label_handlers;
mod models;
mod sql_handlers;
mod ui;

use db_engine::Database;
use dynamic_label_management::LabelManager;
use db_engine::kdb_store::KdbFile;

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DB_FILE").unwrap_or_else(|_| "db_data.kdb".to_string());
    let addr    = std::env::var("DB_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let is_kdb  = db_path.ends_with(".kdb");

    let (db, kdb_file) = if is_kdb {
        println!("[INFO] KDB mode: encrypted binary format");
        match Database::load_or_new_kdb(&db_path) {
            Ok(db) => {
                println!("[INFO] Loaded {} record(s) from '{}'", db.count(), db_path);
                let kdb = KdbFile::open_or_create(&db_path)
                    .unwrap_or_else(|e| { eprintln!("[ERROR] KDB open: {}", e); std::process::exit(1); });
                (db, Some(kdb))
            }
            Err(e) => { eprintln!("[ERROR] Failed to load KDB: {}", e); std::process::exit(1); }
        }
    } else {
        println!("[INFO] JSON mode (legacy)");
        match Database::load_or_new(&db_path) {
            Ok(db) => { println!("[INFO] Loaded {} record(s)", db.count()); (db, None) }
            Err(e) => { eprintln!("[ERROR] Failed to load DB: {}", e); std::process::exit(1); }
        }
    };

    use std::sync::Arc;
    use tokio::sync::RwLock;
    use crate::handlers::AppStateInner;

    let state = Arc::new(RwLock::new(AppStateInner {
        mgr:     LabelManager::from_db(db),
        db_path: db_path.clone(),
        kdb:     kdb_file,
    }));

    let (router, _) = app::build_app_with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await
        .unwrap_or_else(|e| { eprintln!("[ERROR] Bind {}: {}", addr, e); std::process::exit(1); });

    println!("KAGURA DB listening on http://{}", addr);
    println!("Storage: {} ({})", db_path, if is_kdb { "KDB encrypted" } else { "JSON" });
    println!("Endpoints: /records /labels /sql /db/info /ui");

    axum::serve(listener, router).await.expect("Server failed");
}
