// src/sql_handlers.rs
// SQL クエリ実行 API ハンドラー

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use crate::auth::Privilege;
use crate::auth_handlers::{resolve_actor, resolve_actor_readonly};
use crate::bootstrap::reload_database;
use crate::handlers::{auto_save, expire_stale_transaction, AppState, AppStateInner, TxnMeta};
use dynamic_label_management::LabelManager;
use crate::index_sql::{execute_index_statement, parse_index_statement, save_index_definitions, IndexSqlOutcome};
use crate::schema_sql::{execute_schema_statement, parse_schema_statement, save_schema_definitions, SchemaSqlOutcome};
use crate::user_sql::{execute_user_statement, parse_user_statement, UserSqlOutcome};
use sql_engine::{run_select, run_insert_fast, run_update, run_delete, run_explain, CellValue};

// ---------------------------------------------------------------------------
// リクエスト / レスポンス DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SqlQueryRequest {
    /// 実行するSQL文
    pub query: String,
    /// 脆弱なパスワードで `CREATE USER` / `ALTER USER` を実行する際の確認回答。
    /// 未指定=未回答（サーバーは needs_confirmation を返す）, true=続行, false=中止。
    #[serde(default)]
    pub confirm_weak_password: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SqlQueryResponse {
    /// 成功・失敗
    pub ok: bool,
    /// カラム名リスト（成功時）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// 行データ（成功時）各行は columns と同順の値リスト
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<serde_json::Value>>,
    /// LIMIT前のヒット件数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_matched: Option<usize>,
    /// 返却行数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned: Option<usize>,
    /// INSERTで発行されたレコードID（成功時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted_id: Option<u64>,
    /// INSERTで付与されたラベル（成功時）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// UPDATEで更新されたレコード数（成功時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_count: Option<usize>,
    /// DELETEで削除されたレコード数（`DELETE LABEL`の場合はラベルを外したレコード数、成功時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_count: Option<usize>,
    /// エラーメッセージ（失敗時）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 脆弱なパスワードにつき yes/no 確認が必要なとき true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_confirmation: Option<bool>,
    /// needs_confirmation 時にクライアントへ提示する確認文言
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// 実行したSQL（エコーバック）
    pub query: String,
}

impl SqlQueryResponse {
    fn error(query: String, message: impl Into<String>) -> Self {
        SqlQueryResponse {
            ok: false, columns: vec![], rows: vec![], total_matched: None, returned: None,
            inserted_id: None, labels: vec![], updated_count: None, deleted_count: None,
            error: Some(message.into()), needs_confirmation: None, warning: None, query,
        }
    }

    /// SHOW USERS など、表形式の結果をそのまま返す
    fn rows(query: String, columns: Vec<String>, rows: Vec<Vec<serde_json::Value>>) -> Self {
        let returned = rows.len();
        SqlQueryResponse {
            ok: true, columns, rows, total_matched: Some(returned), returned: Some(returned),
            inserted_id: None, labels: vec![], updated_count: None, deleted_count: None,
            error: None, needs_confirmation: None, warning: None, query,
        }
    }

    /// CREATE/DROP/ALTER USER の成功（返す行なし）
    fn acknowledged(query: String) -> Self {
        SqlQueryResponse {
            ok: true, columns: vec![], rows: vec![], total_matched: None, returned: None,
            inserted_id: None, labels: vec![], updated_count: None, deleted_count: None,
            error: None, needs_confirmation: None, warning: None, query,
        }
    }

    /// 脆弱パスワードの確認要求
    fn needs_confirmation(query: String, warning: String) -> Self {
        SqlQueryResponse {
            ok: false, columns: vec![], rows: vec![], total_matched: None, returned: None,
            inserted_id: None, labels: vec![], updated_count: None, deleted_count: None,
            error: None, needs_confirmation: Some(true), warning: Some(warning), query,
        }
    }
}

