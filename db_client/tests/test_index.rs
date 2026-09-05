// tests/test_index.rs
// 二次インデックス管理（CREATE INDEX / DROP INDEX / SHOW INDEXES / EXPLAIN）の統合テスト。
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
async fn test_create_index_then_show_indexes() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    let (status, body) = run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["ok"], true);

    let (status, body) = run_sql(&client, &base, &token, "SHOW INDEXES").await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["rows"], json!([["venue"]]));
}

#[tokio::test]
async fn test_create_index_twice_without_if_not_exists_errors() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;
    let (status, body) = run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;
    assert_eq!(status, 409, "body={body}");
    assert_eq!(body["ok"], false);
}

#[tokio::test]
async fn test_create_index_if_not_exists_is_idempotent() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;
    let (status, body) = run_sql(&client, &base, &token, "CREATE INDEX IF NOT EXISTS ON label.race (venue)").await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn test_drop_index_removes_it() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;
    let (status, body) = run_sql(&client, &base, &token, "DROP INDEX ON label.race (venue)").await;
    assert_eq!(status, 200, "body={body}");

    let (_, body) = run_sql(&client, &base, &token, "SHOW INDEXES").await;
    // rows が空の場合、レスポンスDTOの skip_serializing_if によりフィールド自体が省略される
    assert_eq!(body.get("rows"), None, "body={body}");
}

#[tokio::test]
async fn test_drop_nonexistent_index_without_if_exists_errors() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    let (status, body) = run_sql(&client, &base, &token, "DROP INDEX ON label.race (venue)").await;
    assert_eq!(status, 404, "body={body}");
}

#[tokio::test]
async fn test_general_user_without_manage_users_cannot_create_index() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let admin = admin_token(&client, &base).await;

    run_sql(&client, &base, &admin, "CREATE USER 'alice' IDENTIFIED BY 'aA951753'").await;
    let res = login(&client, &base, "alice", "aA951753").await;
    let body: serde_json::Value = res.json().await.unwrap();
    let alice_token = body["token"].as_str().unwrap().to_string();

    let (status, body) = run_sql(&client, &base, &alice_token, "CREATE INDEX ON label.race (venue)").await;
    assert_eq!(status, 403, "body={body}");
}

#[tokio::test]
async fn test_explain_reports_full_scan_before_index_and_index_scan_after() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token,
        "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;

    let (status, body) = run_sql(&client, &base, &token,
        "EXPLAIN SELECT * FROM label.race WHERE venue = 'edogawa'").await;
    assert_eq!(status, 200, "body={body}");
    let plan_before = body["rows"][0][0].as_str().unwrap().to_string();
    assert!(plan_before.starts_with("full scan"), "plan_before={plan_before}");

    run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;

    let (status, body) = run_sql(&client, &base, &token,
        "EXPLAIN SELECT * FROM label.race WHERE venue = 'edogawa'").await;
    assert_eq!(status, 200, "body={body}");
    let plan_after = body["rows"][0][0].as_str().unwrap().to_string();
    assert!(plan_after.starts_with("index scan: venue"), "plan_after={plan_after}");
}

#[tokio::test]
async fn test_select_returns_same_rows_with_or_without_index() {
    let (base, _db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('edogawa')").await;
    run_sql(&client, &base, &token, "INSERT INTO (label.race) (venue) VALUE ('toda')").await;

    let (_, before) = run_sql(&client, &base, &token,
        "SELECT venue FROM label.race WHERE venue = 'edogawa'").await;

    run_sql(&client, &base, &token, "CREATE INDEX ON label.race (venue)").await;

    let (_, after) = run_sql(&client, &base, &token,
        "SELECT venue FROM label.race WHERE venue = 'edogawa'").await;

    assert_eq!(before["rows"], after["rows"]);
    assert_eq!(after["rows"], json!([["edogawa"]]));
}
