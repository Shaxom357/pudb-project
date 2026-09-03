// src/auth.rs
// 認証機能: 複数ユーザーのパスワード検証・セッショントークン管理・永続化
//
// 資格情報は環境変数 KAGURA_AUTH_FILE で指定したファイル（既定: auth.json）に
// 複数ユーザー対応フォーマット
//   { "schema_version": 2, "users": [ { "username", "password_hash", "role", "privileges", ... } ] }
// で保存する。
// ファイルが存在しない場合、初期管理者ユーザー kagura / 初期パスワード root を
// 自動作成する（一般的なDB製品の初期セットアップに準じる）。
// 旧フォーマット { "username", "password_hash" } のファイルは起動時に自動で
// 新フォーマット（role=admin の単一ユーザー）へ移行する。
//
// セッショントークンはプロセスのメモリ上でのみ管理し、再起動すると全て無効になる
// （再ログインが必要）。
//
// 一般ユーザーには privileges リスト（将来の GRANT / REVOKE 用）を持たせる。
// 現バージョンで実際に参照するのは ManageUsers（ユーザー管理系SQLの実行可否）のみ。

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
/// 自分自身のパスワード変更（PUT /auth/password）で要求する最低文字数
const MIN_PASSWORD_LEN: usize = 8;
/// 認証情報ファイルの現行スキーマバージョン
const AUTH_SCHEMA_VERSION: u32 = 2;

// ---------------------------------------------------------------------------
// ロール / 権限
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// マスター管理者。全操作が可能で削除・降格できない。
    Admin,
    /// 一般ユーザー。付与された privileges の範囲で操作できる。
    User,
}

/// きめ細かい権限。現状 enforce しているのは `ManageUsers` のみだが、
/// 将来の GRANT / REVOKE のために構造だけ先に用意しておく。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    Select,
    Insert,
    Update,
    Delete,
    ManageUsers,
    ManageSettings,
    ViewLogs,
    /// すべての権限を包含するワイルドカード（管理者に付与）
    All,
}

impl Privilege {
    /// GRANT / REVOKE・エラーメッセージ・SHOW USERS 表示で使う大文字ラベル。
    pub fn label(&self) -> &'static str {
        match self {
            Privilege::Select => "SELECT",
            Privilege::Insert => "INSERT",
            Privilege::Update => "UPDATE",
            Privilege::Delete => "DELETE",
            Privilege::ManageUsers => "MANAGE_USERS",
            Privilege::ManageSettings => "MANAGE_SETTINGS",
            Privilege::ViewLogs => "VIEW_LOGS",
            Privilege::All => "ALL",
        }
    }
}

/// 一般ユーザー作成時に既定で付与する権限（データ操作一式）。
fn default_user_privileges() -> Vec<Privilege> {
    vec![Privilege::Select, Privilege::Insert, Privilege::Update, Privilege::Delete]
}

fn role_user() -> Role {
    Role::User
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// 永続化フォーマット
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserRecord {
    username: String,
    password_hash: String,
    #[serde(default = "role_user")]
    role: Role,
    #[serde(default)]
    privileges: Vec<Privilege>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedAuth {
    schema_version: u32,
    users: Vec<UserRecord>,
}

/// 旧フォーマット（単一管理者）の判定・移行用
#[derive(Debug, Clone, Deserialize)]
struct LegacyCredentials {
    username: String,
    password_hash: String,
}

/// `SHOW USERS` 等で外部へ返すユーザー情報（パスワードハッシュを含まない）
#[derive(Debug, Clone, Serialize)]
pub struct UserInfo {
    pub username: String,
    pub role: Role,
    pub privileges: Vec<Privilege>,
    pub created_at: String,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
struct Session {
    username: String,
    created_at: Instant,
}

#[derive(Debug, Default)]
struct LoginFailures {
    count: u32,
    locked_until: Option<Instant>,
}

// ---------------------------------------------------------------------------
// エラー
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    InvalidCredentials,
    Locked { retry_after_secs: u64 },
    PasswordTooShort,
    /// ユーザー名が不正（空・空白を含む・長すぎる 等）
    InvalidUsername,
    /// パスワードが空
    EmptyPassword,
    /// 指定ユーザーが存在しない
    UserNotFound,
    /// 同名ユーザーが既に存在する
    UserAlreadyExists,
    /// 実行に必要な権限がない
    PermissionDenied,
    /// 管理者ユーザーは削除・変更できない
    CannotModifyAdmin,
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
            AuthError::InvalidUsername => write!(
                f,
                "ユーザー名が不正です（空白を含まない1〜64文字で指定してください）"
            ),
            AuthError::EmptyPassword => write!(f, "パスワードを空にはできません"),
            AuthError::UserNotFound => write!(f, "指定されたユーザーは存在しません"),
            AuthError::UserAlreadyExists => write!(f, "同名のユーザーが既に存在します"),
            AuthError::PermissionDenied => write!(f, "この操作を行う権限がありません（管理者のみ実行できます）"),
            AuthError::CannotModifyAdmin => write!(f, "管理者ユーザーは削除・変更できません"),
        }
    }
}
impl std::error::Error for AuthError {}

