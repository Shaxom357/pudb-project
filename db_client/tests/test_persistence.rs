
use db_client::test_helpers::{spawn_test_server, post_record};
use dynamic_label_management::LabelManager;
use db_engine::Database;

// 保存→再起動後にデータが復元されることを検証
#[tokio::test]
async fn test_data_persists_after_restart() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // レコード3件挿入
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Alice"}},"labels":["dept:eng","env:prod"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Bob"}},"labels":["dept:eng"]})).await;
    post_record(&client, &base, serde_json::json!({"columns":{"name":{"type":"text","value":"Carol"}},"labels":["dept:sales"]})).await;

    // サーバー停止後、直接 DB をロードして検証
    let db2 = Database::load(&db_path).unwrap();
    assert_eq!(db2.count(), 3);
    // IDで取得
    assert_eq!(db2.get(1).unwrap().get_col("name"),
        Some(&db_engine::DataType::Text("Alice".to_string())));
    // ラベルインデックスも復元されている
    assert_eq!(db2.get_by_label("dept:eng").len(), 2);
    assert_eq!(db2.get_by_label("env:prod").len(), 1);

    let _ = std::fs::remove_file(&db_path);
}

// 削除レコードは保存ファイルに含まれないことを検証
#[tokio::test]
async fn test_deleted_records_not_persisted() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let r1: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":["keep"]})).await;
    let r2: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":["delete-me"]})).await;
    let id2 = r2["id"].as_u64().unwrap();
    let _id1 = r1["id"].as_u64().unwrap();

    // id2 を削除
    client.delete(format!("{}/records/{}", base, id2)).send().await.unwrap();

    let db2 = Database::load(&db_path).unwrap();
    assert_eq!(db2.count(), 1);
    assert!(db2.get(id2).is_err()); // 削除済みは復元されない
    let _ = std::fs::remove_file(&db_path);
}

// next_id が保存・復元されることを検証
#[tokio::test]
async fn test_next_id_preserved_after_reload() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // 5件挿入してサーバー停止
    for _ in 0..5 {
        post_record(&client, &base, serde_json::json!({"columns":{},"labels":[]})).await;
    }

    // 再起動後に新規挿入した場合、id=6 になるはず
    let db_path2 = db_path.clone();
    let mgr2 = LabelManager::from_db(Database::load(&db_path2).unwrap());
    let (app2, _) = db_client::app::build_app(mgr2, db_path2.to_string_lossy().to_string());
    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener2, app2).await.unwrap(); });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let base2 = format!("http://{}", addr2);
    let client2 = reqwest::Client::new();
    let r: serde_json::Value = post_record(&client2, &base2, serde_json::json!({"columns":{},"labels":[]})).await;
    assert_eq!(r["id"], 6);

    let _ = std::fs::remove_file(&db_path);
}

// ラベル操作後のファイルも正しく復元されることを検証
#[tokio::test]
async fn test_label_changes_persisted() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let r: serde_json::Value = post_record(&client, &base, serde_json::json!({"columns":{},"labels":["initial"]})).await;
    let id = r["id"].as_u64().unwrap();

    // ラベル追加
    client.post(format!("{}/records/{}/labels", base, id))
        .json(&serde_json::json!({"label": "added"}))
        .send().await.unwrap();
    // ラベル削除
    client.delete(format!("{}/records/{}/labels/initial", base, id)).send().await.unwrap();

    // 直接ロードして検証
    let db2 = Database::load(&db_path).unwrap();
    let record = db2.get(id).unwrap();
    assert!(record.labels.contains(&"added".to_string()));
    assert!(!record.labels.contains(&"initial".to_string()));
    assert_eq!(db2.get_by_label("added").len(), 1);
    assert_eq!(db2.get_by_label("initial").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}
