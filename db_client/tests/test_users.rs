// tests/test_users.rs
// 一般ユーザー管理（CREATE USER / DROP USER / ALTER USER / SHOW USERS）の統合テスト。
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
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let res = client
        .post(format!("{}/sql", base))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status();
    let json: serde_json::Value = res.json().await.unwrap();
    (status, json)
}

#[tokio::test]
async fn test_create_user_with_strong_password_and_login() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    let (status, body) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'alice' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["ok"], true);

    // 作成したユーザーでログインできる
    let res = login(&client, &base, "alice", "aA951753").await;
    assert_eq!(res.status(), 200);
    let lb: serde_json::Value = res.json().await.unwrap();
    let alice_token = lb["token"].as_str().unwrap().to_string();

    let res = client
        .get(format!("{}/db/info", base))
        .bearer_auth(&alice_token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_weak_password_requires_confirmation() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    // 確認なし -> needs_confirmation
    let (status, body) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'weak' IDENTIFIED BY '1234'"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], false);
    assert_eq!(body["needs_confirmation"], true);
    assert_eq!(
        body["warning"],
        "Warning: The password strength is too weak. Do you want to proceed?\n\nPlease enter 'yes' to proceed or 'no' to cancel."
    );

    // no -> 中止（作成されない）
    let (_s, body) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'weak' IDENTIFIED BY '1234'", "confirm_weak_password": false}),
    )
    .await;
    assert_eq!(body["ok"], false);
    assert!(login(&client, &base, "weak", "1234").await.status() != 200);

    // yes -> 作成される
    let (status, body) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'weak' IDENTIFIED BY '1234'", "confirm_weak_password": true}),
    )
    .await;
    assert_eq!(status, 200, "body={body}");
    assert_eq!(body["ok"], true);
    assert_eq!(login(&client, &base, "weak", "1234").await.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_general_user_cannot_manage_users() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let admin = admin_token(&client, &base).await;

    let (status, _) = run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "CREATE USER 'bob' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    assert_eq!(status, 200);

    let res = login(&client, &base, "bob", "aA951753").await;
    let bt = res.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // 一般ユーザーは CREATE USER も SHOW USERS も不可（403）
    let (status, body) = run_sql(
        &client,
        &base,
        &bt,
        json!({"query": "CREATE USER 'carol' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    assert_eq!(status, 403, "body={body}");

    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SHOW USERS"})).await;
    assert_eq!(status, 403);

    // ただしデータ操作系は従来通り可能
    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SELECT * FROM label.*"})).await;
    assert_eq!(status, 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_show_users_lists_users_without_hash() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'dave' IDENTIFIED BY 'aA951753'"}),
    )
    .await;

    let (status, body) = run_sql(&client, &base, &token, json!({"query": "SHOW USERS"})).await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    let cols: Vec<String> = body["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert!(cols.contains(&"username".to_string()));
    assert!(cols.contains(&"role".to_string()));
    assert!(!cols.iter().any(|c| c.contains("hash") || c.contains("password")));

    let text = body.to_string();
    assert!(text.contains("kagura"));
    assert!(text.contains("dave"));
    assert!(!text.contains("$argon2"));

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_drop_user_and_admin_protection() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'erin' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    assert_eq!(login(&client, &base, "erin", "aA951753").await.status(), 200);

    // 管理者は削除不可
    let (status, _) = run_sql(&client, &base, &token, json!({"query": "DROP USER 'kagura'"})).await;
    assert_eq!(status, 403);

    // 一般ユーザーは削除できる -> 以後ログイン不可
    let (status, _) = run_sql(&client, &base, &token, json!({"query": "DROP USER 'erin'"})).await;
    assert_eq!(status, 200);
    assert_ne!(login(&client, &base, "erin", "aA951753").await.status(), 200);

    // 存在しないユーザー -> 404 / IF EXISTS なら 200
    let (status, _) = run_sql(&client, &base, &token, json!({"query": "DROP USER 'erin'"})).await;
    assert_eq!(status, 404);
    let (status, _) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "DROP USER IF EXISTS 'erin'"}),
    )
    .await;
    assert_eq!(status, 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_alter_user_password() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'frank' IDENTIFIED BY 'aA951753'"}),
    )
    .await;

    let (status, body) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "ALTER USER 'frank' IDENTIFIED BY 'Xy837261'"}),
    )
    .await;
    assert_eq!(status, 200, "body={body}");

    assert_ne!(login(&client, &base, "frank", "aA951753").await.status(), 200);
    assert_eq!(login(&client, &base, "frank", "Xy837261").await.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_grant_and_revoke_enforced_on_sql() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let admin = admin_token(&client, &base).await;

    let (status, _) = run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "CREATE USER 'ken' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    assert_eq!(status, 200);

    let bt = login(&client, &base, "ken", "aA951753")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // 既定では SELECT / INSERT 可能
    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SELECT * FROM label.*"})).await;
    assert_eq!(status, 200);

    // INSERT 権限を剥奪 -> INSERT が 403、SELECT は引き続き 200
    let (status, _) = run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "REVOKE INSERT, DELETE FROM 'ken'"}),
    )
    .await;
    assert_eq!(status, 200);

    let (status, body) = run_sql(
        &client,
        &base,
        &bt,
        json!({"query": "INSERT INTO (label.t) (a) VALUE (1)"}),
    )
    .await;
    assert_eq!(status, 403, "body={body}");
    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SELECT * FROM label.*"})).await;
    assert_eq!(status, 200);

    // 権限を再付与 -> INSERT が通る
    let (status, _) = run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "GRANT INSERT TO 'ken'"}),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = run_sql(
        &client,
        &base,
        &bt,
        json!({"query": "INSERT INTO (label.t) (a) VALUE (1)"}),
    )
    .await;
    assert_eq!(status, 201);

    // MANAGE_USERS を持たない一般ユーザーは GRANT / SHOW USERS ができない
    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SHOW USERS"})).await;
    assert_eq!(status, 403);
    let (status, _) = run_sql(
        &client,
        &base,
        &bt,
        json!({"query": "GRANT SELECT TO 'ken'"}),
    )
    .await;
    assert_eq!(status, 403);

    // SUPER を付与すると MANAGE_USERS も含まれ、SHOW USERS が可能になる
    let (status, _) = run_sql(&client, &base, &admin, json!({"query": "GRANT SUPER TO 'ken'"})).await;
    assert_eq!(status, 200);
    let (status, _) = run_sql(&client, &base, &bt, json!({"query": "SHOW USERS"})).await;
    assert_eq!(status, 200);

    // 管理者ユーザーへの GRANT は 403
    let (status, _) = run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "GRANT SELECT TO 'kagura'"}),
    )
    .await;
    assert_eq!(status, 403);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_grant_enforced_on_rest_api() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let admin = admin_token(&client, &base).await;

    // REST API を有効化
    let res = client
        .put(format!("{}/settings", base))
        .bearer_auth(&admin)
        .json(&json!({"http_api_enabled": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    run_sql(
        &client,
        &base,
        &admin,
        json!({"query": "CREATE USER 'lea' IDENTIFIED BY 'aA951753'"}),
    )
    .await;
    run_sql(&client, &base, &admin, json!({"query": "REVOKE SELECT FROM 'lea'"})).await;

    let lt = login(&client, &base, "lea", "aA951753")
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // SELECT 権限が無いので GET /records は 403
    let res = client
        .get(format!("{}/records", base))
        .bearer_auth(&lt)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_create_user_duplicate_and_syntax_errors() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let client = reqwest::Client::new();
    let token = admin_token(&client, &base).await;

    run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'grace' IDENTIFIED BY 'aA951753'"}),
    )
    .await;

    // 重複 -> 409
    let (status, _) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'grace' IDENTIFIED BY 'zZ864209'"}),
    )
    .await;
    assert_eq!(status, 409);

    // IF NOT EXISTS -> 200
    let (status, _) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER IF NOT EXISTS 'grace' IDENTIFIED BY 'zZ864209'"}),
    )
    .await;
    assert_eq!(status, 200);

    // 構文エラー -> 400
    let (status, _) = run_sql(
        &client,
        &base,
        &token,
        json!({"query": "CREATE USER 'broken'"}),
    )
    .await;
    assert_eq!(status, 400);

    let _ = std::fs::remove_file(db_path);
}