/// トランザクション実行中の書き込み文（INSERT/UPDATE/DELETE）のガード。
/// - トランザクション無し → `Ok(false)`（通常どおり即時永続化する）
/// - 自分（`actor`）のトランザクション中 → `Ok(true)`（永続化を COMMIT まで遅延する）
/// - 他ユーザーのトランザクション中 → `Err(409)`
fn txn_write_check(
    inner: &AppStateInner,
    actor: &str,
    query: &str,
) -> Result<bool, (StatusCode, Json<SqlQueryResponse>)> {
    match &inner.active_txn {
        None => Ok(false),
        Some(txn) if txn.owner == actor => Ok(true),
        Some(txn) => Err((
            StatusCode::CONFLICT,
            Json(SqlQueryResponse::error(
                query.to_string(),
                format!(
                    "別のユーザー（{}）がトランザクションを実行中です。完了までお待ちください。",
                    txn.owner
                ),
            )),
        )),
    }
}

/// 進行中トランザクションの「最終操作時刻」を更新する（無操作タイムアウトの起点をずらす）。
fn touch_txn(inner: &AppStateInner) {
    if let Some(txn) = &inner.active_txn {
        if let Ok(mut t) = txn.last_activity.lock() {
            *t = std::time::Instant::now();
        }
    }
}

/// DDL 系（ユーザー管理／インデックス管理／スキーマ管理）をトランザクション中に拒否する。
fn reject_ddl_in_txn(inner: &AppStateInner, query: &str) -> Option<(StatusCode, Json<SqlQueryResponse>)> {
    if inner.active_txn.is_some() {
        Some((
            StatusCode::CONFLICT,
            Json(SqlQueryResponse::error(
                query.to_string(),
                "トランザクション中は DDL（ユーザー管理／インデックス管理／スキーマ管理）を実行できません。COMMIT または ROLLBACK してください。",
            )),
        ))
    } else {
        None
    }
}

fn status_response(query: String, message: &str) -> (StatusCode, Json<SqlQueryResponse>) {
    (
        StatusCode::OK,
        Json(SqlQueryResponse::rows(
            query,
            vec!["status".to_string()],
            vec![vec![serde_json::Value::String(message.to_string())]],
        )),
    )
}

fn cell_to_json(cell: &CellValue) -> serde_json::Value {
    match cell {
        CellValue::Text(s)    => serde_json::Value::String(s.clone()),
        CellValue::Integer(n) => serde_json::Value::Number((*n).into()),
        CellValue::Float(f)   => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        CellValue::Boolean(b) => serde_json::Value::Bool(*b),
        CellValue::Null       => serde_json::Value::Null,
    }
}

// ---------------------------------------------------------------------------
// POST /sql  -- SQL クエリを実行する
// ---------------------------------------------------------------------------