// ---------------------------------------------------------------------------
// パスワード強度
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStrength {
    /// 脆弱（`1234` / `aaa` / 単一文字種 / 連番 / よくある語 / 8文字未満 など）
    Weak,
    /// 許容範囲
    Ok,
}

/// パスワードの脆弱性を簡易判定する。
/// 例: `1234` `aaa` `12345678` `password` → Weak / `aA951753` → Ok
pub fn password_strength(pw: &str) -> PasswordStrength {
    let len = pw.chars().count();
    if len < MIN_PASSWORD_LEN {
        return PasswordStrength::Weak;
    }

    // よくある弱いパスワード
    const COMMON: &[&str] = &[
        "password", "passw0rd", "password1", "qwerty", "qwerty123", "admin", "administrator",
        "root", "toor", "test", "testtest", "letmein", "welcome", "iloveyou", "abc123",
        "12345678", "123456789", "1234567890", "00000000", "11111111", "aaaaaaaa",
    ];
    let lower = pw.to_lowercase();
    if COMMON.contains(&lower.as_str()) {
        return PasswordStrength::Weak;
    }

    // 全て同じ文字（aaaaaaaa / 00000000）
    let first = pw.chars().next().unwrap();
    if pw.chars().all(|c| c == first) {
        return PasswordStrength::Weak;
    }

    // 連番（12345678 / abcdefgh / 87654321）
    if is_sequential(pw) {
        return PasswordStrength::Weak;
    }

    // 文字種が1種類のみ（英小文字だけ・数字だけ 等）
    let has_lower = pw.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = pw.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = pw.chars().any(|c| c.is_ascii_digit());
    let has_symbol = pw.chars().any(|c| !c.is_ascii_alphanumeric() && !c.is_whitespace());
    let classes = [has_lower, has_upper, has_digit, has_symbol]
        .iter()
        .filter(|b| **b)
        .count();
    if classes < 2 {
        return PasswordStrength::Weak;
    }

    PasswordStrength::Ok
}

/// 文字コードが一定間隔(±1)で並んでいるか（12345 / edcba など）
fn is_sequential(s: &str) -> bool {
    let codes: Vec<i32> = s.chars().map(|c| c as i32).collect();
    if codes.len() < 3 {
        return false;
    }
    let step = codes[1] - codes[0];
    if step != 1 && step != -1 {
        return false;
    }
    codes.windows(2).all(|w| w[1] - w[0] == step)
}

// ---------------------------------------------------------------------------
// AuthState
// ---------------------------------------------------------------------------

/// 認証状態。AppStateInner に保持し、RwLock 越しに共有する。
pub struct AuthState {
    users: Vec<UserRecord>,
    file_path: String,
    sessions: HashMap<String, Session>,
    failures: HashMap<String, LoginFailures>,
    /// テスト専用: true の場合 require_auth ミドルウェアを素通りさせる。
    /// 本番コード（main.rs）からは絶対に true にしない。
    test_bypass: bool,
}

