// src/backup_cmd.rs
// `kdb backup` / `kdb restore` / `kdb keyinfo` の実装。
//
// 2 つの経路をサポートする:
//   - API 経由（既定）: 保存済みセッションを使い、稼働中サーバーへ `POST /sql`
//     （`BACKUP TO` / `RESTORE FROM`）または `GET /settings/master-key` を送る。
//   - 直接ファイル操作（`--direct`）: `--env-file` の `KAGURA_MASTER_KEY` /
//     `DB_FILE` / `KAGURA_AUTH_FILE` を読み、db_engine を直接呼ぶ。サーバー停止中に使う。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use db_engine::backup::{self as eng, NewBackup, RestorePlan};

use crate::client::Client;
use crate::session::Session;

#[derive(clap::Args)]
pub struct BackupArgs {
    /// 保存先。ディレクトリなら自動命名、ファイル名ならそのまま。省略時はサーバー設定の保存先。
    #[arg(short = 'o', long = "out")]
    pub out: Option<String>,
    /// マスターキーをアーカイブに同梱する（.kbak 単体で復旧可能になる。取り扱い注意）
    #[arg(long = "with-key")]
    pub with_key: bool,
    /// auth.json を同梱しない
    #[arg(long = "no-auth")]
    pub no_auth: bool,
    /// サーバーを介さずローカルファイルを直接束ねる（サーバー停止中に使う）
    #[arg(long = "direct")]
    pub direct: bool,
    /// --direct 時に読み込む環境設定ファイル（例: /etc/kagura-db/kagura.env）
    #[arg(long = "env-file")]
    pub env_file: Option<String>,
}

#[derive(clap::Args)]
pub struct RestoreArgs {
    /// 復元元の .kbak ファイル
    pub archive: String,
    /// 作成時のマスターキー（hex64）。復元先のキーと異なるとき必要（.kbak が WITH KEY なら不要）
    #[arg(long = "old-key")]
    pub old_key: Option<String>,
    /// サーバーを介さずローカルファイルへ直接展開する（サーバー停止中に使う）
    #[arg(long = "direct")]
    pub direct: bool,
    /// --direct 時に読み込む環境設定ファイル
    #[arg(long = "env-file")]
    pub env_file: Option<String>,
}

#[derive(clap::Args)]
pub struct KeyinfoArgs {
    /// マスターキー本体（hex64）を表示する（既定は指紋のみ）
    #[arg(long = "reveal")]
    pub reveal: bool,
    /// サーバーを介さず --env-file から読む
    #[arg(long = "direct")]
    pub direct: bool,
    /// --direct 時に読み込む環境設定ファイル
    #[arg(long = "env-file")]
    pub env_file: Option<String>,
}

// ---------------------------------------------------------------------------
// env ファイル
// ---------------------------------------------------------------------------

struct EnvConfig {
    db_file: String,
    auth_file: String,
    master_key: Option<String>,
}

fn parse_env_file(path: &str) -> Result<EnvConfig, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("環境設定ファイル '{}' を読めません: {}", path, e))?;
    let mut map: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            map.insert(k.trim().to_string(), v.to_string());
        }
    }
    Ok(EnvConfig {
        db_file: map.get("DB_FILE").cloned().unwrap_or_else(|| "db_data.kdb".to_string()),
        auth_file: map.get("KAGURA_AUTH_FILE").cloned().unwrap_or_else(|| "auth.json".to_string()),
        master_key: map.get("KAGURA_MASTER_KEY").cloned().filter(|s| !s.is_empty()),
    })
}

