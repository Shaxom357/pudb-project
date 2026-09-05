
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
async fn test_settings_exposes_memory_policy_defaults() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.get(format!("{}/settings", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    // 既定は「全データオンメモリ有効」
    assert_eq!(body["all_in_memory"], true);
    assert!(body["memory_limit"].is_object());
    assert!(body["memory_resident_rows"].is_number());
    assert!(body["memory_total_rows"].is_number());
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_memory_policy_change_rejected_in_json_mode() {
    // テストサーバーは .json モードのため、「全データオンメモリ」設定の変更は 400
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"all_in_memory": false}))
        .send().await.unwrap();
    assert_eq!(res.status(), 400);

    let res = client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"memory_limit_spec": "abc"}))
        .send().await.unwrap();
    assert_eq!(res.status(), 400); // 不正な指定は 400

    // 他フィールドのみの更新は従来どおり成功する
    let res = client.put(format!("{}/settings", base))
        .json(&serde_json::json!({"http_api_enabled": true}))
        .send().await.unwrap();
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
