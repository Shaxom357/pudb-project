// src/user_sql.rs
// ユーザー管理系 SQL 文（CREATE USER / DROP USER / ALTER USER / SHOW USERS）の
// 専用パーサーと実行ロジック。
//
// これらの文はレコードストア（sql_engine / db_engine）ではなく認証状態（AuthState）を
// 操作するため、sql_engine には手を入れず db_client 側で処理する。
// `/sql` ハンドラーの冒頭で parse_user_statement() を呼び、Some が返ったら
// 通常の SELECT/INSERT/... ディスパッチには回さずここで完結させる。

use crate::auth::{password_strength, AuthError, AuthState, PasswordStrength, Role};

/// 脆弱パスワード時にクライアントへ提示する確認メッセージ（要件で文言が固定）
pub const WEAK_PASSWORD_WARNING: &str = "Warning: The password strength is too weak. Do you want to proceed?\n\nPlease enter 'yes' to proceed or 'no' to cancel.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserStatement {
    CreateUser {
        name: String,
        password: String,
        if_not_exists: bool,
    },
    DropUser {
        name: String,
        if_exists: bool,
    },
    AlterUserPassword {
        name: String,
        password: String,
    },
    ShowUsers,
}

/// execute_user_statement の結果
pub enum UserSqlOutcome {
    /// 表形式の結果（SHOW USERS）
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    /// 成功したが返す行はない（CREATE / DROP / ALTER）
    Ok { message: String },
    /// 脆弱パスワードにつき yes/no 確認が必要
    NeedsConfirmation { warning: String },
    /// 失敗。status は HTTP ステータスコード（400 / 403 / 404 / 409）。
    Err { status: u16, message: String },
}

// ---------------------------------------------------------------------------
// トークナイザ
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// キーワード / 素の識別子（大文字化して保持）
    Word(String),
    /// クォート文字列（'..' `..` ".." — 値をそのまま保持）
    Quoted(String),
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ';' {
            // 末尾のセミコロンは無視。以降に非空白があればエラー。
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() {
                return Err("複数の文をまとめて実行することはできません".to_string());
            }
            break;
        }
        if c == '\'' || c == '"' || c == '`' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err("クォートが閉じられていません".to_string());
                }
                let ch = chars[i];
                if ch == quote {
                    // '' / "" / `` は1文字分のエスケープ
                    if i + 1 < chars.len() && chars[i + 1] == quote {
                        s.push(quote);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                s.push(ch);
                i += 1;
            }
            tokens.push(Tok::Quoted(s));
            continue;
        }
        // 素の語（英数字・_・-・.）
        if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-' || chars[i] == '.')
            {
                i += 1;
            }
            let w: String = chars[start..i].iter().collect();
            tokens.push(Tok::Word(w.to_uppercase()));
            continue;
        }
        return Err(format!("解釈できない文字です: '{}'", c));
    }
    Ok(tokens)
}

/// Word なら大文字化済み文字列、Quoted なら生値を返す（識別子/パスワードの取り出し用）
fn tok_value(t: &Tok) -> String {
    match t {
        Tok::Word(w) => w.clone(),
        Tok::Quoted(s) => s.clone(),
    }
}

fn is_word(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Word(w) if w == kw)
}

// ---------------------------------------------------------------------------
// パーサー
// ---------------------------------------------------------------------------

/// ユーザー管理系の文であれば Some（パース結果 or パースエラー）を返す。
/// それ以外（SELECT/INSERT/...）は None を返し、呼び出し側の通常処理に委ねる。
pub fn parse_user_statement(sql: &str) -> Option<Result<UserStatement, String>> {
    let trimmed = sql.trim_start();
    let head: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    let head = head.split_whitespace().collect::<Vec<_>>();

    let is_user_stmt = matches!(
        head.as_slice(),
        ["CREATE", "USER", ..]
            | ["DROP", "USER", ..]
            | ["ALTER", "USER", ..]
            | ["SHOW", "USERS", ..]
            | ["SHOW", "USER", ..]
    );
    if !is_user_stmt {
        return None;
    }
    Some(parse_inner(sql))
}

