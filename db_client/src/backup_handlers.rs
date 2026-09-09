// src/backup_handlers.rs
// バックアップ／復元の REST エンドポイント（Web 管理 UI から使う）。
//
//   POST /backup                 サーバー上にバックアップを作成
//   GET  /backup                 保存先ディレクトリのバックアップ一覧
//   GET  /backup/download?name=  1つの .kbak をダウンロード
//   POST /backup/restore         保存先の .kbak を名前指定で復元（JSON: { name, old_key? }）
//   POST /backup/restore/upload  アップロードした .kbak を復元（multipart: file, old_key?）
//
// すべて **管理者ロール `kagura` のみ**・**トランザクション中は 409**。
// 復元は SQL の `RESTORE FROM` と同じく、適用後の反映にサーバー再起動が必要。

use std::path::{Path, PathBuf};

use axum::{
    extract::{Multipart, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth_handlers::resolve_actor;
use crate::backup_settings;
use crate::handlers::{auto_save, expire_stale_transaction, AppState, AppStateInner};

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": msg.into() })))
}

/// 管理者チェック＋トランザクション拒否＋（write 操作なら）復元適用済みチェックの共通前処理。
/// 成功時は actor 名を返す。
fn guard(inner: &mut AppStateInner, headers: &HeaderMap, is_write: bool) -> Result<String, (StatusCode, Json<Value>)> {
    expire_stale_transaction(inner);
    let Some(actor) = resolve_actor(inner, headers) else {
        return Err(err(StatusCode::UNAUTHORIZED, "認証が必要です。/auth/login でログインしてください。"));
    };
    if !inner.auth.is_admin(&actor) {
        return Err(err(StatusCode::FORBIDDEN, "バックアップ／復元は管理者ロール(kagura)のみ実行できます。"));
    }
    if is_write {
        if inner.active_txn.is_some() {
            return Err(err(StatusCode::CONFLICT, "トランザクション中はバックアップ／復元を実行できません。COMMIT または ROLLBACK してください。"));
        }
        if inner.restore_pending {
            return Err(err(StatusCode::CONFLICT, "バックアップからの復元を適用済みです。変更を反映するにはサーバーを再起動してください。"));
        }
    }
    Ok(actor)
}

fn backup_dir_for(inner: &AppStateInner) -> PathBuf {
    backup_settings::load_or_default(&inner.db_path).resolved_dir(&inner.db_path)
}

/// `name` がディレクトリを跨がない `kagura-backup-*.kbak` であることを検証し、絶対パスを返す。
fn safe_backup_path(dir: &Path, name: &str) -> Result<PathBuf, (StatusCode, Json<Value>)> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.starts_with("kagura-backup-")
        || !name.ends_with(".kbak");
    if bad {
        return Err(err(StatusCode::BAD_REQUEST, "不正なバックアップ名です"));
    }
    Ok(dir.join(name))
}

// ---------------------------------------------------------------------------
// POST /backup
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct CreateBackupRequest {
    /// 保存先。省略時は設定の保存先ディレクトリ。
    #[serde(default)]
    pub dest: Option<String>,
    #[serde(default)]
    pub with_key: bool,
    /// `auth.json` を同梱するか（既定 true）。
    #[serde(default = "default_true")]
    pub include_auth: bool,
}
fn default_true() -> bool { true }

pub async fn create_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<CreateBackupRequest>>,
) -> impl IntoResponse {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let mut inner = state.write().await;
    let actor = match guard(&mut inner, &headers, true) {
        Ok(a) => a,
        Err(e) => return e,
    };

    auto_save(&mut inner);
    let db_path = inner.db_path.clone();
    let auth_path = inner.auth.file_path().to_string();
    let record_count = inner.mgr.db().count() as u64;
    let settings = backup_settings::load_or_default(&db_path);
    let dest = req.dest.unwrap_or_default();

    match crate::backup::run_backup(
        &db_path, &auth_path, &dest, req.with_key, req.include_auth, &actor, record_count, &settings,
    ) {
        Ok(o) => {
            inner.logger.db_info(format!(
                "backup created by '{}' via REST: {} ({} bytes); pruned {}",
                actor, o.path.display(), o.size, o.pruned.len()
            ));
            (
                StatusCode::OK,
                Json(json!({
                    "path": o.path.display().to_string(),
                    "name": o.path.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "bytes": o.size,
                    "records": o.manifest.record_count,
                    "with_key": o.manifest.embedded_key.is_some(),
                    "includes_auth": o.manifest.includes_auth,
                    "key_fingerprint": o.manifest.key_fingerprint,
                    "pruned": o.pruned,
                })),
            )
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, format!("バックアップに失敗しました: {}", e)),
    }
}

// ---------------------------------------------------------------------------
// GET /backup
// ---------------------------------------------------------------------------