/// --direct 用: env ファイルを読み、`KAGURA_MASTER_KEY` をこのプロセスの環境へ反映する
/// （db_engine::kdb_store::resolve_master_key がこれを参照する）。
fn load_direct_env(env_file: &Option<String>) -> Result<EnvConfig, String> {
    let path = env_file
        .as_deref()
        .ok_or("--direct には --env-file <環境設定ファイル> の指定が必要です")?;
    let cfg = parse_env_file(path)?;
    if let Some(key) = &cfg.master_key {
        // SAFETY: シングルスレッドの CLI 起動直後のみ設定する。
        unsafe { std::env::set_var("KAGURA_MASTER_KEY", key); }
    }
    Ok(cfg)
}

fn warn_server_running() {
    eprintln!(
        "[WARN] --direct はサーバー停止中に使ってください。稼働中に実行すると、\
         サーバーが後で書き戻した内容と食い違う可能性があります。"
    );
}

// ---------------------------------------------------------------------------
// backup
// ---------------------------------------------------------------------------

pub async fn cmd_backup(args: BackupArgs) -> Result<(), String> {
    if args.direct {
        return cmd_backup_direct(args);
    }
    let session = require_session()?;
    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;

    let dest = args.out.clone().unwrap_or_default();
    let mut stmt = format!("BACKUP TO '{}'", dest.replace('\'', "''"));
    if args.with_key {
        stmt.push_str(" WITH KEY");
    }
    if args.no_auth {
        stmt.push_str(" WITHOUT AUTH");
    }
    if dest.is_empty() {
        return Err("API 経由では保存先を --out で指定してください（サーバー既定の保存先を使う機能は Web UI PR で対応）".to_string());
    }

    let resp = client
        .post_sql(&session.token, &stmt)
        .await
        .map_err(|e| format!("バックアップに失敗しました: {}", e))?;
    print_sql_rows(&resp);
    Ok(())
}

fn cmd_backup_direct(args: BackupArgs) -> Result<(), String> {
    warn_server_running();
    let cfg = load_direct_env(&args.env_file)?;
    let out = args.out.ok_or("--direct では保存先を --out <パス> で指定してください")?;
    let is_kdb = cfg.db_file.ends_with(".kdb");

    let files = eng::data_file_specs(&cfg.db_file, Some(&cfg.auth_file), !args.no_auth);
    if files.is_empty() {
        return Err(format!("収録対象が見つかりません（{} が存在しません）", cfg.db_file));
    }

    let out_path = resolve_out(&out);
    if let Some(p) = out_path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("保存先を作成できません: {}", e))?;
    }

    let key_fingerprint = if is_kdb {
        db_engine::kdb_store::master_key_fingerprint()
    } else {
        "n/a(json)".to_string()
    };
    let embedded_key = if args.with_key && is_kdb {
        Some(db_engine::kdb_store::master_key_hex())
    } else {
        None
    };
    if args.with_key && cfg.master_key.is_none() && is_kdb {
        eprintln!("[WARN] --with-key 指定ですが env ファイルに KAGURA_MASTER_KEY がありません（既定キーを同梱します）");
    }
    let key_embedded = embedded_key.is_some();

    let manifest = eng::create(
        &out_path,
        NewBackup {
            kagura_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            created_by: Some("kdb --direct".to_string()),
            record_count: 0, // --direct では件数を数えない
            key_fingerprint,
            embedded_key,
            files: &files,
        },
    )
    .map_err(|e| format!("アーカイブ作成に失敗しました: {}", e))?;

    let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    println!("バックアップを作成しました: {} ({} バイト)", out_path.display(), size);
    println!("収録: {}", manifest.files.iter().map(|f| f.logical.as_str()).collect::<Vec<_>>().join(", "));
    if key_embedded {
        println!("※ このアーカイブにはマスターキーが同梱されています。取り扱いに注意してください。");
    } else if is_kdb {
        println!("※ マスターキー（指紋 {}）は同梱していません。復元先で同じキーを用意してください。", manifest.key_fingerprint);
    }
    Ok(())
}

