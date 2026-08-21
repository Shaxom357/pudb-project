// src/client.rs
// KAGURA DB サーバー(db_client)の REST API を叩く薄いクライアント。
// エンドポイントの形状は db_client/src/auth_handlers.rs, info_handlers.rs, models.rs に準拠する。

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    #[allow(dead_code)]
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Deserialize)]
pub struct DbInfoResponse {
    pub engine_name: String,
    pub app_version: String,
    pub storage_mode: String,
}

pub struct ApiError(pub String);

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct Client {
    http: reqwest::Client,
    /// スキーム込み・末尾スラッシュ無しのベースURL（例: http://127.0.0.1:3000）
    base_url: String,
}

impl Client {
    pub fn new(base_url: String, insecure: bool) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(insecure)
            .build()
            .map_err(|e| ApiError(format!("HTTPクライアントの初期化に失敗しました: {}", e)))?;
        Ok(Client { http, base_url })
    }

    async fn parse_error(resp: reqwest::Response) -> ApiError {
        let status = resp.status();
        match resp.json::<ErrorResponse>().await {
            Ok(body) => ApiError(format!("{} ({})", body.error, status)),
            Err(_) => ApiError(format!("サーバーエラー ({})", status)),
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<LoginResponse, ApiError> {
        let url = format!("{}/auth/login", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&LoginRequest { username, password })
            .send()
            .await
            .map_err(|e| ApiError(format!("サーバー '{}' に接続できませんでした: {}", self.base_url, e)))?;

        if resp.status().is_success() {
            resp.json::<LoginResponse>()
                .await
                .map_err(|e| ApiError(format!("応答の解析に失敗しました: {}", e)))
        } else {
            Err(Self::parse_error(resp).await)
        }
    }

    pub async fn logout(&self, token: &str) -> Result<(), ApiError> {
        let url = format!("{}/auth/logout", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ApiError(format!("サーバー '{}' に接続できませんでした: {}", self.base_url, e)))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Self::parse_error(resp).await)
        }
    }

    pub async fn db_info(&self, token: &str) -> Result<DbInfoResponse, ApiError> {
        let url = format!("{}/db/info", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ApiError(format!("サーバー '{}' に接続できませんでした: {}", self.base_url, e)))?;

        if resp.status().is_success() {
            resp.json::<DbInfoResponse>()
                .await
                .map_err(|e| ApiError(format!("応答の解析に失敗しました: {}", e)))
        } else {
            Err(Self::parse_error(resp).await)
        }
    }
}

/// -a/--address で受け取った接続先文字列を正規化する。
/// スキームが無い場合（例: "127.0.0.1:3000"）は http:// を補完する。
/// 末尾のスラッシュは取り除く。
pub fn normalize_address(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    }
}
