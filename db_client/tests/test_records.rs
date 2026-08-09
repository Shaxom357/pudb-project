
use db_client::test_helpers::{spawn_test_server, post_record};

#[tokio::test]
async fn test_list_records_empty() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.get(format!("{}/records", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_list_records_after_insert() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Alice"}},"labels":["team:a"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Bob"}},"labels":["team:b"]})).await;
    let body: serde_json::Value = client.get(format!("{}/records", base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_create_record_returns_201() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.post(format!("{}/records", base))
        .json(&serde_json::json!({"columns":{"name":{"type":"text","value":"Alice"},"age":{"type":"integer","value":30},"active":{"type":"boolean","value":true}},"labels":["dept:eng","env:prod"]}))
        .send().await.unwrap();
    assert_eq!(res.status(), 201);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["id"], 1);
    assert_eq!(body["columns"]["name"]["value"], "Alice");
    assert_eq!(body["columns"]["age"]["value"], 30);
    assert!(body["labels"].as_array().unwrap().contains(&serde_json::json!("dept:eng")));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_create_record_auto_id_increments() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r1: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let r2: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    assert_eq!(r1["id"], 1);
    assert_eq!(r2["id"], 2);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_create_record_duplicate_id_returns_409() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"id":42,"columns":{},"labels":[]})).await;
    let res = client.post(format!("{}/records", base))
        .json(&serde_json::json!({"id":42,"columns":{},"labels":[]}))
        .send().await.unwrap();
    assert_eq!(res.status(), 409);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_get_record_by_id() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let created: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Carol"}},"labels":["vip"]})).await;
    let id = created["id"].as_u64().unwrap();
    let body: serde_json::Value = client.get(format!("{}/records/{}", base, id)).send().await.unwrap().json().await.unwrap();
    assert_eq!(body["columns"]["name"]["value"], "Carol");
    assert!(body["labels"].as_array().unwrap().contains(&serde_json::json!("vip")));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_get_record_not_found_returns_404() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.get(format!("{}/records/9999", base)).send().await.unwrap();
    assert_eq!(res.status(), 404);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_get_records_by_label() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["grp:x"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["grp:x"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["grp:y"]})).await;
    let body: serde_json::Value = client.get(format!("{}/records/label/grp:x", base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_record() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let created: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{"score":{"type":"integer","value":50}},"labels":["old"]})).await;
    let id = created["id"].as_u64().unwrap();
    let updated: serde_json::Value = client.put(format!("{}/records/{}", base, id))
        .json(&serde_json::json!({"columns":{"score":{"type":"integer","value":99}},"labels":["new"]}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(updated["columns"]["score"]["value"], 99);
    assert!(updated["labels"].as_array().unwrap().contains(&serde_json::json!("new")));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_nonexistent_returns_404() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.put(format!("{}/records/9999", base))
        .json(&serde_json::json!({"columns":{},"labels":[]}))
        .send().await.unwrap();
    assert_eq!(res.status(), 404);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_record_returns_204() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let created: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = created["id"].as_u64().unwrap();
    let res = client.delete(format!("{}/records/{}", base, id)).send().await.unwrap();
    assert_eq!(res.status(), 204);
    let res2 = client.get(format!("{}/records/{}", base, id)).send().await.unwrap();
    assert_eq!(res2.status(), 404);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_nonexistent_returns_404() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.delete(format!("{}/records/9999", base)).send().await.unwrap();
    assert_eq!(res.status(), 404);
    let _ = std::fs::remove_file(db_path);
}
