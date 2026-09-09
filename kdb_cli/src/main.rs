// src/main.rs
// kdb: KAGURA DB インストール後に使えるコマンドラインクライアント。
// まずはログイン周り（login / logout / whoami）を実装する。

mod backup_cmd;
mod client;
mod session;

use clap::{Args, Parser, Subcommand, ValueEnum};
use client::{normalize_address, Client};
use session::Session;
use std::io::Write;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "kdb", version, about = "KAGURA DB 用コマンドラインクライアント")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// KAGURA DB サーバーにログインし、セッションを保存する
    Login(LoginArgs),
    /// 現在のセッションからログアウトする
    Logout,
    /// 現在ログイン中のセッション情報を表示する
    Whoami,
    /// 現在のログイン状態をシェルプロンプト用の短い文字列として出力する
    /// （`shell-init` が生成するプロンプトフックから内部的に呼び出される）
    Prompt,
    /// シェルプロンプトにログイン状態を表示するための連携スクリプトを出力する。
    /// シェルの設定ファイル（~/.bashrc や ~/.zshrc）に
    /// `eval "$(kdb shell-init bash)"` のように1行追加して使う
    ShellInit { shell: ShellKind },
    /// 「全データオンメモリ」の有効/無効を切り替える（管理者ロール kagura のみ）。
    /// 例: `kdb in-memory-mode disable`
    InMemoryMode {
        /// enable = 全データをメモリに保持 / disable = 容量制限モード
        #[arg(value_parser = ["enable", "disable"])]
        state: String,
    },
    /// オンメモリ容量の上限を設定する（管理者ロール kagura のみ）。
    /// 数値の後ろに MB / GB を付けると絶対サイズ、% を付けると搭載メモリに対する割合。
    /// 例: `kdb in-memory-size 5GB` / `kdb in-memory-size 20%`
    InMemorySize {
        /// 上限（例: 500MB / 2GB / 20%）
        value: String,
    },
    /// バックアップ（.kbak）を作成する（管理者ロール kagura のみ）。
    /// 既定は稼働中サーバー経由。サーバー停止中は `--direct --env-file <f>`。
    Backup(backup_cmd::BackupArgs),
    /// バックアップ（.kbak）から復元する（管理者ロール kagura のみ）。
    /// サーバー経由の場合、反映にはサーバーの再起動が必要。
    Restore(backup_cmd::RestoreArgs),
    /// KDB 暗号化マスターキーの指紋（と --reveal でキー本体）を表示する（管理者ロール kagura のみ）。
    Keyinfo(backup_cmd::KeyinfoArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum ShellKind {
    Bash,
    Zsh,
}

#[derive(Args)]
struct LoginArgs {
    /// ユーザー名
    #[arg(short = 'u', long = "user", env = "KDB_USER")]
    username: String,

    /// パスワード（省略時は対話的に非表示入力で確認する）
    #[arg(short = 'p', long = "password", env = "KDB_PASSWORD")]
    password: Option<String>,

    /// 接続先（例: 127.0.0.1:3000 / http://host:3000 / https://host）
    #[arg(short = 'a', long = "address", env = "KDB_ADDR")]
    address: String,

    /// TLS証明書の検証をスキップする（自己署名証明書向け。信頼できる接続先のみで使用すること）
    #[arg(short = 'k', long = "insecure")]
    insecure: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Login(args) => cmd_login(args).await,
        Command::Logout => cmd_logout().await,
        Command::Whoami => cmd_whoami().await,
        Command::Prompt => {
            cmd_prompt();
            Ok(())
        }
        Command::ShellInit { shell } => {
            cmd_shell_init(shell);
            Ok(())
        }
        Command::InMemoryMode { state } => cmd_in_memory_mode(state).await,
        Command::InMemorySize { value } => cmd_in_memory_size(value).await,
        Command::Backup(args) => backup_cmd::cmd_backup(args).await,
        Command::Restore(args) => backup_cmd::cmd_restore(args).await,
        Command::Keyinfo(args) => backup_cmd::cmd_keyinfo(args).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("[ERROR] {}", message);
            ExitCode::FAILURE
        }
    }
}

async fn cmd_login(args: LoginArgs) -> Result<(), String> {
    let password = match args.password {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ").map_err(|e| format!("パスワードの入力に失敗しました: {}", e))?,
    };

    let base_url = normalize_address(&args.address);
    let client = Client::new(base_url.clone(), args.insecure).map_err(|e| e.to_string())?;

    let login_resp = client
        .login(&args.username, &password)
        .await
        .map_err(|e| format!("ログインに失敗しました: {}", e))?;

    let session = Session {
        server: base_url.clone(),
        username: args.username.clone(),
        token: login_resp.token,
    };
    session
        .save()
        .map_err(|e| format!("セッションの保存に失敗しました: {}", e))?;

    println!("ログインしました: user='{}' server='{}'", args.username, base_url);

    // ログイン直後にサーバー情報を取得できれば、簡単な接続バナーとして表示する
    // （失敗してもログイン自体は成功しているため、警告に留めて成功扱いとする）
    match client.db_info(&session.token).await {
        Ok(info) => {
            println!(
                "接続先: {} v{} (storage: {})",
                info.engine_name, info.app_version, info.storage_mode
            );
        }
        Err(e) => {
            eprintln!("[WARN] サーバー情報の取得に失敗しました: {}", e);
        }
    }

    Ok(())
}

