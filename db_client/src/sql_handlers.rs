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
use crate::handlers::{auto_save, AppState};
use crate::user_sql::{execute_user_statement, parse_user_statement, UserSqlOutcome};
use sql_engine::{run_select, run_insert_fast, run_update, run_delete, CellValue};

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

    // ユーザー管理系（CREATE USER / DROP USER / ALTER USER / SHOW USERS）は
    // レコードストアではなく認証状態(AuthState)を操作する。SELECT/INSERT/... の
    // ディスパッチより前に処理し、該当しなければ None が返って通常処理へ進む。
    if let Some(parsed) = parse_user_statement(&query) {
        let started = std::time::Instant::now();
        let mut inner = state.write().await;
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

    // 現時点では SELECT / INSERT / UPDATE / DELETE のみサポート
    let upper = query.to_uppercase();
    let trimmed = upper.trim_start();

    // データ操作文はログイン中ユーザーの privileges を検査する（管理者は常に通過）。
    let required_privilege = if trimmed.starts_with("SELECT") {
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

    if trimmed.starts_with("SELECT") {
        let started = std::time::Instant::now();
        let inner = state.read().await;
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
        // kdb モード: WAL 追記(insert_fast, O(1)) / JSON モード: 通常insert
        // Rustの借用チェッカー対策: kdb と mgr を別々に取り出す（handlers::create_recordと同じパターン）
        let insert_result = {
            let mut kdb_taken = inner.kdb.take();
            let result = run_insert_fast(inner.mgr.db_mut(), kdb_taken.as_mut(), &query);
            inner.kdb = kdb_taken;
            result
        };

        return match insert_result {
            Ok(result) => {
                // kdb モードは insert_fast が WAL 追記済みなのでコンパクション不要。JSON モードのみ保存する。
                if inner.kdb.is_none() { auto_save(&mut inner); }
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
        let update_result = run_update(inner.mgr.db_mut(), &query);

        return match update_result {
            Ok(result) => {
                // UPDATE/DELETE後はコンパクションが必要（kdbモードは全件書き直し、JSONモードは通常保存）
                auto_save(&mut inner);
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
        let delete_result = run_delete(inner.mgr.db_mut(), &query);

        return match delete_result {
            Ok(result) => {
                // UPDATE/DELETE後はコンパクションが必要（kdbモードは全件書き直し、JSONモードは通常保存）
                auto_save(&mut inner);
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
        query, "Only SELECT, INSERT, UPDATE, and DELETE statements are supported in this version.",
    )))
}
