// tests/test_auth.rs
// 認証機能（ログイン/ログアウト/パスワード変更/保護対象エンドポイント）の統合テスト

use db_client::test_helpers::spawn_test_server_with_auth;

#[tokio::test]
async fn test_protected_endpoint_without_token_is_rejected() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client.get(format!("{}/db/info", base)).send().await.unwrap();
    assert_eq!(res.status(), 401);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_ui_and_login_stay_public_without_token() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client.get(format!("{}/ui", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_login_with_default_credentials_grants_access() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(body["username"], "kagura");

    let res = client
        .get(format!("{}/db/info", base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_logout_invalidates_token() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    let res = client
        .post(format!("{}/auth/logout", base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    let res = client
        .get(format!("{}/db/info", base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_repeated_failed_logins_lock_the_account() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    for _ in 0..5 {
        let res = client
            .post(format!("{}/auth/login", base))
            .json(&serde_json::json!({"username": "kagura", "password": "wrong"}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
    }

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 423);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_change_password_requires_relogin() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    let res = client
        .put(format!("{}/auth/password", base))
        .bearer_auth(&token)
        .json(&serde_json::json!({"current_password": "root", "new_password": "newpassword123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);

    // 変更前に発行されたトークンは失効している
    let res = client
        .get(format!("{}/db/info", base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    // 旧パスワードではログインできない
    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    // 新パスワードでログインできる
    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "newpassword123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_change_password_rejects_short_new_password() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/auth/login", base))
        .json(&serde_json::json!({"username": "kagura", "password": "root"}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    let token = body["token"].as_str().unwrap().to_string();

    let res = client
        .put(format!("{}/auth/password", base))
        .bearer_auth(&token)
        .json(&serde_json::json!({"current_password": "root", "new_password": "short"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_data_endpoints_require_token_even_when_http_api_enabled() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/records", base))
        .json(&serde_json::json!({"columns":{}, "labels":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let res = client
        .post(format!("{}/sql", base))
        .json(&serde_json::json!({"query": "SELECT * FROM label.*"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let _ = std::fs::remove_file(db_path);
}