pub async fn list_backups(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let mut inner = state.write().await;
    if let Err(e) = guard(&mut inner, &headers, false) {
        return e;
    }
    let dir = backup_dir_for(&inner);
    let mut files: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("kagura-backup-") || !name.ends_with(".kbak") {
                continue;
            }
            let meta = entry.metadata().ok();
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            // マニフェストの要点も返す（壊れていても一覧自体は返す）
            let manifest = db_engine::backup::read_manifest(&entry.path()).ok();
            files.push(json!({
                "name": name,
                "bytes": bytes,
                "modified_unix": modified,
                "created_at": manifest.as_ref().map(|m| m.created_at.clone()),
                "kagura_version": manifest.as_ref().map(|m| m.kagura_version.clone()),
                "records": manifest.as_ref().map(|m| m.record_count),
                "includes_auth": manifest.as_ref().map(|m| m.includes_auth),
                "with_key": manifest.as_ref().map(|m| m.embedded_key.is_some()),
                "key_fingerprint": manifest.as_ref().map(|m| m.key_fingerprint.clone()),
            }));
        }
    }
    files.sort_by(|a, b| {
        b["modified_unix"].as_u64().unwrap_or(0).cmp(&a["modified_unix"].as_u64().unwrap_or(0))
    });
    (
        StatusCode::OK,
        Json(json!({
            "backup_dir": dir.to_string_lossy(),
            "current_key_fingerprint": if inner.db_path.ends_with(".kdb") {
                Some(db_engine::kdb_store::master_key_fingerprint())
            } else {
                None
            },
            "files": files,
        })),
    )
}

// ---------------------------------------------------------------------------
// GET /backup/download?name=
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    pub name: String,
}

pub async fn download_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> axum::response::Response {
    let mut inner = state.write().await;
    if let Err(e) = guard(&mut inner, &headers, false) {
        return e.into_response();
    }
    let dir = backup_dir_for(&inner);
    let path = match safe_backup_path(&dir, &q.name) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    drop(inner);
    match std::fs::read(&path) {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", q.name)),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => err(StatusCode::NOT_FOUND, "バックアップファイルが見つかりません").into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /backup/restore   (JSON: { name, old_key? })
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RestoreByNameRequest {
    pub name: String,
    #[serde(default)]
    pub old_key: Option<String>,
}

pub async fn restore_backup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RestoreByNameRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    let actor = match guard(&mut inner, &headers, true) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let dir = backup_dir_for(&inner);
    let path = match safe_backup_path(&dir, &req.name) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let db_path = inner.db_path.clone();
    let auth_path = inner.auth.file_path().to_string();
    apply_restore(&mut inner, &db_path, &auth_path, &path.to_string_lossy(), req.old_key.as_deref(), &actor)
}

// ---------------------------------------------------------------------------
// POST /backup/restore/upload   (multipart: file, old_key?)
// ---------------------------------------------------------------------------

pub async fn restore_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    let actor = match guard(&mut inner, &headers, true) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut old_key: Option<String> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return err(StatusCode::BAD_REQUEST, format!("アップロードの解析に失敗しました: {}", e)),
        };
        match field.name() {
            Some("file") => {
                match field.bytes().await {
                    Ok(b) => file_bytes = Some(b.to_vec()),
                    Err(e) => return err(StatusCode::BAD_REQUEST, format!("ファイルの読み込みに失敗しました: {}", e)),
                }
            }
            Some("old_key") => {
                old_key = field.text().await.ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }

    let Some(bytes) = file_bytes else {
        return err(StatusCode::BAD_REQUEST, "`file` フィールド（.kbak）が必要です");
    };

    // 一時ファイルへ書き出してから復元する
    let db_path = inner.db_path.clone();
    let auth_path = inner.auth.file_path().to_string();
    let work_parent = PathBuf::from(&db_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let tmp = work_parent.join(format!(
        ".kagura-upload-{}.kbak",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
    ));
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("一時ファイルの書き込みに失敗しました: {}", e));
    }
    let res = apply_restore(&mut inner, &db_path, &auth_path, &tmp.to_string_lossy(), old_key.as_deref(), &actor);
    let _ = std::fs::remove_file(&tmp);
    res
}

fn apply_restore(
    inner: &mut AppStateInner,
    db_path: &str,
    auth_path: &str,
    archive: &str,
    old_key: Option<&str>,
    actor: &str,
) -> (StatusCode, Json<Value>) {
    match crate::backup::run_restore(db_path, auth_path, archive, old_key) {
        Ok(o) => {
            inner.restore_pending = true;
            inner.logger.db_warn(format!(
                "RESTORE applied by '{}' via REST (created {}, {} record(s), rekeyed={}); pre-restore copy at {}. RESTART REQUIRED.",
                actor, o.manifest.created_at, o.manifest.record_count, o.rekeyed, o.pre_restore_dir.display()
            ));
            (
                StatusCode::OK,
                Json(json!({
                    "status": "復元しました。反映にはサーバーの再起動が必要です",
                    "records": o.manifest.record_count,
                    "rekeyed": o.rekeyed,
                    "pre_restore_dir": o.pre_restore_dir.display().to_string(),
                    "restart_required": true,
                })),
            )
        }
        Err(e) => err(StatusCode::BAD_REQUEST, format!("復元に失敗しました: {}", e)),
    }
}
