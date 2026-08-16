
use db_client::test_helpers::spawn_test_server;

#[tokio::test]
async fn test_settings_default_http_api_enabled() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.get(format!("{}/settings", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["http_api_enabled"], true);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_disabling_http_api_blocks_records_and_labels() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"http_api_enabled": false}))
        .send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["http_api_enabled"], false);

    let res = client.get(format!("{}/records", base)).send().await.unwrap();
    assert_eq!(res.status(), 403);

    let res = client.post(format!("{}/records", base))
        .json(&serde_json::json!({"columns":{},"labels":[]}))
        .send().await.unwrap();
    assert_eq!(res.status(), 403);

    let res = client.get(format!("{}/labels", base)).send().await.unwrap();
    assert_eq!(res.status(), 403);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_sql_and_ui_stay_available_when_http_api_disabled() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"http_api_enabled": false}))
        .send().await.unwrap();

    let res = client.post(format!("{}/sql", base))
        .json(&serde_json::json!({"query": "SELECT * FROM label.*"}))
        .send().await.unwrap();
    assert_eq!(res.status(), 200);

    let res = client.get(format!("{}/db/info", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);

    let res = client.get(format!("{}/ui", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_reenabling_http_api_restores_access() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"http_api_enabled": false}))
        .send().await.unwrap();
    client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"http_api_enabled": true}))
        .send().await.unwrap();

    let res = client.get(format!("{}/records", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);

    let _ = std::fs::remove_file(db_path);
}
