use db_client::test_helpers::{spawn_test_server, spawn_test_server_with_auth};

async fn sql(client: &reqwest::Client, base: &str, q: &str) -> (u16, serde_json::Value) {
    let res = client
        .post(format!("{}/sql", base))
        .json(&serde_json::json!({ "query": q }))
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap())
}

async fn sql_auth(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    q: &str,
) -> (u16, serde_json::Value) {
    let res = client
        .post(format!("{}/sql", base))
        .bearer_auth(token)
        .json(&serde_json::json!({ "query": q }))
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    (status, res.json().await.unwrap())
}

fn row_count(body: &serde_json::Value) -> usize {
    body["rows"].as_array().map(|a| a.len()).unwrap_or(0)
}

#[tokio::test]
async fn test_rollback_discards_changes() {
    let (base, db_path) = spawn_test_server().await;
    let c = reqwest::Client::new();

    let (s, _) = sql(&c, &base, "INSERT INTO (label.t) (n) VALUE ('before')").await;
    assert_eq!(s, 201);

    let (s, _) = sql(&c, &base, "BEGIN").await;
    assert_eq!(s, 200);

    sql(&c, &base, "INSERT INTO (label.t) (n) VALUE ('inside')").await;
    // トランザクション中の SELECT は未コミットの変更が見える
    let (_, body) = sql(&c, &base, "SELECT n FROM label.t").await;
    assert_eq!(row_count(&body), 2);

    let (s, _) = sql(&c, &base, "ROLLBACK").await;
    assert_eq!(s, 200);

    // ロールバック後は BEGIN 前の状態に戻る
    let (_, body) = sql(&c, &base, "SELECT n FROM label.t").await;
    assert_eq!(row_count(&body), 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_commit_persists_changes() {
    let (base, db_path) = spawn_test_server().await;
    let c = reqwest::Client::new();

    sql(&c, &base, "BEGIN").await;
    sql(&c, &base, "INSERT INTO (label.t) (n) VALUE ('a')").await;
    sql(&c, &base, "INSERT INTO (label.t) (n) VALUE ('b')").await;
    let (s, _) = sql(&c, &base, "COMMIT").await;
    assert_eq!(s, 200);

    let (_, body) = sql(&c, &base, "SELECT n FROM label.t").await;
    assert_eq!(row_count(&body), 2);

    // COMMIT 済みなので、以降のロールバック単独では戻らない（トランザクションが無い）
    let (s, _) = sql(&c, &base, "ROLLBACK").await;
    assert_eq!(s, 400);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_double_begin_conflicts_and_commit_without_begin_errors() {
    let (base, db_path) = spawn_test_server().await;
    let c = reqwest::Client::new();

    let (s, _) = sql(&c, &base, "COMMIT").await;
    assert_eq!(s, 400);

    sql(&c, &base, "BEGIN").await;
    let (s, _) = sql(&c, &base, "BEGIN TRANSACTION").await;
    assert_eq!(s, 409);

    sql(&c, &base, "ROLLBACK").await;
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_ddl_rejected_during_transaction() {
    let (base, db_path) = spawn_test_server().await;
    let c = reqwest::Client::new();

    sql(&c, &base, "INSERT INTO (label.t) (n) VALUE ('x')").await;
    sql(&c, &base, "BEGIN").await;

    let (s, _) = sql(&c, &base, "CREATE INDEX ON label.t (n)").await;
    assert_eq!(s, 409);
    let (s, _) = sql(&c, &base, "ALTER LABEL t DEFINE COLUMN n TYPE text").await;
    assert_eq!(s, 409);

    sql(&c, &base, "ROLLBACK").await;
    // ロールバック後は DDL が通る
    let (s, _) = sql(&c, &base, "CREATE INDEX ON label.t (n)").await;
    assert_eq!(s, 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_other_users_write_is_blocked_during_transaction() {
    let (base, db_path) = spawn_test_server_with_auth().await;
    let c = reqwest::Client::new();

    // 管理者ログイン
    let admin_tok: String = {
        let r = c.post(format!("{}/auth/login", base))
            .json(&serde_json::json!({"username":"kagura","password":"root"}))
            .send().await.unwrap().json::<serde_json::Value>().await.unwrap();
        r["token"].as_str().unwrap().to_string()
    };
    // 一般ユーザーを作成してログイン
    sql_auth(&c, &base, &admin_tok, "CREATE USER 'bob' IDENTIFIED BY 'bobsecret123'").await;
    let bob_tok: String = {
        let r = c.post(format!("{}/auth/login", base))
            .json(&serde_json::json!({"username":"bob","password":"bobsecret123"}))
            .send().await.unwrap().json::<serde_json::Value>().await.unwrap();
        r["token"].as_str().unwrap().to_string()
    };

    // 管理者がトランザクション開始
    let (s, _) = sql_auth(&c, &base, &admin_tok, "BEGIN").await;
    assert_eq!(s, 200);

    // bob の書き込みは 409
    let (s, _) = sql_auth(&c, &base, &bob_tok, "INSERT INTO (label.t) (n) VALUE ('bob')").await;
    assert_eq!(s, 409);
    // bob の COMMIT も 409（所有者ではない）
    let (s, _) = sql_auth(&c, &base, &bob_tok, "COMMIT").await;
    assert_eq!(s, 409);
    // bob の SELECT は許可（未コミット値が見える場合あり）
    let (s, _) = sql_auth(&c, &base, &bob_tok, "SELECT * FROM label.*").await;
    assert_eq!(s, 200);

    sql_auth(&c, &base, &admin_tok, "ROLLBACK").await;

    let _ = std::fs::remove_file(db_path);
}