fn resolve_out(out: &str) -> PathBuf {
    let name = format!("kagura-backup-{}.kbak", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let p = PathBuf::from(out);
    if out.ends_with('/') || p.is_dir() || p.extension().is_none() {
        p.join(name)
    } else {
        p
    }
}

// ---------------------------------------------------------------------------
// restore
// ---------------------------------------------------------------------------

pub async fn cmd_restore(args: RestoreArgs) -> Result<(), String> {
    if args.direct {
        return cmd_restore_direct(args);
    }
    let session = require_session()?;
    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;

    let mut stmt = format!("RESTORE FROM '{}'", args.archive.replace('\'', "''"));
    if let Some(k) = &args.old_key {
        stmt.push_str(&format!(" OLD KEY '{}'", k.replace('\'', "''")));
    }
    let resp = client
        .post_sql(&session.token, &stmt)
        .await
        .map_err(|e| format!("復元に失敗しました: {}", e))?;
    print_sql_rows(&resp);
    println!("反映にはサーバーの再起動が必要です。");
    Ok(())
}

fn cmd_restore_direct(args: RestoreArgs) -> Result<(), String> {
    warn_server_running();
    let cfg = load_direct_env(&args.env_file)?;
    let archive = PathBuf::from(&args.archive);
    if !archive.is_file() {
        return Err(format!("バックアップファイルが見つかりません: {}", args.archive));
    }
    let work_parent = Path::new(&cfg.db_file)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let report = eng::restore(RestorePlan {
        archive: &archive,
        db_path: &cfg.db_file,
        auth_path: Some(&cfg.auth_file),
        old_key_hex: args.old_key.as_deref(),
        current_version: env!("CARGO_PKG_VERSION"),
        work_parent: &work_parent,
    })
    .map_err(|e| format!("復元に失敗しました: {}", e))?;

    println!("復元しました（{} 件、リキー: {}）", report.manifest.record_count, if report.rekeyed { "あり" } else { "なし" });
    println!("置き換え前のファイルは {} に退避しました。", report.pre_restore_dir.display());
    println!("サーバーを起動してください。");
    Ok(())
}

// ---------------------------------------------------------------------------
// keyinfo
// ---------------------------------------------------------------------------

pub async fn cmd_keyinfo(args: KeyinfoArgs) -> Result<(), String> {
    if args.direct {
        let _cfg = load_direct_env(&args.env_file)?;
        println!("fingerprint: {}", db_engine::kdb_store::master_key_fingerprint());
        if args.reveal {
            println!("master_key:  {}", db_engine::kdb_store::master_key_hex());
        }
        return Ok(());
    }
    let session = require_session()?;
    let client = Client::new(session.server.clone(), false).map_err(|e| e.to_string())?;
    let resp = client
        .get_master_key(&session.token)
        .await
        .map_err(|e| format!("マスターキー情報の取得に失敗しました: {}", e))?;
    if let Some(fp) = resp.get("fingerprint").and_then(|v| v.as_str()) {
        println!("fingerprint: {}", fp);
    }
    if args.reveal {
        if let Some(k) = resp.get("master_key").and_then(|v| v.as_str()) {
            println!("master_key:  {}", k);
        }
    } else {
        println!("（キー本体を表示するには --reveal を付けてください。管理者ロール(kagura)のみ）");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 共通
// ---------------------------------------------------------------------------

fn require_session() -> Result<Session, String> {
    Session::load()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "ログインしていません。'kdb login' を実行してください。".to_string())
}

fn print_sql_rows(resp: &serde_json::Value) {
    let cols = resp.get("columns").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let rows = resp.get("rows").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if cols.is_empty() {
        if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
            eprintln!("{}", err);
        }
        return;
    }
    let names: Vec<String> = cols.iter().map(|c| c.as_str().unwrap_or("").to_string()).collect();
    for row in rows {
        if let Some(arr) = row.as_array() {
            for (name, val) in names.iter().zip(arr) {
                let s = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                println!("{}: {}", name, s);
            }
        }
    }
}