impl AuthState {
    fn from_users(users: Vec<UserRecord>, file_path: &str) -> Self {
        AuthState {
            users,
            file_path: file_path.to_string(),
            sessions: HashMap::new(),
            failures: HashMap::new(),
            test_bypass: false,
        }
    }

    /// KAGURA_AUTH_FILE から資格情報を読み込む。
    /// 新フォーマット → そのまま / 旧フォーマット → 移行して書き戻し / 無し → 既定管理者を作成。
    pub fn load_or_init(file_path: &str) -> Self {
        if let Ok(content) = fs::read_to_string(file_path) {
            if let Ok(parsed) = serde_json::from_str::<PersistedAuth>(&content) {
                if !parsed.users.is_empty() {
                    return Self::from_users(parsed.users, file_path);
                }
            }
            if let Ok(legacy) = serde_json::from_str::<LegacyCredentials>(&content) {
                let admin = UserRecord {
                    username: legacy.username,
                    password_hash: legacy.password_hash,
                    role: Role::Admin,
                    privileges: vec![Privilege::All],
                    created_at: now_rfc3339(),
                    disabled: false,
                };
                let state = Self::from_users(vec![admin], file_path);
                state.persist();
                println!(
                    "[INFO] 認証情報ファイル '{}' を複数ユーザー対応フォーマット(schema_version={})へ移行しました",
                    file_path, AUTH_SCHEMA_VERSION
                );
                return state;
            }
            eprintln!(
                "[WARN] 認証情報ファイル '{}' の読み込みに失敗しました。初期状態で再作成します",
                file_path
            );
        }

        let admin = UserRecord {
            username: DEFAULT_USERNAME.to_string(),
            password_hash: hash_password(DEFAULT_PASSWORD),
            role: Role::Admin,
            privileges: vec![Privilege::All],
            created_at: now_rfc3339(),
            disabled: false,
        };
        let state = Self::from_users(vec![admin], file_path);
        state.persist();
        println!(
            "[WARN] 認証情報が見つからなかったため初期管理者ユーザーを作成しました: username='{}' password='{}' (至急パスワードを変更してください / PUT /auth/password)",
            DEFAULT_USERNAME, DEFAULT_PASSWORD
        );
        state
    }

