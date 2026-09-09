// src/test_helpers.rs
// 統合テスト用ヘルパー（lib として公開）

use dynamic_label_management::LabelManager;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(15000);

pub fn next_test_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// テスト用サーバーを起動し、ベースURLと一時DBパスを返す
pub async fn spawn_test_server() -> (String, std::path::PathBuf) {
    let db_path = std::env::temp_dir().join(format!(
        "db_client_test_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));

    let mgr = LabelManager::new();
    let (app, _state) =
        crate::app::build_app(mgr, db_path.to_string_lossy().to_string());

    let port = next_test_port();
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (base_url, db_path)
}

/// 認証が実際に有効な状態でテストサーバーを起動する（test_auth.rs 専用）。
/// build_app() とは異なり AuthState::bypass_for_tests() を使わないため、
/// ログイン(POST /auth/login)しない限り保護対象エンドポイントは 401 を返す。
pub async fn spawn_test_server_with_auth() -> (String, std::path::PathBuf) {
    use crate::auth::AuthState;
    use crate::handlers::AppStateInner;
    use crate::logging::Logger;

    let db_path = std::env::temp_dir().join(format!(
        "db_client_authtest_{}_{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let auth_path = std::env::temp_dir().join(format!(
        "db_client_authtest_{}_{}.auth.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));

    let logger = Arc::new(Logger::init(&format!("{}.log", db_path.to_string_lossy())));
    let auth = AuthState::load_or_init(&auth_path.to_string_lossy());
    let state = Arc::new(RwLock::new(AppStateInner {
        mgr: LabelManager::new(),
        db_path: db_path.to_string_lossy().to_string(),
        kdb: None,
        http_api_enabled: true,
        logger,
        auth,
        active_txn: None,
        restore_pending: false,
    }));
    let (app, _) = crate::app::build_app_with_state(state);

    let port = next_test_port();
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _ = std::fs::remove_file(&auth_path);
    (base_url, db_path)
}

/// POST /records のショートカット
pub async fn post_record(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    client
        .post(format!("{}/records", base))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
