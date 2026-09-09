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
async fn test_rest_backup_list_download_and_restore() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('alice')").await;
    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('bob')").await;

    // POST /backup（保存先は設定の既定＝ <db_path のディレクトリ>/backups）
    let res = client.post(format!("{}/backup", base)).json(&serde_json::json!({})).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let name = body["name"].as_str().unwrap().to_string();
    assert_eq!(body["records"], 2);

    // GET /backup で一覧に出る
    let res = client.get(format!("{}/backup", base)).send().await.unwrap();
    let list: serde_json::Value = res.json().await.unwrap();
    assert!(list["files"].as_array().unwrap().iter().any(|f| f["name"] == serde_json::json!(name)));

    // GET /backup/download でバイト列が取れる（先頭マジック KBAK1\n）
    let res = client.get(format!("{}/backup/download?name={}", base, name)).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let bytes = res.bytes().await.unwrap();
    assert_eq!(&bytes[..6], b"KBAK1\n");

    // パストラバーサルは弾く
    let res = client.get(format!("{}/backup/download?name=../secret.kbak", base)).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 400);

    // 追加してから名前指定で復元
    post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('carol')").await;
    let res = client.post(format!("{}/backup/restore", base)).json(&serde_json::json!({ "name": name })).send().await.unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["restart_required"], true);
    assert_eq!(body["records"], 2);

    // 復元後は書き込み 409
    let (status, _) = post_sql(&client, &base, "INSERT INTO (label.emp) (name) VALUE ('dave')").await;
    assert_eq!(status, 409);

    let restored = std::fs::read_to_string(&db_path).unwrap();
    assert!(restored.contains("alice") && !restored.contains("carol"));

    let dir = db_path.parent().unwrap().join("backups");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_settings_exposes_backup_generation_fields() {
    let (base, db_path) = spawn_test_server().await;
    let client = reqwest::Client::new();

    let res = client.get(format!("{}/settings", base)).send().await.unwrap();
    let s: serde_json::Value = res.json().await.unwrap();
    assert_eq!(s["backup_max_generations"], 7);
    assert_eq!(s["backup_retention_days"], 30);

    let res = client.put(format!("{}/settings", base))
        .json(&serde_json::json!({ "backup_max_generations": 3, "backup_retention_days": 0 }))
        .send().await.unwrap();
    let s: serde_json::Value = res.json().await.unwrap();
    assert_eq!(s["backup_max_generations"], 3);
    assert_eq!(s["backup_retention_days"], 0);

    // サイドカーに永続化されている
    assert!(std::path::Path::new(&format!("{}.backup.json", db_path.to_string_lossy())).is_file());

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(format!("{}.backup.json", db_path.to_string_lossy()));
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
