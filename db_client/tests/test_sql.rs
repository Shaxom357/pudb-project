
use db_client::test_helpers::spawn_test_server;

async fn post_sql(client: &reqwest::Client, base: &str, query: &str) -> (u16, serde_json::Value) {
    let res = client.post(format!("{}/sql", base))
        .json(&serde_json::json!({ "query": query }))
        .send().await.unwrap();
    let status = res.status().as_u16();
    let body: serde_json::Value = res.json().await.unwrap();
    (status, body)
}

#[tokio::test]
async fn test_insert_with_explicit_column_order_returns_201() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base,
        "INSERT INTO ('label.employee') (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')"
    ).await;

    assert_eq!(status, 201);
    assert_eq!(body["ok"], true);
    assert_eq!(body["inserted_id"], 1);
    assert_eq!(body["labels"], serde_json::json!(["employee"]));
    assert_eq!(body["columns"], serde_json::json!(["employee_name", "employee_age", "employee_department"]));
    assert_eq!(body["rows"][0], serde_json::json!(["田中", 24, "developer"]));

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_inserted_record_is_selectable() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (employee_name, employee_age) VALUE ('田中', 24)"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "SELECT * FROM label.employee WHERE employee_name = '田中'"
    ).await;

    assert_eq!(status, 200);
    assert_eq!(body["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_multiple_labels() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (_, body) = post_sql(&client, &base,
        "INSERT INTO (label.employee, label.manager) (employee_name) VALUE ('田中')"
    ).await;
    assert_eq!(body["labels"], serde_json::json!(["employee", "manager"]));

    let (status, sel) = post_sql(&client, &base, "SELECT * FROM label.manager").await;
    assert_eq!(status, 200);
    assert_eq!(sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_creates_new_label_dynamically() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.brandnew) (name) VALUE ('x')").await;

    let (status, labels) = post_sql(&client, &base, "SELECT * FROM label.brandnew").await;
    assert_eq!(status, 200);
    assert_eq!(labels["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_rejects_empty_label() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base, "INSERT INTO (label.*) (name) VALUE ('x')").await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().len() > 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_without_column_order_on_empty_db_errors() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base, "INSERT INTO (label.employee) VALUE ('田中')").await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_column_value_count_mismatch_errors() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base,
        "INSERT INTO (label.employee) (employee_name) VALUE ('田中', 24)"
    ).await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_uses_existing_column_order_when_omitted() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (age, name) VALUE (24, '田中')"
    ).await;
    let (status, body) = post_sql(&client, &base,
        "INSERT INTO (label.employee) VALUE (31, 'Suzuki')"
    ).await;

    assert_eq!(status, 201);
    assert_eq!(body["columns"], serde_json::json!(["age", "name"]));
    assert_eq!(body["rows"][0], serde_json::json!([31, "Suzuki"]));

    let _ = std::fs::remove_file(db_path);
}
