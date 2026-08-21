// src/auth.rs
// 認証機能: 管理者ユーザーのパスワード検証・セッショントークン管理・永続化
//
// 資格情報は環境変数 KAGURA_AUTH_FILE で指定したファイル（既定: auth.json）に
// { "username": ..., "password_hash": ... } の形式で保存する。
// ファイルが存在しない場合、初期管理者ユーザー kagura / 初期パスワード root を
// 自動作成する（一般的なDB製品の初期セットアップに準じる）。
//
// セッショントークンはプロセスのメモリ上でのみ管理し、再起動すると全て無効になる
// （再ログインが必要）。

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::time::{Duration, Instant};

/// デフォルトの管理者（マスター）ユーザー名
pub const DEFAULT_USERNAME: &str = "kagura";
/// デフォルトの初期パスワード（初回起動時のみ使用。運用開始後は変更を強く推奨）
pub const DEFAULT_PASSWORD: &str = "root";

/// セッショントークンの有効期限
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// ログイン失敗の許容回数（これを超えるとロックする）
const MAX_LOGIN_ATTEMPTS: u32 = 5;
/// ロック時間
const LOCKOUT_DURATION: Duration = Duration::from_secs(15 * 60);
/// 新規パスワードの最低文字数
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCredentials {
    username: String,
    password_hash: String,
}

#[derive(Debug, Clone)]
struct Session {
    created_at: Instant,
}

#[derive(Debug, Default)]
struct LoginFailures {
    count: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    Locked { retry_after_secs: u64 },
    PasswordTooShort,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::InvalidCredentials => write!(f, "ユーザー名またはパスワードが正しくありません"),
            AuthError::Locked { retry_after_secs } => write!(
                f,
                "ログイン試行回数が上限を超えたため一時的にロックされています（あと約{}秒）",
                retry_after_secs
            ),
            AuthError::PasswordTooShort => write!(
                f,
                "新しいパスワードは{}文字以上にしてください",
                MIN_PASSWORD_LEN
            ),
        }
    }
}
impl std::error::Error for AuthError {}

/// 認証状態。AppStateInner に保持し、RwLock 越しに共有する。
pub struct AuthState {
    username: String,
    password_hash: String,
    file_path: String,
    sessions: HashMap<String, Session>,
    failures: HashMap<String, LoginFailures>,
    /// テスト専用: true の場合 require_auth ミドルウェアを素通りさせる。
    /// 本番コード（main.rs）からは絶対に true にしない。
    test_bypass: bool,
}

impl AuthState {
    /// KAGURA_AUTH_FILE から資格情報を読み込む。無ければ既定の管理者ユーザーを作成して保存する。
    pub fn load_or_init(file_path: &str) -> Self {
        if let Ok(content) = fs::read_to_string(file_path) {
            if let Ok(creds) = serde_json::from_str::<PersistedCredentials>(&content) {
                return AuthState {
                    username: creds.username,
                    password_hash: creds.password_hash,
                    file_path: file_path.to_string(),
                    sessions: HashMap::new(),
                    failures: HashMap::new(),
                    test_bypass: false,
                };
            }
            eprintln!("[WARN] 認証情報ファイル '{}' の読み込みに失敗しました。初期状態で再作成します", file_path);
        }
        let password_hash = hash_password(DEFAULT_PASSWORD);
        let state = AuthState {
            username: DEFAULT_USERNAME.to_string(),
            password_hash,
            file_path: file_path.to_string(),
            sessions: HashMap::new(),
            failures: HashMap::new(),
            test_bypass: false,
        };
        state.persist();
        println!(
            "[WARN] 認証情報が見つからなかったため初期管理者ユーザーを作成しました: username='{}' password='{}' (至急パスワードを変更してください / PUT /auth/password)",
            DEFAULT_USERNAME, DEFAULT_PASSWORD
        );
        state
    }

    /// テスト専用: 認証を素通りさせる状態を作る。本番の main.rs からは使用しない。
    pub fn bypass_for_tests() -> Self {
        AuthState {
            username: DEFAULT_USERNAME.to_string(),
            password_hash: hash_password(DEFAULT_PASSWORD),
            file_path: String::new(),
            sessions: HashMap::new(),
            failures: HashMap::new(),
            test_bypass: true,
        }
    }

    pub fn is_test_bypass(&self) -> bool {
        self.test_bypass
    }

