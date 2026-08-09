
use db_client::test_helpers::{spawn_test_server, post_record};

// GET /records/{id}/labels
#[tokio::test]
async fn test_list_record_labels() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":["z","a","m"]})).await;
    let id = r["id"].as_u64().unwrap();
    let labels: Vec<String> = client.get(format!("{}/records/{}/labels", base, id)).send().await.unwrap().json().await.unwrap();
    // ソート済みで返ることを確認
    assert_eq!(labels, vec!["a", "m", "z"]);
    let _ = std::fs::remove_file(db_path);
}

// POST /records/{id}/labels
#[tokio::test]
async fn test_add_label_to_record() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = r["id"].as_u64().unwrap();
    let labels: Vec<String> = client.post(format!("{}/records/{}/labels", base, id))
        .json(&serde_json::json!({"label": "new-tag"}))
        .send().await.unwrap().json().await.unwrap();
    assert!(labels.contains(&"new-tag".to_string()));
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_add_label_idempotent() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = r["id"].as_u64().unwrap();
    let url = format!("{}/records/{}/labels", base, id);
    let payload = serde_json::json!({"label": "dup"});
    client.post(&url).json(&payload).send().await.unwrap();
    let labels: Vec<String> = client.post(&url).json(&payload).send().await.unwrap().json().await.unwrap();
    assert_eq!(labels.iter().filter(|l| l.as_str() == "dup").count(), 1);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_add_label_invalid_name_returns_422() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = r["id"].as_u64().unwrap();
    let res = client.post(format!("{}/records/{}/labels", base, id))
        .json(&serde_json::json!({"label": "invalid label"}))
        .send().await.unwrap();
    assert_eq!(res.status(), 422);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_add_label_empty_name_returns_422() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = r["id"].as_u64().unwrap();
    let res = client.post(format!("{}/records/{}/labels", base, id))
        .json(&serde_json::json!({"label": ""}))
        .send().await.unwrap();
    assert_eq!(res.status(), 422);
    let _ = std::fs::remove_file(db_path);
}

// DELETE /records/{id}/labels/{label}
#[tokio::test]
async fn test_remove_label_returns_204() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":["remove-me"]})).await;
    let id = r["id"].as_u64().unwrap();
    let res = client.delete(format!("{}/records/{}/labels/remove-me", base, id)).send().await.unwrap();
    assert_eq!(res.status(), 204);
    let labels: Vec<String> = client.get(format!("{}/records/{}/labels", base, id)).send().await.unwrap().json().await.unwrap();
    assert!(labels.is_empty());
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_remove_nonexistent_label_returns_404() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    let id = r["id"].as_u64().unwrap();
    let res = client.delete(format!("{}/records/{}/labels/no-such-label", base, id)).send().await.unwrap();
    assert_eq!(res.status(), 404);
    let _ = std::fs::remove_file(db_path);
}

// GET /labels
#[tokio::test]
async fn test_list_all_labels_with_stats() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["popular","rare"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["popular"]})).await;
    let body: serde_json::Value = client.get(format!("{}/labels", base)).send().await.unwrap().json().await.unwrap();
    let labels = body["labels"].as_array().unwrap();
    assert!(labels.contains(&serde_json::json!("popular")));
    assert!(labels.contains(&serde_json::json!("rare")));
    let stats = body["stats"].as_array().unwrap();
    // popular が先頭（2件）
    assert_eq!(stats[0]["label"], "popular");
    assert_eq!(stats[0]["record_count"], 2);
    let _ = std::fs::remove_file(db_path);
}

// POST /labels/search
#[tokio::test]
async fn test_search_labels_and() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"A"}},"labels":["backend","senior"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"B"}},"labels":["backend"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"C"}},"labels":["frontend","senior"]})).await;
    let body: serde_json::Value = client.post(format!("{}/labels/search", base))
        .json(&serde_json::json!({"labels":["backend","senior"],"mode":"and"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(body["matched_count"], 1);
    assert_eq!(body["mode"], "and");
    assert_eq!(body["records"][0]["columns"]["name"]["value"], "A");
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_search_labels_or() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["rust"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["go"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["java"]})).await;
    let body: serde_json::Value = client.post(format!("{}/labels/search", base))
        .json(&serde_json::json!({"labels":["rust","go"],"mode":"or"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(body["matched_count"], 2);
    assert_eq!(body["mode"], "or");
    let _ = std::fs::remove_file(db_path);
}

// PUT /labels/rename
#[tokio::test]
async fn test_rename_label() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["old-name"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{},"labels":["old-name"]})).await;
    let body: serde_json::Value = client.put(format!("{}/labels/rename", base))
        .json(&serde_json::json!({"old_label":"old-name","new_label":"new-name"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(body["updated_count"], 2);
    // 元のラベルは消えている
    let search_old: serde_json::Value = client.post(format!("{}/labels/search", base))
        .json(&serde_json::json!({"labels":["old-name"],"mode":"or"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(search_old["matched_count"], 0);
    // 新しいラベルに移っている
    let search_new: serde_json::Value = client.post(format!("{}/labels/search", base))
        .json(&serde_json::json!({"labels":["new-name"],"mode":"or"}))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(search_new["matched_count"], 2);
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_rename_label_invalid_returns_422() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let res = client.put(format!("{}/labels/rename", base))
        .json(&serde_json::json!({"old_label":"valid","new_label":"invalid name"}))
        .send().await.unwrap();
    assert_eq!(res.status(), 422);
    let _ = std::fs::remove_file(db_path);
}
