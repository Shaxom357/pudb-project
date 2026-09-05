// src/app.rs
// Router の組み立てを main から分離する
// テストからも同じルーター定義を使えるようにする

use axum::{
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use dynamic_label_management::LabelManager;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::auth::{AuthState, Privilege};
use crate::auth_handlers::{bearer_token, change_password, login, logout, require_auth};
use crate::handlers::{
    create_record, delete_record, get_record,
    get_records_by_label, list_records, update_record,
    AppState, AppStateInner,
};
use crate::label_handlers::{
    add_label, list_record_labels, remove_label,
    list_all_labels, search_by_labels, rename_label,
};
use crate::info_handlers::get_db_info;
use crate::logging::Logger;
use crate::logging_handlers::get_logs;
use crate::models::ErrorResponse;
use crate::settings_handlers::{get_settings, update_settings};
use crate::sql_handlers::execute_sql;

use crate::ui::ui_handler;

/// 全リクエストに対して応答時間・ステータスコードをログへ記録するミドルウェア
/// (「Webクライアントの応答時間」ログ要件)
async fn request_logging_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    let duration_ms = started.elapsed().as_millis();
    let status = response.status().as_u16();
    state.read().await.logger.http_request(&method, &path, status, duration_ms);
    response
}

/// /records, /labels 系のREST APIを設定でオフにしている場合に弾くミドルウェア。
/// SQL経由(/sql)や /ui, /db/info, /settings はこのミドルウェアの対象外。
async fn require_http_api_enabled(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let enabled = state.read().await.http_api_enabled;
    if !enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "HTTPリクエスト(REST API)は設定で無効化されています。設定画面で有効化するか、SQL(/sql)経由で操作してください。",
            )),
        ).into_response();
    }
    next.run(req).await
}

/// REST データ操作エンドポイントに必要な権限を、HTTP メソッドとパスから判定する。
/// `None` の場合は権限チェック不要。
fn required_privilege_for(method: &Method, path: &str) -> Option<Privilege> {
    match path {
        // ラベル検索は POST だが実体は参照系
        "/labels/search" => return Some(Privilege::Select),
        // ラベルのリネームは更新系
        "/labels/rename" => return Some(Privilege::Update),
        _ => {}
    }
    match *method {
        Method::GET => Some(Privilege::Select),
        Method::POST => Some(Privilege::Insert),
        Method::PUT | Method::PATCH => Some(Privilege::Update),
        Method::DELETE => Some(Privilege::Delete),
        _ => None,
    }
}

/// /records・/labels 系 REST API に対し、ログイン中ユーザーの privileges を検査するミドルウェア。
/// 管理者ロール・`All` 権限保持者は常に通過する。権限不足は 403。
/// テスト用バイパスモード（build_app）では素通りする。
async fn require_data_privilege(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let mut inner = state.write().await;
    if inner.auth.is_test_bypass() {
        drop(inner);
        return next.run(req).await;
    }
    let actor = bearer_token(&req).and_then(|t| inner.auth.username_for_token(&t));
    let Some(actor) = actor else {
        drop(inner);
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new(
                "認証が必要です。/auth/login でログインしてください。",
            )),
        )
            .into_response();
    };
    if let Some(privilege) = required_privilege_for(&method, &path) {
        if !inner.auth.user_has_privilege(&actor, privilege) {
            let msg = format!(
                "この操作には {} 権限が必要です。管理者に GRANT を依頼してください。",
                privilege.label()
            );
            drop(inner);
            return (StatusCode::FORBIDDEN, Json(ErrorResponse::new(msg))).into_response();
        }
    }
    drop(inner);
    next.run(req).await
}

fn build_router(state: AppState) -> Router {
    // データ操作系のREST API。設定でオンオフできる。
    let gated = Router::new()
        .route("/records", get(list_records).post(create_record))
        .route("/records/label/{label}", get(get_records_by_label))
        .route("/records/{id}", get(get_record).put(update_record).delete(delete_record))
        .route("/records/{id}/labels", get(list_record_labels).post(add_label))
        .route("/records/{id}/labels/{label}", delete(remove_label))
        .route("/labels", get(list_all_labels))
        .route("/labels/search", post(search_by_labels))
        .route("/labels/rename", put(rename_label))
        // route_layer は後に追加したものが外側。REST API 無効チェックを先に、
        // 続いて privileges チェックを行う。
        .route_layer(middleware::from_fn_with_state(state.clone(), require_data_privilege))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_http_api_enabled));

    // ログイン必須のエンドポイント群（/ui のアプリ殻と /auth/login 以外の全て）。
    // Bearer トークンが無い/無効な場合は require_auth が 401 を返す。
    let protected = Router::new()
        .route("/db/info", get(get_db_info))
        .route("/sql", post(execute_sql))
        .route("/settings", get(get_settings).put(update_settings))
        .route("/logs", get(get_logs))
        .route("/auth/logout", post(logout))
        .route("/auth/password", put(change_password))
        .merge(gated)
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/ui", get(ui_handler))
        .route("/", get(|| async { axum::response::Redirect::temporary("/ui") }))
        .route("/auth/login", post(login))
        .merge(protected)
        .route_layer(middleware::from_fn_with_state(state.clone(), request_logging_middleware))
        .with_state(state)
}

/// テスト用: mgr と db_path から AppState を作って Router を返す
/// 本番の既定値(無効)とは異なり、REST APIテストを直接書けるよう有効化しておく。
/// 認証も同様の理由で AuthState::bypass_for_tests() により素通りにしてある
/// （認証自体の挙動は test_auth.rs で個別に検証する）。
pub fn build_app(mgr: LabelManager, db_path: String) -> (Router, AppState) {
    let logger = Arc::new(Logger::init(&format!("{}.log", db_path)));
    let state: AppState = Arc::new(RwLock::new(AppStateInner {
        mgr, db_path, kdb: None, http_api_enabled: true, logger,
        auth: AuthState::bypass_for_tests(),
        active_txn: None,
    }));
    let router = build_router(state.clone());
    (router, state)
}

/// 本番用: 外から組み立てた AppState を受け取って Router を返す
pub fn build_app_with_state(state: AppState) -> (Router, AppState) {
    let router = build_router(state.clone());
    (router, state)
}
