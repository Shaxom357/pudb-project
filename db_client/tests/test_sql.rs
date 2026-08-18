
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
async fn test_select_star_includes_id_and_labels_columns() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee, label.manager) (employee_name) VALUE ('田中')"
    ).await;

    let (status, body) = post_sql(&client, &base, "SELECT * FROM label.*").await;

    assert_eq!(status, 200);
    assert_eq!(body["columns"], serde_json::json!(["id", "employee_name", "labels"]));
    let row = body["rows"][0].as_array().unwrap();
    assert_eq!(row[0], serde_json::json!(1));
    assert_eq!(row[1], serde_json::json!("田中"));
    assert_eq!(row[2], serde_json::json!("employee,manager"));

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

#[tokio::test]
async fn test_update_data_sets_matching_row_and_keeps_others() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (employee_name, employee_age) VALUE ('木邑', 24)"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["updated_count"], 1);

    let (status, sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(status, 200);
    let row = sel["rows"][0].as_array().unwrap();
    let cols: Vec<&str> = sel["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    let name_idx = cols.iter().position(|&c| c == "employee_name").unwrap();
    let age_idx = cols.iter().position(|&c| c == "employee_age").unwrap();
    assert_eq!(row[name_idx], serde_json::json!("木村"));
    // SET対象でないカラムは維持される
    assert_eq!(row[age_idx], serde_json::json!(24));

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_data_no_match_returns_zero_count() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (employee_name) VALUE ('田中')"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "UPDATE label.employee SET employee_name='X' WHERE employee_name='存在しない'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["updated_count"], 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_label_rename_moves_records() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (name) VALUE ('田中')"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "UPDATE LABEL label.employee SET label.staff"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["updated_count"], 1);

    let (status, old_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(status, 200);
    assert_eq!(old_sel["returned"], 0);

    let (status, new_sel) = post_sql(&client, &base, "SELECT * FROM label.staff").await;
    assert_eq!(status, 200);
    assert_eq!(new_sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_label_rename_with_where_only_renames_matching_records() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (name, department) VALUE ('田中', 'sales')"
    ).await;
    post_sql(&client, &base,
        "INSERT INTO (label.employee) (name, department) VALUE ('鈴木', 'dev')"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "UPDATE LABEL label.employee SET label.sales_staff WHERE department='sales'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["updated_count"], 1);

    let (_, employee_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(employee_sel["returned"], 1);
    let (_, sales_sel) = post_sql(&client, &base, "SELECT * FROM label.sales_staff").await;
    assert_eq!(sales_sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_label_rename_rejects_duplicate_target() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.employee) (name) VALUE ('田中')").await;
    post_sql(&client, &base, "INSERT INTO (label.staff) (name) VALUE ('鈴木')").await;

    let (status, body) = post_sql(&client, &base, "UPDATE LABEL label.employee SET label.staff").await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().len() > 0);

    // 何も変更されていないこと
    let (_, employee_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(employee_sel["returned"], 1);
    let (_, staff_sel) = post_sql(&client, &base, "SELECT * FROM label.staff").await;
    assert_eq!(staff_sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_insert_rejects_duplicate_label_in_same_statement() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base,
        "INSERT INTO (label.employee, label.employee) (name) VALUE ('田中')"
    ).await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().len() > 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_update_rejects_malformed_syntax() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base, "UPDATE label.employee SET").await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().len() > 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_data_with_where_removes_matching_row_and_keeps_others() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.employee) (employee_name) VALUE ('田中')").await;
    post_sql(&client, &base, "INSERT INTO (label.employee) (employee_name) VALUE ('鈴木')").await;

    let (status, body) = post_sql(&client, &base,
        "DELETE FROM label.employee WHERE employee_name='田中'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["deleted_count"], 1);

    let (status, sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(status, 200);
    assert_eq!(sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_data_without_where_removes_all_records_in_label() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.employee) (name) VALUE ('田中')").await;
    post_sql(&client, &base, "INSERT INTO (label.employee) (name) VALUE ('鈴木')").await;
    post_sql(&client, &base, "INSERT INTO (label.manager) (name) VALUE ('佐藤')").await;

    let (status, body) = post_sql(&client, &base, "DELETE FROM label.employee").await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted_count"], 2);

    let (_, employee_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(employee_sel["returned"], 0);
    // 別ラベルのレコードは影響を受けない
    let (_, manager_sel) = post_sql(&client, &base, "SELECT * FROM label.manager").await;
    assert_eq!(manager_sel["returned"], 1);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_from_label_star_removes_all_data() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.employee) (name) VALUE ('田中')").await;
    post_sql(&client, &base, "INSERT INTO (label.manager) (name) VALUE ('佐藤')").await;

    let (status, body) = post_sql(&client, &base, "DELETE FROM label.*").await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted_count"], 2);

    let (_, sel) = post_sql(&client, &base, "SELECT * FROM label.*").await;
    assert_eq!(sel["returned"], 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_label_only_detaches_label_and_keeps_data() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee, label.manager) (name) VALUE ('田中')"
    ).await;

    let (status, body) = post_sql(&client, &base, "DELETE LABEL FROM label.employee").await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted_count"], 1);

    // employeeラベルからは検索できなくなるが、レコード自体はmanagerラベル経由で残っている
    let (_, employee_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(employee_sel["returned"], 0);
    let (_, manager_sel) = post_sql(&client, &base, "SELECT * FROM label.manager").await;
    assert_eq!(manager_sel["returned"], 1);
    assert_eq!(manager_sel["rows"][0][1], serde_json::json!("田中"));

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_label_with_where_only_affects_matching_records() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base,
        "INSERT INTO (label.employee) (name, department) VALUE ('田中', 'sales')"
    ).await;
    post_sql(&client, &base,
        "INSERT INTO (label.employee) (name, department) VALUE ('鈴木', 'dev')"
    ).await;

    let (status, body) = post_sql(&client, &base,
        "DELETE LABEL FROM label.employee WHERE department='sales'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted_count"], 1);

    let (_, employee_sel) = post_sql(&client, &base, "SELECT * FROM label.employee").await;
    assert_eq!(employee_sel["returned"], 1);
    // データ自体は削除されていない（全レコード数は2のまま）
    let (_, all_sel) = post_sql(&client, &base, "SELECT * FROM label.*").await;
    assert_eq!(all_sel["returned"], 2);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_no_match_returns_zero_count() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.employee) (name) VALUE ('田中')").await;

    let (status, body) = post_sql(&client, &base,
        "DELETE FROM label.employee WHERE name='存在しない'"
    ).await;
    assert_eq!(status, 200);
    assert_eq!(body["deleted_count"], 0);

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_delete_rejects_malformed_syntax() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, body) = post_sql(&client, &base, "DELETE label.employee").await;
    assert_eq!(status, 400);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().len() > 0);

    let _ = std::fs::remove_file(db_path);
}