    /// テスト専用: 認証を素通りさせる状態を作る。本番の main.rs からは使用しない。
    pub fn bypass_for_tests() -> Self {
        let admin = UserRecord {
            username: DEFAULT_USERNAME.to_string(),
            password_hash: hash_password(DEFAULT_PASSWORD),
            role: Role::Admin,
            privileges: vec![Privilege::All],
            created_at: now_rfc3339(),
            disabled: false,
        };
        AuthState {
            users: vec![admin],
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
        let data = PersistedAuth {
            schema_version: AUTH_SCHEMA_VERSION,
            users: self.users.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            if let Some(dir) = std::path::Path::new(&self.file_path).parent() {
                if !dir.as_os_str().is_empty() {
                    let _ = fs::create_dir_all(dir);
                }
            }
            if let Err(e) = fs::write(&self.file_path, json) {
                eprintln!(
                    "[WARN] 認証情報ファイル '{}' への書き込みに失敗しました: {}",
                    self.file_path, e
                );
            }
        }
    }

    fn find(&self, username: &str) -> Option<&UserRecord> {
        self.users.iter().find(|u| u.username == username)
    }

    fn find_mut(&mut self, username: &str) -> Option<&mut UserRecord> {
        self.users.iter_mut().find(|u| u.username == username)
    }

    /// ユーザー名/パスワードを検証し、成功したらセッショントークンを発行する。
    pub fn login(&mut self, username: &str, password: &str) -> Result<String, AuthError> {
        if let Some(f) = self.failures.get(username) {
            if let Some(until) = f.locked_until {
                let now = Instant::now();
                if now < until {
                    return Err(AuthError::Locked {
                        retry_after_secs: (until - now).as_secs(),
                    });
                }
            }
        }

        let ok = self
            .find(username)
            .map(|u| !u.disabled && verify_password(password, &u.password_hash))
            .unwrap_or(false);
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
        self.sessions.insert(
            token.clone(),
            Session {
                username: username.to_string(),
                created_at: Instant::now(),
            },
        );
        Ok(token)
    }

    /// 期限切れセッションの掃除を行わずにトークン→ユーザー名を引く。
    /// read ロックしか取れない箇所（`/sql` の SELECT 権限判定など）で使う。
    pub fn peek_username_for_token(&self, token: &str) -> Option<String> {
        let s = self.sessions.get(token)?;
        if s.created_at.elapsed() > SESSION_TTL {
            return None;
        }
        Some(s.username.clone())
    }

    /// Bearer トークンが有効なら、対応するユーザー名を返す。期限切れは無効として掃除する。
    pub fn username_for_token(&mut self, token: &str) -> Option<String> {
        let expired = match self.sessions.get(token) {
            Some(s) => s.created_at.elapsed() > SESSION_TTL,
            None => return None,
        };
        if expired {
            self.sessions.remove(token);
            return None;
        }
        self.sessions.get(token).map(|s| s.username.clone())
    }

    /// Bearer トークンが有効なセッションを指しているか確認する。
    pub fn verify_token(&mut self, token: &str) -> bool {
        self.username_for_token(token).is_some()
    }

    pub fn logout(&mut self, token: &str) {
        self.sessions.remove(token);
    }

    /// ログイン中ユーザー（actor）自身のパスワード変更。
    /// 現在のパスワードの検証に成功した場合のみ更新し、
    /// 盗まれたトークンを無効化するため既存の全セッションを破棄する。
    pub fn change_password(
        &mut self,
        actor: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AuthError> {
        let current_hash = self
            .find(actor)
            .map(|u| u.password_hash.clone())
            .ok_or(AuthError::UserNotFound)?;
        if !verify_password(current_password, &current_hash) {
            return Err(AuthError::InvalidCredentials);
        }
        if new_password.len() < MIN_PASSWORD_LEN {
            return Err(AuthError::PasswordTooShort);
        }
        let hash = hash_password(new_password);
        self.find_mut(actor).unwrap().password_hash = hash;
        self.sessions.clear();
        self.persist();
        Ok(())
    }

    // ---- ユーザー管理（CREATE USER / DROP USER / ALTER USER / SHOW USERS） ----

    /// actor がユーザー管理系操作を実行できるか。
    pub fn can_manage_users(&self, actor: &str) -> bool {
        self.user_has_privilege(actor, Privilege::ManageUsers)
    }

    /// actor が指定 privilege を持つか（Admin ロールと `All` は常に true）。
    pub fn user_has_privilege(&self, username: &str, privilege: Privilege) -> bool {
        self.find(username)
            .map(|u| {
                !u.disabled
                    && (matches!(u.role, Role::Admin)
                        || u.privileges.contains(&Privilege::All)
                        || u.privileges.contains(&privilege))
            })
            .unwrap_or(false)
    }

    /// 一般ユーザーを新規作成する。
    pub fn create_user(
        &mut self,
        username: &str,
        password: &str,
        if_not_exists: bool,
    ) -> Result<(), AuthError> {
        validate_username(username)?;
        if password.is_empty() {
            return Err(AuthError::EmptyPassword);
        }
        if self.find(username).is_some() {
            return if if_not_exists {
                Ok(())
            } else {
                Err(AuthError::UserAlreadyExists)
            };
        }
        self.users.push(UserRecord {
            username: username.to_string(),
            password_hash: hash_password(password),
            role: Role::User,
            privileges: default_user_privileges(),
            created_at: now_rfc3339(),
            disabled: false,
        });
        self.persist();
        Ok(())
    }

    /// ユーザーを削除する。管理者ユーザーは削除できない。
    pub fn drop_user(&mut self, username: &str, if_exists: bool) -> Result<(), AuthError> {
        let Some(idx) = self.users.iter().position(|u| u.username == username) else {
            return if if_exists {
                Ok(())
            } else {
                Err(AuthError::UserNotFound)
            };
        };
        if matches!(self.users[idx].role, Role::Admin) {
            return Err(AuthError::CannotModifyAdmin);
        }
        self.users.remove(idx);
        self.sessions.retain(|_, s| s.username != username);
        self.failures.remove(username);
        self.persist();
        Ok(())
    }

    /// 既存ユーザーのパスワードを（管理者権限で）リセットする。
    pub fn set_user_password(&mut self, username: &str, password: &str) -> Result<(), AuthError> {
        if password.is_empty() {
            return Err(AuthError::EmptyPassword);
        }
        let hash = hash_password(password);
        let Some(user) = self.find_mut(username) else {
            return Err(AuthError::UserNotFound);
        };
        user.password_hash = hash;
        self.sessions.retain(|_, s| s.username != username);
        self.persist();
        Ok(())
    }

    // ---- 権限付与 / 剥奪（GRANT / REVOKE） ----

    /// GRANT / REVOKE の対象にできるユーザーかを事前検証する。
    /// 管理者ユーザーは常に全権限を持つため対象外（`CannotModifyAdmin`）。
    pub fn ensure_grantable(&self, username: &str) -> Result<(), AuthError> {
        match self.find(username) {
            None => Err(AuthError::UserNotFound),
            Some(u) if matches!(u.role, Role::Admin) => Err(AuthError::CannotModifyAdmin),
            Some(_) => Ok(()),
        }
    }

    /// 指定ユーザーへ権限を付与する（既に持っている権限は無視）。
    pub fn grant_privileges(&mut self, username: &str, privs: &[Privilege]) -> Result<(), AuthError> {
        self.ensure_grantable(username)?;
        let user = self.find_mut(username).ok_or(AuthError::UserNotFound)?;
        for p in privs {
            if !user.privileges.contains(p) {
                user.privileges.push(*p);
            }
        }
        self.persist();
        Ok(())
    }

    /// 指定ユーザーから権限を剥奪する（持っていない権限は無視）。
    pub fn revoke_privileges(&mut self, username: &str, privs: &[Privilege]) -> Result<(), AuthError> {
        self.ensure_grantable(username)?;
        let user = self.find_mut(username).ok_or(AuthError::UserNotFound)?;
        user.privileges.retain(|p| !privs.contains(p));
        self.persist();
        Ok(())
    }

    /// 登録ユーザー一覧（パスワードハッシュを除く）
    pub fn list_users(&self) -> Vec<UserInfo> {
        self.users
            .iter()
            .map(|u| UserInfo {
                username: u.username.clone(),
                role: u.role,
                privileges: u.privileges.clone(),
                created_at: u.created_at.clone(),
                disabled: u.disabled,
            })
            .collect()
    }

    /// 管理者（マスター）ユーザー名。起動時警告・設定画面の注意表示に使う。
    pub fn username(&self) -> &str {
        self.users
            .iter()
            .find(|u| matches!(u.role, Role::Admin))
            .map(|u| u.username.as_str())
            .unwrap_or(DEFAULT_USERNAME)
    }

    /// 管理者ユーザーが初期パスワードのままかどうか
    pub fn is_default_password(&self) -> bool {
        self.users
            .iter()
            .find(|u| matches!(u.role, Role::Admin))
            .map(|u| verify_password(DEFAULT_PASSWORD, &u.password_hash))
            .unwrap_or(false)
    }
}

fn validate_username(username: &str) -> Result<(), AuthError> {
    let len = username.chars().count();
    if len == 0 || len > 64 || username.chars().any(|c| c.is_whitespace()) {
        return Err(AuthError::InvalidUsername);
    }
    Ok(())
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
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
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

    fn admin_auth() -> AuthState {
        AuthState::bypass_for_tests()
    }

    #[test]
    fn test_login_success_and_token_verification() {
        let mut auth = admin_auth();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        assert!(auth.verify_token(&token));
        assert_eq!(auth.username_for_token(&token).as_deref(), Some(DEFAULT_USERNAME));
    }

    #[test]
    fn test_login_wrong_password_fails() {
        let mut auth = admin_auth();
        assert_eq!(
            auth.login(DEFAULT_USERNAME, "wrong"),
            Err(AuthError::InvalidCredentials)
        );
    }

    #[test]
    fn test_logout_invalidates_token() {
        let mut auth = admin_auth();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        auth.logout(&token);
        assert!(!auth.verify_token(&token));
    }

    #[test]
    fn test_lockout_after_repeated_failures() {
        let mut auth = admin_auth();
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
        let mut auth = admin_auth();
        assert_eq!(
            auth.change_password(DEFAULT_USERNAME, "wrong", "newpassword123"),
            Err(AuthError::InvalidCredentials)
        );
    }

    #[test]
    fn test_change_password_rejects_short_password() {
        let mut auth = admin_auth();
        assert_eq!(
            auth.change_password(DEFAULT_USERNAME, DEFAULT_PASSWORD, "short"),
            Err(AuthError::PasswordTooShort)
        );
    }

    #[test]
    fn test_change_password_invalidates_existing_sessions() {
        let mut auth = admin_auth();
        let token = auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).unwrap();
        auth.change_password(DEFAULT_USERNAME, DEFAULT_PASSWORD, "newpassword123")
            .unwrap();
        assert!(!auth.verify_token(&token));
        let new_token = auth.login(DEFAULT_USERNAME, "newpassword123").unwrap();
        assert!(auth.verify_token(&new_token));
    }

    // ---- 複数ユーザー ----

    #[test]
    fn test_create_user_and_login() {
        let mut auth = admin_auth();
        auth.create_user("alice", "aA951753", false).unwrap();
        let token = auth.login("alice", "aA951753").unwrap();
        assert_eq!(auth.username_for_token(&token).as_deref(), Some("alice"));
    }

    #[test]
    fn test_create_user_duplicate() {
        let mut auth = admin_auth();
        auth.create_user("bob", "aA951753", false).unwrap();
        assert_eq!(
            auth.create_user("bob", "zZ864209", false),
            Err(AuthError::UserAlreadyExists)
        );
        // IF NOT EXISTS なら黙って成功
        assert_eq!(auth.create_user("bob", "zZ864209", true), Ok(()));
    }

    #[test]
    fn test_create_user_rejects_empty_password_and_bad_name() {
        let mut auth = admin_auth();
        assert_eq!(
            auth.create_user("carol", "", false),
            Err(AuthError::EmptyPassword)
        );
        assert_eq!(
            auth.create_user("has space", "aA951753", false),
            Err(AuthError::InvalidUsername)
        );
    }

    #[test]
    fn test_general_user_cannot_manage_users() {
        let mut auth = admin_auth();
        auth.create_user("dave", "aA951753", false).unwrap();
        assert!(auth.can_manage_users(DEFAULT_USERNAME));
        assert!(!auth.can_manage_users("dave"));
        assert!(auth.user_has_privilege("dave", Privilege::Select));
        assert!(!auth.user_has_privilege("dave", Privilege::ManageUsers));
    }

    #[test]
    fn test_drop_user() {
        let mut auth = admin_auth();
        auth.create_user("erin", "aA951753", false).unwrap();
        auth.drop_user("erin", false).unwrap();
        assert_eq!(auth.login("erin", "aA951753"), Err(AuthError::InvalidCredentials));
        assert_eq!(auth.drop_user("erin", false), Err(AuthError::UserNotFound));
        assert_eq!(auth.drop_user("erin", true), Ok(()));
    }

    #[test]
    fn test_cannot_drop_admin() {
        let mut auth = admin_auth();
        assert_eq!(
            auth.drop_user(DEFAULT_USERNAME, false),
            Err(AuthError::CannotModifyAdmin)
        );
    }

    #[test]
    fn test_alter_user_password() {
        let mut auth = admin_auth();
        auth.create_user("frank", "aA951753", false).unwrap();
        auth.set_user_password("frank", "Xy837261").unwrap();
        assert_eq!(
            auth.login("frank", "aA951753"),
            Err(AuthError::InvalidCredentials)
        );
        assert!(auth.login("frank", "Xy837261").is_ok());
    }

    #[test]
    fn test_list_users() {
        let mut auth = admin_auth();
        auth.create_user("grace", "aA951753", false).unwrap();
        let users = auth.list_users();
        assert_eq!(users.len(), 2);
        assert!(users.iter().any(|u| u.username == DEFAULT_USERNAME && matches!(u.role, Role::Admin)));
        assert!(users.iter().any(|u| u.username == "grace" && matches!(u.role, Role::User)));
    }

    // ---- 権限付与 / 剥奪 ----

    #[test]
    fn test_grant_and_revoke_privileges() {
        let mut auth = admin_auth();
        auth.create_user("ivan", "aA951753", false).unwrap();
        // 既定はデータ操作4種
        assert!(auth.user_has_privilege("ivan", Privilege::Select));
        assert!(!auth.user_has_privilege("ivan", Privilege::ManageUsers));

        // 剥奪
        auth.revoke_privileges("ivan", &[Privilege::Delete, Privilege::Update])
            .unwrap();
        assert!(!auth.user_has_privilege("ivan", Privilege::Delete));
        assert!(!auth.user_has_privilege("ivan", Privilege::Update));
        assert!(auth.user_has_privilege("ivan", Privilege::Select));

        // 付与（重複は無視）
        auth.grant_privileges("ivan", &[Privilege::Delete, Privilege::ManageUsers, Privilege::Select])
            .unwrap();
        assert!(auth.user_has_privilege("ivan", Privilege::Delete));
        assert!(auth.user_has_privilege("ivan", Privilege::ManageUsers));
        assert_eq!(
            auth.list_users()
                .iter()
                .find(|u| u.username == "ivan")
                .unwrap()
                .privileges
                .iter()
                .filter(|p| **p == Privilege::Select)
                .count(),
            1,
            "同じ権限が重複して格納されない"
        );
    }

    #[test]
    fn test_grant_rejects_admin_and_missing_user() {
        let mut auth = admin_auth();
        assert_eq!(
            auth.grant_privileges(DEFAULT_USERNAME, &[Privilege::Select]),
            Err(AuthError::CannotModifyAdmin)
        );
        assert_eq!(
            auth.grant_privileges("nobody", &[Privilege::Select]),
            Err(AuthError::UserNotFound)
        );
    }

    // ---- パスワード強度 ----

    #[test]
    fn test_password_strength() {
        for weak in ["1234", "aaa", "12345678", "aaaaaaaa", "password", "abcdefgh", "87654321", "abcdefg"] {
            assert_eq!(password_strength(weak), PasswordStrength::Weak, "{weak} should be weak");
        }
        for ok in ["aA951753", "Str0ngPass", "x9K2m4Qz", "pa$$W0rd!"] {
            assert_eq!(password_strength(ok), PasswordStrength::Ok, "{ok} should be ok");
        }
    }

    // ---- 永続化・移行 ----

    #[test]
    fn test_legacy_auth_file_is_migrated() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "auth_migrate_test_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let legacy = format!(
            "{{\"username\":\"{}\",\"password_hash\":{:?}}}",
            DEFAULT_USERNAME,
            hash_password(DEFAULT_PASSWORD)
        );
        fs::write(&path, legacy).unwrap();

        let mut auth = AuthState::load_or_init(&path.to_string_lossy());
        assert!(auth.login(DEFAULT_USERNAME, DEFAULT_PASSWORD).is_ok());

        // ディスク上が新フォーマットへ書き換わっている
        let content = fs::read_to_string(&path).unwrap();
        let parsed: PersistedAuth = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.schema_version, AUTH_SCHEMA_VERSION);
        assert_eq!(parsed.users.len(), 1);
        assert!(matches!(parsed.users[0].role, Role::Admin));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_new_format_roundtrip_persists_created_user() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "auth_roundtrip_test_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_str = path.to_string_lossy().to_string();
        {
            let mut auth = AuthState::load_or_init(&path_str);
            auth.create_user("heidi", "aA951753", false).unwrap();
        }
        // 別インスタンスで読み直しても heidi が残っている
        let mut auth2 = AuthState::load_or_init(&path_str);
        assert!(auth2.login("heidi", "aA951753").is_ok());

        let _ = fs::remove_file(&path);
    }
}
