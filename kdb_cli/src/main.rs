// src/main.rs
// kdb: KAGURA DB インストール後に使えるコマンドラインクライアント。
// まずはログイン周り（login / logout / whoami）を実装する。

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
