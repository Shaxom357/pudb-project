// src/auth_handlers.rs
// 認証 API ハンドラー（ログイン/ログアウト/パスワード変更）と、
// Bearer トークンを検証する require_auth ミドルウェア

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::auth::DEFAULT_USERNAME;
use serde::{Deserialize, Serialize};

use crate::auth::AuthError;
use crate::handlers::AppState;
use crate::models::ErrorResponse;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

fn auth_error_response(e: AuthError) -> Response {
    let status = match e {
        AuthError::InvalidCredentials => StatusCode::UNAUTHORIZED,
        AuthError::Locked { .. } => StatusCode::LOCKED,
        AuthError::PasswordTooShort
        | AuthError::InvalidUsername
        | AuthError::EmptyPassword => StatusCode::BAD_REQUEST,
        AuthError::UserNotFound => StatusCode::NOT_FOUND,
        AuthError::UserAlreadyExists => StatusCode::CONFLICT,
        AuthError::PermissionDenied | AuthError::CannotModifyAdmin => StatusCode::FORBIDDEN,
    };
    (status, Json(ErrorResponse::new(e.to_string()))).into_response()
}

/// Authorization: Bearer <token> ヘッダーからトークンを取り出す。
pub fn bearer_token(req: &Request) -> Option<String> {
    bearer_from_headers(req.headers())
}

/// HeaderMap から Bearer トークンを取り出す（Json 抽出と併用するハンドラー向け）。
pub fn bearer_from_headers(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    header.strip_prefix("Bearer ").map(|t| t.to_string())
}

/// リクエストの Bearer トークンから操作主体（ログイン中ユーザー名）を解決する。
/// テスト用のバイパスモードでは常に管理者ユーザーとして扱う。
pub fn resolve_actor(inner: &mut crate::handlers::AppStateInner, headers: &HeaderMap) -> Option<String> {
    if inner.auth.is_test_bypass() {
        return Some(DEFAULT_USERNAME.to_string());
    }
    let token = bearer_from_headers(headers)?;
    inner.auth.username_for_token(&token)
}

/// POST /auth/login -- ユーザー名/パスワードを検証し、成功したらセッショントークンを発行する
pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    match inner.auth.login(&payload.username, &payload.password) {
        Ok(token) => {
            inner.logger.auth_info(format!("login success user='{}'", payload.username));
            (StatusCode::OK, Json(LoginResponse { token, username: payload.username })).into_response()
        }
        Err(e) => {
            inner.logger.auth_warn(format!("login failed user='{}': {}", payload.username, e));
            auth_error_response(e)
        }
    }
}

/// POST /auth/logout -- 現在のセッショントークンを無効化する（require_auth の後段で呼ばれる）
pub async fn logout(State(state): State<AppState>, req: Request) -> impl IntoResponse {
    let token = bearer_token(&req);
    let mut inner = state.write().await;
    if let Some(token) = token {
        inner.auth.logout(&token);
    }
    inner.logger.auth_info("logout");
    StatusCode::NO_CONTENT
}

/// PUT /auth/password -- 現在のパスワードを検証し、新しいパスワードへ変更する。
/// 成功すると他の全セッション（このリクエストのトークンを含む）が無効化されるため、
/// クライアントは再ログインが必要になる。
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChangePasswordRequest>,
) -> impl IntoResponse {
    let mut inner = state.write().await;
    let Some(actor) = resolve_actor(&mut inner, &headers) else {
        return auth_error_response(AuthError::InvalidCredentials);
    };
    match inner
        .auth
        .change_password(&actor, &payload.current_password, &payload.new_password)
    {
        Ok(()) => {
            inner.logger.auth_info("password changed");
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            inner.logger.auth_warn(format!("password change failed: {}", e));
            auth_error_response(e)
        }
    }
}

/// 保護対象ルートに適用するミドルウェア。Authorization: Bearer <token> を検証し、
/// 無効な場合は 401 を返す。テスト用の build_app() では AuthState::bypass_for_tests() により素通りする。
pub async fn require_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mut inner = state.write().await;
    if inner.auth.is_test_bypass() {
        drop(inner);
        return next.run(req).await;
    }
    let token = bearer_token(&req);
    let valid = match &token {
        Some(t) => inner.auth.verify_token(t),
        None => false,
    };
    drop(inner);
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse::new("認証が必要です。/auth/login でログインしてください。")),
        ).into_response();
    }
    next.run(req).await
}