    fn persist(&self) {
        if self.file_path.is_empty() {
            return;
        }
        let creds = PersistedCredentials {
            username: self.username.clone(),
            password_hash: self.password_hash.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&creds) {
            if let Some(dir) = std::path::Path::new(&self.file_path).parent() {
                if !dir.as_os_str().is_empty() {
                    let _ = fs::create_dir_all(dir);
                }
            }
            if let Err(e) = fs::write(&self.file_path, json) {
                eprintln!("[WARN] 認証情報ファイル '{}' への書き込みに失敗しました: {}", self.file_path, e);
            }
        }
    }

    /// ユーザー名/パスワードを検証し、成功したらセッショントークンを発行する。
    pub fn login(&mut self, username: &str, password: &str) -> Result<String, AuthError> {
        if let Some(f) = self.failures.get(username) {
            if let Some(until) = f.locked_until {
                let now = Instant::now();
                if now < until {
                    return Err(AuthError::Locked { retry_after_secs: (until - now).as_secs() });
                }
            }
        }

        let ok = username == self.username && verify_password(password, &self.password_hash);
        if !ok {
            let entry = self.failures.entry(username.to_string()).or_default();
            entry.count += 1;
            if entry.count >= MAX_LOGIN_ATTEMPTS {
                entry.locked_until = Some(Instant::now() + LOCKOUT_DURATION);
                entry.count = 0;
            }
            return Err(AuthError::InvalidCredentials);
        }

        self.failures.remove(username);
        let token = generate_token();
        self.sessions.insert(token.clone(), Session { created_at: Instant::now() });
        Ok(token)
    }

    /// Bearer トークンが有効なセッションを指しているか確認する。期限切れは無効として扱う。
    pub fn verify_token(&mut self, token: &str) -> bool {
        let expired = match self.sessions.get(token) {
            Some(s) => s.created_at.elapsed() > SESSION_TTL,
            None => return false,
        };
        if expired {
            self.sessions.remove(token);
            return false;
        }
        true
    }

    pub fn logout(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    /// パスワード変更。現在のパスワードの検証に成功した場合のみ更新し、
    /// 盗まれたトークンを無効化するため既存の全セッションを破棄する。
    pub fn change_password(&mut self, current_password: &str, new_password: &str) -> Result<(), AuthError> {
        if !verify_password(current_password, &self.password_hash) {
            return Err(AuthError::InvalidCredentials);
        }
        if new_password.len() < MIN_PASSWORD_LEN {
            return Err(AuthError::PasswordTooShort);
        }
        self.password_hash = hash_password(new_password);
        self.sessions.clear();
        self.persist();
        Ok(())
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    /// 初期パスワードのままかどうか（起動時警告・設定画面での注意表示に使う）
    pub fn is_default_password(&self) -> bool {
        verify_password(DEFAULT_PASSWORD, &self.password_hash)
    }
}

fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 hashing failed")
        .to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

fn generate_token() -> String {
    use argon2::password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_success_and_token_verification() {
        let mut auth = AuthState::bypass_for_tests();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        assert!(auth.verify_token(&token));
    }

    #[test]
    fn test_login_wrong_password_fails() {
        let mut auth = AuthState::bypass_for_tests();
        assert_eq!(auth.login(DEFAULT_USERNAME, "wrong"), Err(AuthError::InvalidCredentials));
    }

    #[test]
    fn test_logout_invalidates_token() {
        let mut auth = AuthState::bypass_for_tests();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        auth.logout(&token);
        assert!(!auth.verify_token(&token));
    }

    #[test]
    fn test_lockout_after_repeated_failures() {
        let mut auth = AuthState::bypass_for_tests();
        for _ in 0..MAX_LOGIN_ATTEMPTS {
            let _ = auth.login(DEFAULT_USERNAME, "wrong");
        }
        match auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD) {
            Err(AuthError::Locked { .. }) => {}
            other => panic!("expected Locked, got {:?}", other),
        }
    }

    #[test]
    fn test_change_password_requires_current_password() {
        let mut auth = AuthState::bypass_for_tests();
        assert_eq!(
            auth.change_password("wrong", "newpassword123"),
            Err(AuthError::InvalidCredentials)
        );
    }

    #[test]
    fn test_change_password_rejects_short_password() {
        let mut auth = AuthState::bypass_for_tests();
        assert_eq!(
            auth.change_password(DEFAULT_PASSWORD, "short"),
            Err(AuthError::PasswordTooShort)
        );
    }

    #[test]
    fn test_change_password_invalidates_existing_sessions() {
        let mut auth = AuthState::bypass_for_tests();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        auth.change_password(DEFAULT_PASSWORD, "newpassword123").unwrap();
        assert!(!auth.verify_token(&token));
        let new_token = auth.login(DEFAULT_USERNAME, "newpassword123").unwrap();
        assert!(auth.verify_token(&new_token));
    }
}
