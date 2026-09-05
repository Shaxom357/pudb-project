// tests/test_schema.rs
// 任意スキーマ層（ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA / DESCRIBE /
// SHOW SCHEMAS / VALIDATE LABEL、および schema_enforcement_enabled 設定）の統合テスト。
// 認証が実際に有効な状態のテストサーバー（spawn_test_server_with_auth）を使う。

use db_client::test_helpers::spawn_test_server_with_auth;
use serde_json::json;

async fn login(client: &reqwest::Client, base: &str, user: &str, pass: &str) -> reqwest::Response {
    client
        .post(format!("{}/auth/login", base))
        .json(&json!({"username": user, "password": pass}))
        .send()
        .await
        .unwrap()
}

async fn admin_token(client: &reqwest::Client, base: &str) -> String {
    let res = login(client, base, "kagura", "root").await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

async fn run_sql(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    query: &str,
) -> (reqwest::StatusCode, serde_json::Value) {
    let res = client
        .post(format!("{}/sql", base))
        .bearer_auth(token)
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let json: serde_json::Value = res.json().await.unwrap();
    (status, json)
}

#[tokio::test]
async fn test_define_column_does_not_enforce_until_enabled() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    let (status, body) = run_sql(&client, &base, &token,
        "ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL").await;
    assert_eq!(status, 200, "body={body}");

    // まだ ENABLE していないので date 無しで INSERT できる
    let (status, body) = run_sql(&client, &base, &token,
        "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;
    assert_eq!(status, 201, "body={body}");
}

#[tokio::test]
async fn test_enable_schema_enforces_not_null() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL").await;
    run_sql(&client, &base, &token, "ALTER LABEL race ENABLE SCHEMA").await;

    let (status, body) = run_sql(&client, &base, &token,
        "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;
    assert_eq!(status, 400, "body={body}");
    assert_eq!(body["ok"], false);

    let (status, _) = run_sql(&client, &base, &token,
        "INSERT INTO (label.race) (date, venue) VALUE ('2026-01-01', 'edogawa')").await;
    assert_eq!(status, 201);
}

#[tokio::test]
async fn test_disable_schema_stops_enforcement() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL").await;
    run_sql(&client, &base, &token, "ALTER LABEL race ENABLE SCHEMA").await;
    run_sql(&client, &base, &token, "ALTER LABEL race DISABLE SCHEMA").await;

    let (status, body) = run_sql(&client, &base, &token,
        "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;
    assert_eq!(status, 201, "body={body}");
}

#[tokio::test]
async fn test_describe_and_show_schemas() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token,
        "ALTER LABEL race DEFINE COLUMN odds TYPE float DEFAULT 0.0").await;
    run_sql(&client, &base, &token, "ALTER LABEL race ENABLE SCHEMA").await;

    let (status, body) = run_sql(&client, &base, &token, "DESCRIBE label.race").await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["columns"], json!(["column", "type", "not_null", "default", "unique", "schema_enabled"]));
    assert_eq!(body["rows"], json!([["odds", "float", false, 0.0, false, true]]));

    let (status, body) = run_sql(&client, &base, &token, "SHOW SCHEMAS").await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["rows"], json!([["race", true, 1]]));
}

#[tokio::test]
async fn test_describe_unknown_label_errors() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    let (status, body) = run_sql(&client, &base, &token, "DESCRIBE label.race").await;
    assert_eq!(status, 404, "body={body}");
}

#[tokio::test]
async fn test_validate_label_reports_violations_without_blocking() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    // スキーマ無効のまま date 無しで1件投入
    run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;
    run_sql(&client, &base, &token, "ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL").await;

    let (status, body) = run_sql(&client, &base, &token, "VALIDATE LABEL race").await;
    assert_eq!(status, 200, "body={body}");
    let rows = body["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0][0].as_str().unwrap().contains("date"));
}

#[tokio::test]
async fn test_unique_constraint_rejects_duplicate_via_sql() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token,
        "ALTER LABEL racer DEFINE COLUMN racer_id TYPE integer NOT NULL UNIQUE").await;
    run_sql(&client, &base, &token, "ALTER LABEL racer ENABLE SCHEMA").await;

    let (status, _) = run_sql(&client, &base, &token,
        "INSERT INTO (label.racer) (racer_id) VALUE (1)").await;
    assert_eq!(status, 201);

    let (status, body) = run_sql(&client, &base, &token,
        "INSERT INTO (label.racer) (racer_id) VALUE (1)").await;
    assert_eq!(status, 400, "body={body}");
}

#[tokio::test]
async fn test_global_schema_enforcement_setting_overrides_enabled_label() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL").await;
    run_sql(&client, &base, &token, "ALTER LABEL race ENABLE SCHEMA").await;

    // デフォルトでは有効なので拒否される
    let (status, _) = run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('x')").await;
    assert_eq!(status, 400);

    // グローバルスイッチを切ると、ラベル側は ENABLE のままでも通る
    let res = client.put(format!("{}/settings", base)).bearer_auth(&token)
        .json(&json!({"schema_enforcement_enabled": false})).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["schema_enforcement_enabled"], false);
    // http_api_enabled は明示していないので既定値のまま維持されている
    assert_eq!(body["http_api_enabled"], true);

    let (status, body) = run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('x')").await;
    assert_eq!(status, 201, "body={body}");

    // 戻すと再び拒否される
    client.put(format!("{}/settings", base)).bearer_auth(&token)
        .json(&json!({"schema_enforcement_enabled": true})).send().await.unwrap();
    let (status, _) = run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('x')").await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn test_general_user_without_manage_users_cannot_define_column() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let admin = admin_token(&client, &base).await;

    run_sql(&client, &base, &admin, "CREATE USER 'alice' IDENTIFIED BY 'aA951753'").await;
    let res = login(&client, &base, "alice", "aA951753").await;
    let body: serde_json::Value = res.json().await.unwrap();
    let alice_token = body["token"].as_str().unwrap().to_string();

    let (status, body) = run_sql(&client, &base, &alice_token,
        "ALTER LABEL race DEFINE COLUMN date TYPE text").await;
    assert_eq!(status, 403, "body={body}");
}
