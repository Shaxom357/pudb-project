
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
async fn test_settings_default_memory_limit_is_60_percent() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.get(format!("{}/settings", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["memory_limit_mode"], "percent");
    assert_eq!(body["memory_limit_percent"], 60);
    assert!(body["memory_usage_bytes"].is_number() || body["memory_usage_bytes"].is_null());
    let _ = std::fs::remove_file(format!("{}.memlimit.json", db_path.to_string_lossy()));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_memory_limit_percent() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.put(format!("{}/settings/memory", base))
        .json(&serde_json::json!({"mode": "percent", "percent": 25}))
        .send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["memory_limit_mode"], "percent");
    assert_eq!(body["memory_limit_percent"], 25);

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.memlimit.json", db_path.to_string_lossy()));
}

#[tokio::test]
async fn test_update_memory_limit_absolute() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.put(format!("{}/settings/memory", base))
        .json(&serde_json::json!({"mode": "absolute", "absolute_bytes": 1_073_741_824u64}))
        .send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["memory_limit_mode"], "absolute");
    assert_eq!(body["memory_limit_absolute_bytes"], 1_073_741_824u64);
    assert_eq!(body["memory_limit_effective_bytes"], 1_073_741_824u64);

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.memlimit.json", db_path.to_string_lossy()));
}

#[tokio::test]
async fn test_update_memory_limit_rejects_invalid_percent() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.put(format!("{}/settings/memory", base))
        .json(&serde_json::json!({"mode": "percent", "percent": 0}))
        .send().await.unwrap();
    assert_eq!(res.status(), 400);

    let res = client.put(format!("{}/settings/memory", base))
        .json(&serde_json::json!({"mode": "percent", "percent": 101}))
        .send().await.unwrap();
    assert_eq!(res.status(), 400);

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.memlimit.json", db_path.to_string_lossy()));
}

#[tokio::test]
async fn test_record_creation_rejected_when_memory_limit_exceeded() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // 現在のプロセスRSSより確実に小さい上限(1 byte)を設定し、以降の書き込みを拒否させる
    let res = client.put(format!("{}/settings/memory", base))
        .json(&serde_json::json!({"mode": "absolute", "absolute_bytes": 1u64}))
        .send().await.unwrap();
    assert_eq!(res.status(), 200);

    let res = client.post(format!("{}/records", base))
        .json(&serde_json::json!({"columns":{},"labels":[]}))
        .send().await.unwrap();
    assert_eq!(res.status(), 507);

    let res = client.post(format!("{}/sql", base))
        .json(&serde_json::json!({"query": "INSERT INTO (label.employee) (name) VALUE ('田中')"}))
        .send().await.unwrap();
    assert_eq!(res.status(), 507);

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.memlimit.json", db_path.to_string_lossy()));
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
