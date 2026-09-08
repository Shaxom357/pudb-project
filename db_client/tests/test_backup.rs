// tests/test_backup.rs
// SQL の BACKUP TO / RESTORE FROM を HTTP 経由（POST /sql）で検証する統合テスト。
// テストサーバーは JSON モード（`spawn_test_server`）で動くため、マスターキー／リキーは
// 対象外（それらは db_engine::backup / kdb_store のユニットテストで検証済み）。

use db_client::test_helpers::spawn_test_server;

async fn post_sql(client: &reqwest::Client, base: &str, query: &str) -> (u16, serde_json::Value) {
    let res = client
        .post(format!("{}/sql", base))
        .json(&serde_json::json!({ "query": query }))
        .send()
        .await
        .unwrap();
    let status = res.status().as_u16();
    let body: serde_json::Value = res.json().await.unwrap();
    (status, body)
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "kdb_backup_it_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test]
async fn test_backup_then_restore_roundtrip_and_restart_guard() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let backup_dir = unique_dir("roundtrip");

    // データを 2 件投入
    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('alice')").await;
    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('bob')").await;

    // バックアップ作成
    let (status, body) = post_sql(
        &client,
        &base,
        &format!("BACKUP TO '{}/'", backup_dir.to_string_lossy()),
    )
    .await;
    assert_eq!(status, 200, "backup should succeed: {body}");
    assert_eq!(body["ok"], true);
    let archive_path = body["rows"][0][0].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&archive_path).is_file(), "archive file exists");
    assert_eq!(body["rows"][0][2], 2, "record count in manifest");

    // さらに 1 件足す（このあと復元で消えるはず）
    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('carol')").await;
    let (_, body) = post_sql(&client, &base, "SELECT * FROM label.emp").await;
    assert_eq!(body["returned"], 3);

    // 復元
    let (status, body) = post_sql(&client, &base, &format!("RESTORE FROM '{archive_path}'")).await;
    assert_eq!(status, 200, "restore should succeed: {body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["columns"], serde_json::json!(["status", "records", "rekeyed", "pre_restore_dir"]));
    assert_eq!(body["rows"][0][1], 2, "restored manifest record count");
    assert_eq!(body["rows"][0][2], false, "no rekey in JSON mode");
    let pre_restore_dir = body["rows"][0][3].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&pre_restore_dir).is_dir(), "pre-restore copy kept");

    // 復元後は書き込みが 409（再起動が必要）
    let (status, body) = post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('dave')").await;
    assert_eq!(status, 409, "writes blocked after restore");
    assert!(body["error"].as_str().unwrap().contains("再起動"));

    // 参照は引き続き可能（メモリ上はまだ復元前の 3 件だが、SELECT は許可される）
    let (status, _) = post_sql(&client, &base, "SELECT * FROM label.emp").await;
    assert_eq!(status, 200);

    // ディスク上の JSON 本体は復元済み（2 件）であることを確認
    let restored = std::fs::read_to_string(&db_path).unwrap();
    assert!(restored.contains("alice") && restored.contains("bob"));
    assert!(!restored.contains("carol"), "carol should be gone from disk");

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(&backup_dir);
    let _ = std::fs::remove_dir_all(&pre_restore_dir);
}

#[tokio::test]
async fn test_backup_rejects_bad_syntax_and_missing_archive() {
    let (base, _db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let (status, _) = post_sql(&client, &base, "BACKUP TO /no/quotes").await;
    assert_eq!(status, 400);

    let (status, body) = post_sql(&client, &base, "RESTORE FROM '/tmp/does-not-exist-xyz.kbak'").await;
    assert_eq!(status, 400);
    assert!(body["error"].as_str().unwrap().contains("見つかりません"));
}

#[tokio::test]
async fn test_backup_blocked_during_transaction() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();
    let backup_dir = unique_dir("txn");

    post_sql(&client, &base, "BEGIN").await;
    let (status, body) = post_sql(
        &client,
        &base,
        &format!("BACKUP TO '{}/'", backup_dir.to_string_lossy()),
    )
    .await;
    assert_eq!(status, 409, "backup not allowed during a transaction: {body}");
    post_sql(&client, &base, "ROLLBACK").await;

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(&backup_dir);
}
