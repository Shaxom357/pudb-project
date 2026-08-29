// src/main.rs
// axum HTTP API サーバー（KDB暗号化ストレージ対応）
//
// 環境変数:
//   DB_FILE  保存先ファイルパス（デフォルト: db_data.kdb）
//            拡張子 .kdb = 暗号化KDBモード, それ以外 = JSONモード（後方互換）
//   DB_ADDR  バインドアドレス（デフォルト: 0.0.0.0:3000）
//   KAGURA_MASTER_KEY  暗号化マスターキー(hex64)
//   KAGURA_LOG_FILE  ログ保存先ファイルパス（デフォルト: logs/kagura.log）
//   KAGURA_AUTH_FILE  認証情報（管理者ユーザー名・パスワードハッシュ）保存先ファイルパス
//                     （デフォルト: auth.json。初回起動時は username=kagura, password=root で自動作成）

mod app;
mod auth;
mod auth_handlers;
mod handlers;
mod info_handlers;
mod label_handlers;
mod logging;
mod logging_handlers;
mod models;
mod settings_handlers;
mod sql_handlers;
mod ui;
mod user_sql;

use std::sync::Arc;

use db_engine::Database;
use dynamic_label_management::LabelManager;
use db_engine::kdb_store::KdbFile;

use crate::logging::{is_hardware_io_error, Logger, StopReason};

/// ログ記録付きの致命的エラー終了。停止理由（アプリエラー / HW起因）を記録してから
/// プロセスを終了する。
fn fatal_exit(logger: &Logger, reason: StopReason, message: impl Into<String>) -> ! {
    let message = message.into();
    eprintln!("[ERROR] {}", message);
    logger.shutdown(reason, message);
    std::process::exit(1);
}

/// SIGINT (Ctrl+C) / SIGTERM を待ち受け、受信したら正常停止としてログに記録する。
async fn shutdown_signal(logger: Arc<Logger>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let reason = tokio::select! {
        _ = ctrl_c => "SIGINT (Ctrl+C) を受信",
        _ = terminate => "SIGTERM を受信",
    };
    logger.shutdown(StopReason::Normal, reason);
}

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DB_FILE").unwrap_or_else(|_| "db_data.kdb".to_string());
    let addr    = std::env::var("DB_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let log_path = std::env::var("KAGURA_LOG_FILE").unwrap_or_else(|_| "logs/kagura.log".to_string());
    let auth_path = std::env::var("KAGURA_AUTH_FILE").unwrap_or_else(|_| "auth.json".to_string());
    let is_kdb  = db_path.ends_with(".kdb");

    let logger = Arc::new(Logger::init(&log_path));
    let auth = crate::auth::AuthState::load_or_init(&auth_path);
    if auth.is_default_password() {
        println!("[WARN] 管理者ユーザー '{}' は初期パスワードのままです。至急 PUT /auth/password で変更してください", auth.username());
    }

    // パニックが発生した場合もエラー停止としてログへ残す
    // (「エラーで停止したのか」の判定材料。パニックは HW 起因とは判定できないため Error 扱い)
    {
        let panic_logger = logger.clone();
        std::panic::set_hook(Box::new(move |info| {
            eprintln!("[ERROR] panic: {}", info);
            panic_logger.shutdown(StopReason::Error, format!("panic: {}", info));
        }));
    }

    let (db, kdb_file) = if is_kdb {
        println!("[INFO] KDB mode: encrypted binary format");
        match Database::load_or_new_kdb(&db_path) {
            Ok(db) => {
                println!("[INFO] Loaded {} record(s) from '{}'", db.count(), db_path);
                let kdb = KdbFile::open_or_create(&db_path).unwrap_or_else(|e| {
                    let reason = if let db_engine::kdb_store::KdbError::Io(io_err) = &e {
                        if is_hardware_io_error(io_err) { StopReason::Hardware } else { StopReason::Error }
                    } else { StopReason::Error };
                    fatal_exit(&logger, reason, format!("KDB open: {}", e));
                });
                (db, Some(kdb))
            }
            Err(e) => {
                let reason = if let db_engine::KdbPersistError::Kdb(db_engine::kdb_store::KdbError::Io(io_err)) = &e {
                    if is_hardware_io_error(io_err) { StopReason::Hardware } else { StopReason::Error }
                } else { StopReason::Error };
                fatal_exit(&logger, reason, format!("Failed to load KDB: {}", e));
            }
        }
    } else {
        println!("[INFO] JSON mode (legacy)");
        match Database::load_or_new(&db_path) {
            Ok(db) => { println!("[INFO] Loaded {} record(s)", db.count()); (db, None) }
            Err(e) => {
                let reason = if let db_engine::PersistError::Io(io_err) = &e {
                    if is_hardware_io_error(io_err) { StopReason::Hardware } else { StopReason::Error }
                } else { StopReason::Error };
                fatal_exit(&logger, reason, format!("Failed to load DB: {}", e));
            }
        }
    };

    use tokio::sync::RwLock;
    use crate::handlers::AppStateInner;

    let state = Arc::new(RwLock::new(AppStateInner {
        mgr:     LabelManager::from_db(db),
        db_path: db_path.clone(),
        kdb:     kdb_file,
        // 既定ではHTTPリクエスト(REST API)を無効化し、データのやり取りはSQL(/sql)を基本とする。
        // 大量テストデータ投入など用途がある場合は設定画面(/settings)から有効化する。
        http_api_enabled: false,
        logger: logger.clone(),
        auth,
    }));

    let (router, _) = app::build_app_with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        let reason = if is_hardware_io_error(&e) { StopReason::Hardware } else { StopReason::Error };
        fatal_exit(&logger, reason, format!("Bind {}: {}", addr, e));
    });

    println!("KAGURA DB listening on http://{}", addr);
    println!("Storage: {} ({})", db_path, if is_kdb { "KDB encrypted" } else { "JSON" });
    println!("Endpoints: /records /labels /sql /db/info /settings /logs /ui /auth/login");
    println!("HTTP API (REST /records /labels): disabled by default -- enable via PUT /settings or the 設定 view in /ui");
    println!("Authentication: required for all endpoints except /ui and /auth/login -- POST /auth/login to obtain a Bearer token");
    println!("Logs: {} (view recent entries via GET /logs)", log_path);

    logger.startup(&addr, if is_kdb { "kdb" } else { "json" }, &db_path);

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal(logger.clone()))
        .await;

    if let Err(e) = serve_result {
        let reason = if is_hardware_io_error(&e) { StopReason::Hardware } else { StopReason::Error };
        fatal_exit(&logger, reason, format!("Server failed: {}", e));
    }
}
