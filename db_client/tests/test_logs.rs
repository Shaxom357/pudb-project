// tests/test_logs.rs
// GET /logs（ログ出力保管機能）の統合テスト

use db_client::test_helpers::spawn_test_server;

async fn post_sql(client: &reqwest::Client, base: &str, query: &str) -> (u16, serde_json::Value) {
    let res = client.post(format!("{}/sql", base))
        .json(&serde_json::json!({ "query": query }))
        .send().await.unwrap();
    let status = res.status().as_u16();
    let body: serde_json::Value = res.json().await.unwrap();
    (status, body)
}

async fn get_logs(client: &reqwest::Client, base: &str, lines: Option<usize>) -> Vec<serde_json::Value> {
    let url = match lines {
        Some(n) => format!("{}/logs?lines={}", base, n),
        None => format!("{}/logs", base),
    };
    let res = client.get(url).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    res.json().await.unwrap()
}

fn cleanup(db_path: &std::path::Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(format!("{}.log", db_path.to_string_lossy()));
}

#[tokio::test]
async fn test_logs_endpoint_records_sql_and_http_activity() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (employee_name, employee_age) VALUE ('田中', 24)"
    ).await;

    let entries = get_logs(&client, &base, None).await;
    assert!(entries.iter().any(|e| e["category"] == "sql" && e["success"] == true));
    assert!(entries.iter().any(|e| e["category"] == "http"));
    // 新しい順に返る
    assert!(!entries.is_empty());

    cleanup(&db_path);
}

#[tokio::test]
async fn test_logs_records_sql_failure_as_unsuccessful() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, _) = post_sql(&client, &base, "SELECT * FROM label.employee WHERE bogus =").await;
    assert_eq!(status, 400);

    let entries = get_logs(&client, &base, None).await;
    assert!(entries.iter().any(|e| e["category"] == "sql" && e["success"] == false));

    cleanup(&db_path);
}

#[tokio::test]
async fn test_logs_lines_param_limits_returned_count() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    for i in 0..5 {
        post_sql(&client, &base, &format!("INSERT INTO (label.employee) (n) VALUE ({})", i)).await;
    }

    let entries = get_logs(&client, &base, Some(3)).await;
    assert_eq!(entries.len(), 3);

    cleanup(&db_path);
}

#[tokio::test]
async fn test_logs_entries_have_timestamp_and_level() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "SELECT * FROM label.*").await;

    let entries = get_logs(&client, &base, Some(1)).await;
    assert_eq!(entries.len(), 1);
    assert!(entries[0]["timestamp"].as_str().is_some());
    assert!(entries[0]["level"].as_str().is_some());

    cleanup(&db_path);
}