async fn cmd_logout() -> Result<(), String> {
    let session = Session::load().map_err(|e| e.to_string())?;
    let Some(session) = session else {
        println!("ログインしていません。");
        return Ok(());
    };

    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;
    if let Err(e) = client.logout(&session.token).await {
        // サーバー側が既に落ちている/トークンが失効している等でも、
        // ローカルセッションは破棄してよいため警告に留める。
        eprintln!("[WARN] サーバーへのログアウト通知に失敗しました: {}", e);
    }

    Session::delete().map_err(|e| e.to_string())?;
    println!("ログアウトしました: user='{}' server='{}'", session.username, session.server);
    Ok(())
}

async fn cmd_whoami() -> Result<(), String> {
    let session = Session::load().map_err(|e| e.to_string())?;
    let Some(session) = session else {
        println!("ログインしていません。'kdb login' を実行してください。");
        return Ok(());
    };

    println!("user: {}", session.username);
    println!("server: {}", session.server);

    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;
    match client.db_info(&session.token).await {
        Ok(info) => println!("status: 接続OK ({} v{})", info.engine_name, info.app_version),
        Err(e) => println!("status: セッションが無効です（再ログインが必要な可能性があります） - {}", e),
    }

    Ok(())
}

/// 現在のセッションを使って `PUT /settings` を叩く共通処理。
async fn put_settings_with_session(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let session = Session::load().map_err(|e| e.to_string())?;
    let Some(session) = session else {
        return Err("ログインしていません。'kdb login' を実行してください。".to_string());
    };
    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;
    client
        .put_settings(&session.token, &body)
        .await
        .map_err(|e| format!("設定の更新に失敗しました: {}", e))
}

/// 更新後の設定レスポンスから、オンメモリ関連の状況を1行で表示する。
fn print_memory_status(resp: &serde_json::Value) {
    let all_in_memory = resp.get("all_in_memory").and_then(|v| v.as_bool()).unwrap_or(true);
    if all_in_memory {
        println!("全データオンメモリ: 有効（無制限）");
        return;
    }
    let limit = resp.get("memory_limit_bytes").and_then(|v| v.as_u64());
    let resident = resp.get("memory_resident_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    let rrows = resp.get("memory_resident_rows").and_then(|v| v.as_u64()).unwrap_or(0);
    let trows = resp.get("memory_total_rows").and_then(|v| v.as_u64()).unwrap_or(0);
    let limit_str = limit
        .map(|b| format!("{:.2} MB", b as f64 / (1024.0 * 1024.0)))
        .unwrap_or_else(|| "無制限（搭載メモリ量を取得できず）".to_string());
    println!(
        "全データオンメモリ: 無効（容量制限モード）  上限 {}  / 常駐 {:.2} MB, {}/{} 行",
        limit_str,
        resident as f64 / (1024.0 * 1024.0),
        rrows,
        trows
    );
}

async fn cmd_in_memory_mode(state: String) -> Result<(), String> {
    let enable = state == "enable";
    let resp = put_settings_with_session(serde_json::json!({ "all_in_memory": enable })).await?;
    print_memory_status(&resp);
    Ok(())
}

async fn cmd_in_memory_size(value: String) -> Result<(), String> {
    let resp = put_settings_with_session(serde_json::json!({ "memory_limit_spec": value })).await?;
    print_memory_status(&resp);
    Ok(())
}

/// シェルプロンプトへ埋め込む短いタグを出力する。`shell-init` が生成する
/// `$(kdb prompt)` フックから毎回のプロンプト描画時に呼ばれるため、
/// セッションが無い/壊れている等の場合もエラーにせず単に何も出力しない
/// （プロンプトの描画を妨げないことを優先する）。
/// サーバーへの通信は行わない（ローカルのセッションファイルの有無のみを見る、
/// 高速・オフラインでも動作する軽量チェック）。トークンがサーバー側で失効/期限切れの
/// 場合でも表示上は「ログイン中」のままになりうる点に注意（正確な有効性確認は `kdb whoami` を使う）。
fn cmd_prompt() {
    let Ok(Some(session)) = Session::load() else {
        return;
    };
    let server = session.server.strip_prefix("http://").unwrap_or(&session.server);
    print!("(kdb:{}@{}) ", session.username, server);
    let _ = std::io::stdout().flush();
}

fn cmd_shell_init(shell: ShellKind) {
    let script = match shell {
        ShellKind::Bash => {
            r#"# kdb シェル統合 (bash) -- ~/.bashrc に以下を1行追加して使う:
#   eval "$(kdb shell-init bash)"
__kdb_ps1() {
    kdb prompt 2>/dev/null
}
case "$PS1" in
    *'$(__kdb_ps1)'*) ;;
    *) PS1='$(__kdb_ps1)'"$PS1" ;;
esac
"#
        }
        ShellKind::Zsh => {
            r#"# kdb シェル統合 (zsh) -- ~/.zshrc に以下を1行追加して使う:
#   eval "$(kdb shell-init zsh)"
setopt PROMPT_SUBST
__kdb_ps1() {
    kdb prompt 2>/dev/null
}
case "$PROMPT" in
    *'$(__kdb_ps1)'*) ;;
    *) PROMPT='$(__kdb_ps1)'"$PROMPT" ;;
esac
"#
        }
    };
    print!("{}", script);
}