pub async fn execute_sql(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SqlQueryRequest>,
) -> impl IntoResponse {
    let query = payload.query.trim().to_string();

    // バックアップ復元を適用済みなら、メモリ上の（復元前）状態でディスクを上書きしないよう
    // 書き込み系の文をすべて 409 で弾く（反映にはサーバー再起動が必要）。参照系は許可する。
    {
        let inner = state.read().await;
        if inner.restore_pending {
            let up = query.trim_start().to_uppercase();
            const WRITE_KEYWORDS: [&str; 16] = [
                "INSERT", "UPDATE", "DELETE", "BEGIN", "START", "COMMIT", "ROLLBACK",
                "CREATE", "DROP", "ALTER", "GRANT", "REVOKE", "BACKUP", "RESTORE",
                "ENABLE", "DISABLE",
            ];
            if WRITE_KEYWORDS.iter().any(|k| up.starts_with(k)) {
                return (
                    StatusCode::CONFLICT,
                    Json(SqlQueryResponse::error(
                        query,
                        "バックアップからの復元を適用済みです。変更を反映するにはサーバーを再起動してください。",
                    )),
                );
            }
        }
    }

    // バックアップ／復元（BACKUP TO / RESTORE FROM）。管理者ロール（kagura）のみ・
    // トランザクション中は不可。SELECT/INSERT/... のディスパッチより前に処理する。
    if let Some(parsed) = crate::backup::parse_backup_statement(&query) {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        if let Some(resp) = reject_ddl_in_txn(&inner, &query) { return resp; }
        let stmt = match parsed {
            Ok(s) => s,
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                return (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e)));
            }
        };
        let Some(actor) = resolve_actor(&mut inner, &headers) else {
            let msg = "認証が必要です。/auth/login でログインしてください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
        };
        if !inner.auth.is_admin(&actor) {
            let msg = "バックアップ／復元は管理者ロール(kagura)のみ実行できます。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::FORBIDDEN, Json(SqlQueryResponse::error(query, msg)));
        }

        return match stmt {
            crate::backup::BackupStatement::Backup { dest, with_key, include_auth } => {
                // 現在のメモリ状態を .kdb/JSON へ確定してからアーカイブする
                auto_save(&mut inner);
                let db_path = inner.db_path.clone();
                let auth_path = inner.auth.file_path().to_string();
                let record_count = inner.mgr.db().count() as u64;
                let settings = crate::backup_settings::load_or_default(&db_path);
                match crate::backup::run_backup(
                    &db_path, &auth_path, &dest, with_key, include_auth, &actor, record_count, &settings,
                ) {
                    Ok(o) => {
                        inner.logger.db_info(format!(
                            "backup created by '{}': {} ({} bytes, {} record(s){}{}); pruned {} old generation(s)",
                            actor, o.path.display(), o.size, o.manifest.record_count,
                            if with_key { ", WITH KEY" } else { "" },
                            if include_auth { "" } else { ", WITHOUT AUTH" },
                            o.pruned.len(),
                        ));
                        inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                        let cols = vec!["path".to_string(), "bytes".to_string(), "records".to_string(), "pruned".to_string()];
                        let row = vec![
                            serde_json::Value::String(o.path.display().to_string()),
                            serde_json::Value::Number(o.size.into()),
                            serde_json::Value::Number(o.manifest.record_count.into()),
                            serde_json::Value::Number((o.pruned.len() as u64).into()),
                        ];
                        (StatusCode::OK, Json(SqlQueryResponse::rows(query, cols, vec![row])))
                    }
                    Err(e) => {
                        inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                        (StatusCode::INTERNAL_SERVER_ERROR, Json(SqlQueryResponse::error(query, format!("バックアップに失敗しました: {}", e))))
                    }
                }
            }
            crate::backup::BackupStatement::Restore { archive, old_key } => {
                let db_path = inner.db_path.clone();
                let auth_path = inner.auth.file_path().to_string();
                match crate::backup::run_restore(&db_path, &auth_path, &archive, old_key.as_deref()) {
                    Ok(o) => {
                        inner.restore_pending = true;
                        inner.logger.db_warn(format!(
                            "RESTORE applied by '{}' from {} (created {}, {} record(s), rekeyed={}); pre-restore copy at {}. RESTART REQUIRED.",
                            actor, archive, o.manifest.created_at, o.manifest.record_count, o.rekeyed,
                            o.pre_restore_dir.display(),
                        ));
                        inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                        let cols = vec!["status".to_string(), "records".to_string(), "rekeyed".to_string(), "pre_restore_dir".to_string()];
                        let row = vec![
                            serde_json::Value::String("復元しました。反映にはサーバーの再起動が必要です".to_string()),
                            serde_json::Value::Number(o.manifest.record_count.into()),
                            serde_json::Value::Bool(o.rekeyed),
                            serde_json::Value::String(o.pre_restore_dir.display().to_string()),
                        ];
                        (StatusCode::OK, Json(SqlQueryResponse::rows(query, cols, vec![row])))
                    }
                    Err(e) => {
                        inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                        (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, format!("復元に失敗しました: {}", e))))
                    }
                }
            }
        };
    }

    // ユーザー管理系（CREATE USER / DROP USER / ALTER USER / SHOW USERS）は
    // レコードストアではなく認証状態(AuthState)を操作する。SELECT/INSERT/... の
    // ディスパッチより前に処理し、該当しなければ None が返って通常処理へ進む。
    if let Some(parsed) = parse_user_statement(&query) {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        if let Some(resp) = reject_ddl_in_txn(&inner, &query) { return resp; }
        let stmt = match parsed {
            Ok(s) => s,
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                return (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e)));
            }
        };
        let Some(actor) = resolve_actor(&mut inner, &headers) else {
            let msg = "認証が必要です。/auth/login でログインしてください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
        };
        return match execute_user_statement(&mut inner.auth, &actor, stmt, payload.confirm_weak_password) {
            UserSqlOutcome::Rows { columns, rows } => {
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::rows(query, columns, rows)))
            }
            UserSqlOutcome::Ok { message } => {
                inner.logger.auth_info(format!("user-mgmt by '{}': {} | query={}", actor, message, query));
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::acknowledged(query)))
            }
            UserSqlOutcome::NeedsConfirmation { warning } => {
                // 確認待ち。ログ上は失敗扱いにしない（未実行）。
                (StatusCode::OK, Json(SqlQueryResponse::needs_confirmation(query, warning)))
            }
            UserSqlOutcome::Err { status, message } => {
                inner.logger.auth_warn(format!("user-mgmt by '{}' failed: {} | query={}", actor, message, query));
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&message));
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                (code, Json(SqlQueryResponse::error(query, message)))
            }
        };
    }

    // 二次インデックス管理系（CREATE INDEX / DROP INDEX / SHOW INDEXES）は、CREATE USER 等と
    // 同様にレコードストアの構造を操作する管理操作のため MANAGE_USERS 権限が必要。
    // インデックスの実体は db_engine::Database 上に構築するが、定義（カラム名）だけを
    // サイドカーファイルへ保存し .kdb/JSON 本体のフォーマットは変更しない。
    if let Some(parsed) = parse_index_statement(&query) {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        if let Some(resp) = reject_ddl_in_txn(&inner, &query) { return resp; }
        let stmt = match parsed {
            Ok(s) => s,
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                return (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e)));
            }
        };
        let Some(actor) = resolve_actor(&mut inner, &headers) else {
            let msg = "認証が必要です。/auth/login でログインしてください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
        };
        if !inner.auth.can_manage_users(&actor) {
            let msg = "この操作には MANAGE_USERS 権限が必要です。管理者に依頼してください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::FORBIDDEN, Json(SqlQueryResponse::error(query, msg)));
        }
        let outcome = execute_index_statement(inner.mgr.db_mut(), stmt);
        // CREATE/DROP が成功した場合だけ、定義をサイドカーファイルへ保存する
        if matches!(outcome, IndexSqlOutcome::Ok { .. }) {
            let db_path = inner.db_path.clone();
            if let Err(e) = save_index_definitions(&db_path, inner.mgr.db()) {
                inner.logger.db_info(format!("インデックス定義の保存に失敗しました: {}", e));
            }
        }
        return match outcome {
            IndexSqlOutcome::Rows { columns, rows } => {
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::rows(query, columns, rows)))
            }
            IndexSqlOutcome::Ok { message } => {
                inner.logger.db_info(format!("index-mgmt by '{}': {} | query={}", actor, message, query));
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::acknowledged(query)))
            }
            IndexSqlOutcome::Err { status, message } => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&message));
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                (code, Json(SqlQueryResponse::error(query, message)))
            }
        };
    }

    // 任意スキーマ層（ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA / DESCRIBE /
    // SHOW SCHEMAS / VALIDATE LABEL）も CREATE USER・CREATE INDEX と同様に MANAGE_USERS 権限
    // が必要な管理操作。定義（カラム名一覧）だけをサイドカーファイルへ保存し、
    // `.kdb`/JSON 本体のフォーマットは変更しない。
    if let Some(parsed) = parse_schema_statement(&query) {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        if let Some(resp) = reject_ddl_in_txn(&inner, &query) { return resp; }
        let stmt = match parsed {
            Ok(s) => s,
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e));
                return (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e)));
            }
        };
        let Some(actor) = resolve_actor(&mut inner, &headers) else {
            let msg = "認証が必要です。/auth/login でログインしてください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
        };
        if !inner.auth.can_manage_users(&actor) {
            let msg = "この操作には MANAGE_USERS 権限が必要です。管理者に依頼してください。";
            inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(msg));
            return (StatusCode::FORBIDDEN, Json(SqlQueryResponse::error(query, msg)));
        }
        let outcome = execute_schema_statement(inner.mgr.db_mut(), stmt);
        // DEFINE/DROP COLUMN・ENABLE/DISABLE SCHEMA が成功した場合だけ、
        // 定義をサイドカーファイルへ保存する（DESCRIBE/SHOW/VALIDATE は読み取り専用）
        if matches!(outcome, SchemaSqlOutcome::Ok { .. }) {
            let db_path = inner.db_path.clone();
            if let Err(e) = save_schema_definitions(&db_path, inner.mgr.db()) {
                inner.logger.db_info(format!("スキーマ定義の保存に失敗しました: {}", e));
            }
        }
        return match outcome {
            SchemaSqlOutcome::Rows { columns, rows } => {
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::rows(query, columns, rows)))
            }
            SchemaSqlOutcome::Ok { message } => {
                inner.logger.db_info(format!("schema-mgmt by '{}': {} | query={}", actor, message, query));
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::acknowledged(query)))
            }
            SchemaSqlOutcome::Err { status, message } => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&message));
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                (code, Json(SqlQueryResponse::error(query, message)))
            }
        };
    }

    // トランザクション制御（BEGIN / BEGIN TRANSACTION / START TRANSACTION / COMMIT / ROLLBACK）。
    // 同時に開けるトランザクションは1つだけ（先着のログインユーザーが所有者になる）。
    {
        let normalized = query.trim().trim_end_matches(';').trim_end().to_uppercase();
        let is_begin = matches!(normalized.as_str(), "BEGIN" | "BEGIN TRANSACTION" | "START TRANSACTION");
        let is_commit = normalized == "COMMIT";
        let is_rollback = normalized == "ROLLBACK";
        if is_begin || is_commit || is_rollback {
            let started = std::time::Instant::now();
            let mut inner = state.write().await;
            let Some(actor) = resolve_actor(&mut inner, &headers) else {
                let msg = "認証が必要です。/auth/login でログインしてください。";
                return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
            };

            if is_begin {
                expire_stale_transaction(&mut inner);
                if let Some(txn) = &inner.active_txn {
                    let msg = format!(
                        "既にトランザクションが実行中です（owner: {}）。先に COMMIT または ROLLBACK してください。",
                        txn.owner
                    );
                    return (StatusCode::CONFLICT, Json(SqlQueryResponse::error(query, msg)));
                }
                inner.mgr.db_mut().begin_transaction();
                inner.active_txn = Some(TxnMeta {
                    owner: actor.clone(),
                    last_activity: std::sync::Mutex::new(std::time::Instant::now()),
                });
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                return status_response(query, "トランザクションを開始しました（COMMIT / ROLLBACK で終了）");
            }

            // COMMIT / ROLLBACK: 所有者チェック
            let owner = inner.active_txn.as_ref().map(|t| t.owner.clone());
            match owner {
                None => {
                    return (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, "実行中のトランザクションがありません。")));
                }
                Some(o) if o != actor => {
                    let msg = format!("このトランザクションは別のユーザー（{}）が開始したものです。", o);
                    return (StatusCode::CONFLICT, Json(SqlQueryResponse::error(query, msg)));
                }
                Some(_) => {}
            }

            if is_commit {
                // compact で一括永続化 → オフセット再同期 → トランザクション終了（追い出し再開）
                auto_save(&mut inner);
                inner.mgr.db_mut().end_transaction();
                inner.active_txn = None;
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                return status_response(query, "コミットしました");
            }

            // ROLLBACK: トランザクション中は .kdb/JSON を一切書き換えていないので、
            // ディスクから読み直すだけで開始前の状態に戻せる。
            let db_path = inner.db_path.clone();
            match reload_database(&db_path) {
                Ok(db) => {
                    inner.mgr = LabelManager::from_db(db);
                    inner.active_txn = None;
                    inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                    return status_response(query, "ロールバックしました");
                }
                Err(e) => {
                    inner.logger.db_warn(format!("rollback reload failed: {}", e));
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(SqlQueryResponse::error(query, format!("ロールバックに失敗しました: {}", e))),
                    );
                }
            }
        }
    }

    // 現時点では SELECT / INSERT / UPDATE / DELETE / EXPLAIN のみサポート
    let upper = query.to_uppercase();
    let trimmed = upper.trim_start();

    // データ操作文はログイン中ユーザーの privileges を検査する（管理者は常に通過）。
    // EXPLAIN は SELECT の実行計画を見るだけなので SELECT と同じ権限とする。
    let required_privilege = if trimmed.starts_with("SELECT") || trimmed.starts_with("EXPLAIN") {
        Some(Privilege::Select)
    } else if trimmed.starts_with("INSERT") {
        Some(Privilege::Insert)
    } else if trimmed.starts_with("UPDATE") {
        Some(Privilege::Update)
    } else if trimmed.starts_with("DELETE") {
        Some(Privilege::Delete)
    } else {
        None
    };
    if let Some(privilege) = required_privilege {
        let inner = state.read().await;
        let Some(actor) = resolve_actor_readonly(&inner, &headers) else {
            let msg = "認証が必要です。/auth/login でログインしてください。";
            inner.logger.sql_query(&query, false, 0, Some(msg));
            return (StatusCode::UNAUTHORIZED, Json(SqlQueryResponse::error(query, msg)));
        };
        if !inner.auth.user_has_privilege(&actor, privilege) {
            let msg = format!(
                "この操作には {} 権限が必要です。管理者に GRANT を依頼してください。",
                privilege.label()
            );
            inner.logger.sql_query(&query, false, 0, Some(&msg));
            return (StatusCode::FORBIDDEN, Json(SqlQueryResponse::error(query, msg)));
        }
    }

    if trimmed.starts_with("EXPLAIN") {
        let started = std::time::Instant::now();
        let inner = state.read().await;
        let db = inner.mgr.db();

        return match run_explain(db, &query) {
            Ok(result) => {
                let rows: Vec<Vec<serde_json::Value>> = result.rows.iter()
                    .map(|row| row.iter().map(cell_to_json).collect())
                    .collect();
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse::rows(query, result.columns, rows)))
            }
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e.to_string()));
                (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string())))
            }
        };
    }

    if trimmed.starts_with("SELECT") {
        let started = std::time::Instant::now();
        let inner = state.read().await;
        // 自分のトランザクション中の SELECT なら無操作タイムアウトの起点を更新する
        // （read ロックのままでも last_activity は内部可変な Mutex なので更新できる）
        if let Some(actor) = resolve_actor_readonly(&inner, &headers) {
            if inner.active_txn.as_ref().map(|t| t.owner == actor).unwrap_or(false) {
                touch_txn(&inner);
            }
        }
        let db = inner.mgr.db();

        return match run_select(db, &query) {
            Ok(result) => {
                let returned = result.rows.len();
                let rows: Vec<Vec<serde_json::Value>> = result.rows.iter()
                    .map(|row| row.iter().map(cell_to_json).collect())
                    .collect();
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse {
                    ok: true,
                    columns: result.columns,
                    rows,
                    total_matched: Some(result.total_matched),
                    returned: Some(returned),
                    inserted_id: None,
                    labels: vec![],
                    updated_count: None,
                    deleted_count: None,
                    error: None,
                    needs_confirmation: None,
                    warning: None,
                    query,
                }))
            }
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e.to_string()));
                (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string())))
            }
        };
    }

    if trimmed.starts_with("INSERT") {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        let actor = resolve_actor_readonly(&inner, &headers).unwrap_or_default();
        let in_txn = match txn_write_check(&inner, &actor, &query) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        // kdb モード: WAL 追記(insert_fast, O(1)) / JSON モード: 通常insert
        // Rustの借用チェッカー対策: kdb と mgr を別々に取り出す（handlers::create_recordと同じパターン）。
        // トランザクション中は .kdb へ書かず（kdb ハンドルを渡さない）、COMMIT 時にまとめて永続化する。
        let insert_result = {
            let mut kdb_taken = if in_txn { None } else { inner.kdb.take() };
            let result = run_insert_fast(inner.mgr.db_mut(), kdb_taken.as_mut(), &query);
            if !in_txn { inner.kdb = kdb_taken; }
            result
        };

        return match insert_result {
            Ok(result) => {
                if in_txn {
                    touch_txn(&inner);
                } else if inner.kdb.is_none() {
                    // JSON モードのみ即時保存（kdb モードは insert_fast が WAL 追記済み）
                    auto_save(&mut inner);
                }
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                // 挿入後のレコードを読み直し、使用したカラム順に値を並べて返す
                let record = inner.mgr.db().get(result.id).unwrap();
                let row: Vec<serde_json::Value> = result.columns.iter()
                    .map(|col| record.columns.get(col)
                        .map(CellValue::from_data_type)
                        .map(|c| cell_to_json(&c))
                        .unwrap_or(serde_json::Value::Null))
                    .collect();
                (StatusCode::CREATED, Json(SqlQueryResponse {
                    ok: true,
                    columns: result.columns,
                    rows: vec![row],
                    total_matched: None,
                    returned: Some(1),
                    inserted_id: Some(result.id),
                    labels: result.labels,
                    updated_count: None,
                    deleted_count: None,
                    error: None,
                    needs_confirmation: None,
                    warning: None,
                    query,
                }))
            }
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e.to_string()));
                (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string())))
            }
        };
    }

    if trimmed.starts_with("UPDATE") {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        let actor = resolve_actor_readonly(&inner, &headers).unwrap_or_default();
        let in_txn = match txn_write_check(&inner, &actor, &query) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let update_result = run_update(inner.mgr.db_mut(), &query);

        return match update_result {
            Ok(result) => {
                // UPDATE/DELETE後はコンパクションが必要（kdbモードは全件書き直し、JSONモードは通常保存）。
                // トランザクション中は遅延し、COMMIT 時にまとめて永続化する。
                if in_txn { touch_txn(&inner); } else { auto_save(&mut inner); }
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse {
                    ok: true,
                    columns: vec![],
                    rows: vec![],
                    total_matched: None,
                    returned: None,
                    inserted_id: None,
                    labels: vec![],
                    updated_count: Some(result.updated_count),
                    deleted_count: None,
                    error: None,
                    needs_confirmation: None,
                    warning: None,
                    query,
                }))
            }
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e.to_string()));
                (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string())))
            }
        };
    }

    if trimmed.starts_with("DELETE") {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
        expire_stale_transaction(&mut inner);
        let actor = resolve_actor_readonly(&inner, &headers).unwrap_or_default();
        let in_txn = match txn_write_check(&inner, &actor, &query) {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let delete_result = run_delete(inner.mgr.db_mut(), &query);

        return match delete_result {
            Ok(result) => {
                // UPDATE/DELETE後はコンパクションが必要（kdbモードは全件書き直し、JSONモードは通常保存）。
                // トランザクション中は遅延し、COMMIT 時にまとめて永続化する。
                if in_txn { touch_txn(&inner); } else { auto_save(&mut inner); }
                inner.logger.sql_query(&query, true, started.elapsed().as_millis(), None);
                (StatusCode::OK, Json(SqlQueryResponse {
                    ok: true,
                    columns: vec![],
                    rows: vec![],
                    total_matched: None,
                    returned: None,
                    inserted_id: None,
                    labels: vec![],
                    updated_count: None,
                    deleted_count: Some(result.deleted_count),
                    error: None,
                    needs_confirmation: None,
                    warning: None,
                    query,
                }))
            }
            Err(e) => {
                inner.logger.sql_query(&query, false, started.elapsed().as_millis(), Some(&e.to_string()));
                (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(query, e.to_string())))
            }
        };
    }

    {
        let inner = state.read().await;
        inner.logger.sql_query(&query, false, 0, Some("unsupported statement"));
    }
    (StatusCode::BAD_REQUEST, Json(SqlQueryResponse::error(
        query, "Only SELECT, INSERT, UPDATE, DELETE, EXPLAIN, and index/user/schema management statements are supported in this version.",
    )))
}