fn parse_inner(sql: &str) -> Result<UserStatement, String> {
    let toks = tokenize(sql)?;
    if toks.is_empty() {
        return Err("空の文です".to_string());
    }

    if is_word(&toks[0], "SHOW") {
        // SHOW USERS
        if toks.len() == 2 && (is_word(&toks[1], "USERS") || is_word(&toks[1], "USER")) {
            return Ok(UserStatement::ShowUsers);
        }
        return Err("構文エラー: 'SHOW USERS' と記述してください".to_string());
    }

    if is_word(&toks[0], "CREATE") {
        // CREATE USER [IF NOT EXISTS] <name> IDENTIFIED BY <password>
        if toks.len() < 2 || !is_word(&toks[1], "USER") {
            return Err("構文エラー: 'CREATE USER ...' と記述してください".to_string());
        }
        let mut idx = 2;
        let mut if_not_exists = false;
        if toks.len() >= 5
            && is_word(&toks[2], "IF")
            && is_word(&toks[3], "NOT")
            && is_word(&toks[4], "EXISTS")
        {
            if_not_exists = true;
            idx = 5;
        }
        let name = take_identifier(&toks, &mut idx)?;
        expect_identified_by(&toks, &mut idx)?;
        let password = take_password(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(UserStatement::CreateUser {
            name,
            password,
            if_not_exists,
        });
    }

    if is_word(&toks[0], "ALTER") {
        // ALTER USER <name> IDENTIFIED BY <password>
        if toks.len() < 2 || !is_word(&toks[1], "USER") {
            return Err("構文エラー: 'ALTER USER ...' と記述してください".to_string());
        }
        let mut idx = 2;
        let name = take_identifier(&toks, &mut idx)?;
        expect_identified_by(&toks, &mut idx)?;
        let password = take_password(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(UserStatement::AlterUserPassword { name, password });
    }

    if is_word(&toks[0], "DROP") {
        // DROP USER [IF EXISTS] <name>
        if toks.len() < 2 || !is_word(&toks[1], "USER") {
            return Err("構文エラー: 'DROP USER ...' と記述してください".to_string());
        }
        let mut idx = 2;
        let mut if_exists = false;
        if toks.len() >= 4 && is_word(&toks[2], "IF") && is_word(&toks[3], "EXISTS") {
            if_exists = true;
            idx = 4;
        }
        let name = take_identifier(&toks, &mut idx)?;
        expect_end(&toks, idx)?;
        return Ok(UserStatement::DropUser { name, if_exists });
    }

    Err("サポートされていないユーザー管理文です".to_string())
}

fn take_identifier(toks: &[Tok], idx: &mut usize) -> Result<String, String> {
    let t = toks
        .get(*idx)
        .ok_or_else(|| "ユーザー名が指定されていません".to_string())?;
    // 予約語を素の語で使うのは避ける（クォートは可）
    if let Tok::Word(w) = t {
        if matches!(w.as_str(), "IDENTIFIED" | "BY" | "IF" | "NOT" | "EXISTS" | "USER") {
            return Err(format!("ユーザー名が指定されていません（'{}' の前）", w));
        }
    }
    *idx += 1;
    Ok(tok_value(t))
}

fn expect_identified_by(toks: &[Tok], idx: &mut usize) -> Result<(), String> {
    let ident = toks.get(*idx);
    let by = toks.get(*idx + 1);
    match (ident, by) {
        (Some(a), Some(b)) if is_word(a, "IDENTIFIED") && is_word(b, "BY") => {
            *idx += 2;
            Ok(())
        }
        _ => Err("構文エラー: ユーザー名の後に 'IDENTIFIED BY <パスワード>' が必要です".to_string()),
    }
}

fn take_password(toks: &[Tok], idx: &mut usize) -> Result<String, String> {
    let t = toks
        .get(*idx)
        .ok_or_else(|| "パスワードが指定されていません".to_string())?;
    *idx += 1;
    Ok(tok_value(t))
}

fn expect_end(toks: &[Tok], idx: usize) -> Result<(), String> {
    if idx == toks.len() {
        Ok(())
    } else {
        Err(format!(
            "構文エラー: 余分なトークンがあります（'{}'）",
            tok_value(&toks[idx])
        ))
    }
}

// ---------------------------------------------------------------------------
// 実行
// ---------------------------------------------------------------------------

fn map_err(e: AuthError) -> UserSqlOutcome {
    let status = match e {
        AuthError::UserNotFound => 404,
        AuthError::UserAlreadyExists => 409,
        AuthError::PermissionDenied | AuthError::CannotModifyAdmin => 403,
        _ => 400,
    };
    UserSqlOutcome::Err {
        status,
        message: e.to_string(),
    }
}

fn err_400(message: impl Into<String>) -> UserSqlOutcome {
    UserSqlOutcome::Err {
        status: 400,
        message: message.into(),
    }
}

/// ユーザー管理文を実行する。
/// - actor は `/sql` を叩いているログイン中ユーザー名（権限判定に使う）
/// - confirm_weak: 脆弱パスワード確認への回答（None=未回答, Some(true)=続行, Some(false)=中止）
pub fn execute_user_statement(
    auth: &mut AuthState,
    actor: &str,
    stmt: UserStatement,
    confirm_weak: Option<bool>,
) -> UserSqlOutcome {
    if !auth.can_manage_users(actor) {
        return map_err(AuthError::PermissionDenied);
    }

    match stmt {
        UserStatement::ShowUsers => {
            let columns = vec![
                "username".to_string(),
                "role".to_string(),
                "privileges".to_string(),
                "created_at".to_string(),
                "disabled".to_string(),
            ];
            let rows = auth
                .list_users()
                .into_iter()
                .map(|u| {
                    let role = match u.role {
                        Role::Admin => "admin",
                        Role::User => "user",
                    };
                    let privs = u
                        .privileges
                        .iter()
                        .map(|p| {
                            serde_json::to_value(p)
                                .ok()
                                .and_then(|v| v.as_str().map(str::to_string))
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    vec![
                        serde_json::Value::String(u.username),
                        serde_json::Value::String(role.to_string()),
                        serde_json::Value::String(privs),
                        serde_json::Value::String(u.created_at),
                        serde_json::Value::Bool(u.disabled),
                    ]
                })
                .collect();
            UserSqlOutcome::Rows { columns, rows }
        }

        UserStatement::CreateUser {
            name,
            password,
            if_not_exists,
        } => {
            if password.is_empty() {
                return map_err(AuthError::EmptyPassword);
            }
            if password_strength(&password) == PasswordStrength::Weak {
                match confirm_weak {
                    None => {
                        return UserSqlOutcome::NeedsConfirmation {
                            warning: WEAK_PASSWORD_WARNING.to_string(),
                        }
                    }
                    Some(false) => {
                        return err_400("ユーザーの作成を中止しました（パスワードが脆弱です）")
                    }
                    Some(true) => {}
                }
            }
            match auth.create_user(&name, &password, if_not_exists) {
                Ok(()) => UserSqlOutcome::Ok {
                    message: format!("ユーザー '{}' を作成しました", name),
                },
                Err(e) => map_err(e),
            }
        }

        UserStatement::AlterUserPassword { name, password } => {
            if password.is_empty() {
                return map_err(AuthError::EmptyPassword);
            }
            if password_strength(&password) == PasswordStrength::Weak {
                match confirm_weak {
                    None => {
                        return UserSqlOutcome::NeedsConfirmation {
                            warning: WEAK_PASSWORD_WARNING.to_string(),
                        }
                    }
                    Some(false) => {
                        return err_400("パスワードの変更を中止しました（パスワードが脆弱です）")
                    }
                    Some(true) => {}
                }
            }
            match auth.set_user_password(&name, &password) {
                Ok(()) => UserSqlOutcome::Ok {
                    message: format!("ユーザー '{}' のパスワードを変更しました", name),
                },
                Err(e) => map_err(e),
            }
        }

        UserStatement::DropUser { name, if_exists } => match auth.drop_user(&name, if_exists) {
            Ok(()) => UserSqlOutcome::Ok {
                message: format!("ユーザー '{}' を削除しました", name),
            },
            Err(e) => map_err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_create_user_basic() {
        let s = parse_user_statement("CREATE USER 'test' IDENTIFIED BY 'aA951753'").unwrap().unwrap();
        assert_eq!(
            s,
            UserStatement::CreateUser {
                name: "test".to_string(),
                password: "aA951753".to_string(),
                if_not_exists: false
            }
        );
    }

    #[test]
    fn parse_create_user_if_not_exists_and_lowercase() {
        let s = parse_user_statement("create user if not exists `bob` identified by \"Passw0rd!\"")
            .unwrap()
            .unwrap();
        assert_eq!(
            s,
            UserStatement::CreateUser {
                name: "bob".to_string(),
                password: "Passw0rd!".to_string(),
                if_not_exists: true
            }
        );
    }

    #[test]
    fn parse_drop_user_if_exists() {
        let s = parse_user_statement("DROP USER IF EXISTS 'x';").unwrap().unwrap();
        assert_eq!(
            s,
            UserStatement::DropUser {
                name: "x".to_string(),
                if_exists: true
            }
        );
    }

    #[test]
    fn parse_alter_user() {
        let s = parse_user_statement("ALTER USER 'x' IDENTIFIED BY 'Yy837261'").unwrap().unwrap();
        assert_eq!(
            s,
            UserStatement::AlterUserPassword {
                name: "x".to_string(),
                password: "Yy837261".to_string()
            }
        );
    }

    #[test]
    fn parse_show_users() {
        assert_eq!(
            parse_user_statement("SHOW USERS").unwrap().unwrap(),
            UserStatement::ShowUsers
        );
    }

    #[test]
    fn non_user_statements_return_none() {
        assert!(parse_user_statement("SELECT * FROM label.x").is_none());
        assert!(parse_user_statement("INSERT INTO (label.x) (a) VALUE (1)").is_none());
        assert!(parse_user_statement("  update label.x set a=1").is_none());
    }

    #[test]
    fn quoted_password_with_spaces_and_escapes() {
        let s = parse_user_statement("CREATE USER 'a' IDENTIFIED BY 'p ''q'' r'")
            .unwrap()
            .unwrap();
        match s {
            UserStatement::CreateUser { password, .. } => assert_eq!(password, "p 'q' r"),
            _ => panic!(),
        }
    }

    #[test]
    fn syntax_errors() {
        assert!(parse_user_statement("CREATE USER 'a'").unwrap().is_err());
        assert!(parse_user_statement("CREATE USER IDENTIFIED BY 'x'").unwrap().is_err());
        assert!(parse_user_statement("DROP USER").unwrap().is_err());
        assert!(parse_user_statement("CREATE USER 'a' IDENTIFIED BY 'x' EXTRA").unwrap().is_err());
    }

    #[test]
    fn weak_password_needs_confirmation_then_proceeds() {
        let mut auth = AuthState::bypass_for_tests();
        let stmt = UserStatement::CreateUser {
            name: "weak".to_string(),
            password: "1234".to_string(),
            if_not_exists: false,
        };
        match execute_user_statement(&mut auth, "kagura", stmt.clone(), None) {
            UserSqlOutcome::NeedsConfirmation { warning } => assert!(warning.contains("too weak")),
            _ => panic!("expected NeedsConfirmation"),
        }
        match execute_user_statement(&mut auth, "kagura", stmt.clone(), Some(false)) {
            UserSqlOutcome::Err { .. } => {}
            _ => panic!("expected Err on cancel"),
        }
        match execute_user_statement(&mut auth, "kagura", stmt, Some(true)) {
            UserSqlOutcome::Ok { .. } => {}
            _ => panic!("expected Ok on confirm"),
        }
        assert!(auth.login("weak", "1234").is_ok());
    }

    #[test]
    fn general_user_denied() {
        let mut auth = AuthState::bypass_for_tests();
        auth.create_user("norm", "aA951753", false).unwrap();
        let stmt = UserStatement::ShowUsers;
        match execute_user_statement(&mut auth, "norm", stmt, None) {
            UserSqlOutcome::Err { status, message } => {
                assert_eq!(status, 403);
                assert!(message.contains("権限"));
            }
            _ => panic!("expected permission denied"),
        }
    }
}
