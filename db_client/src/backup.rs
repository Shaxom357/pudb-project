// src/backup.rs
// SQL の `BACKUP TO` / `RESTORE FROM` 文のパースと実行オーケストレーション。
//
// アーカイブのコンテナ形式・チェックサム・リキー・世代剪定は db_engine::backup が担当し、
// ここでは「収録対象ファイルの決定」「バージョン・件数・作成者などのメタ情報の付与」
// 「保存先ディレクトリの解決」を行う。
//
// バックアップ／復元は管理者ロール（kagura）のみ・トランザクション中は不可（呼び出し側で判定）。

use std::path::{Path, PathBuf};

use db_engine::backup::{self as eng, Manifest, NewBackup, RestorePlan};

use crate::backup_settings::BackupSettings;

/// このビルドの KAGURA DB バージョン（全クレート同期）。
pub const KAGURA_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub enum BackupStatement {
    /// `BACKUP TO '<path>' [WITH KEY] [WITHOUT AUTH]`
    Backup {
        /// 保存先。ディレクトリ（既存 or 末尾 `/`）ならその中に自動命名、ファイル名ならそのまま。
        dest: String,
        with_key: bool,
        include_auth: bool,
    },
    /// `RESTORE FROM '<path>' [OLD KEY '<hex64>']`
    Restore {
        archive: String,
        old_key: Option<String>,
    },
}

/// `BACKUP` / `RESTORE` 文かどうかを判定してパースする。該当しなければ `None`。
pub fn parse_backup_statement(query: &str) -> Option<Result<BackupStatement, String>> {
    let q = query.trim().trim_end_matches(';').trim();
    let upper = q.to_uppercase();
    if upper.starts_with("BACKUP") {
        Some(parse_backup(q, &upper))
    } else if upper.starts_with("RESTORE") {
        Some(parse_restore(q, &upper))
    } else {
        None
    }
}

fn parse_backup(q: &str, _upper: &str) -> Result<BackupStatement, String> {
    // BACKUP TO '<path>' [WITH KEY] [WITHOUT AUTH]
    let after_backup = q[6..].trim_start();
    let kw = after_backup.get(..2).unwrap_or("");
    let after_to = after_backup[2..].trim_start();
    if !kw.eq_ignore_ascii_case("TO") || !after_to.starts_with('\'') {
        return Err("構文エラー: BACKUP TO '<保存先>' [WITH KEY] [WITHOUT AUTH]".to_string());
    }
    let (path, rest) = take_quoted(after_to)
        .ok_or("構文エラー: 保存先はシングルクォートで囲んでください（例: BACKUP TO '/var/lib/kagura-db/backups/'）")?;
    let rest_upper = rest.trim().to_uppercase();

    let with_key = rest_upper.contains("WITH KEY");
    let without_auth = rest_upper.contains("WITHOUT AUTH");
    // 未知のトークンが無いかざっくり検査
    let cleaned = rest_upper
        .replace("WITH KEY", "")
        .replace("WITHOUT AUTH", "")
        .replace("WITH AUTH", "");
    if !cleaned.trim().is_empty() {
        return Err(format!("構文エラー: 認識できない指定があります: '{}'", rest.trim()));
    }
    if path.trim().is_empty() {
        return Err("構文エラー: 保存先が空です".to_string());
    }
    Ok(BackupStatement::Backup {
        dest: path,
        with_key,
        include_auth: !without_auth,
    })
}

fn parse_restore(q: &str, upper: &str) -> Result<BackupStatement, String> {
    let idx = upper.find("FROM").ok_or("構文エラー: RESTORE FROM '<バックアップファイル>' [OLD KEY '<hex64>']")?;
    let after_from = q[idx + 4..].trim_start();
    let (archive, rest) = take_quoted(after_from)
        .ok_or("構文エラー: バックアップファイルのパスはシングルクォートで囲んでください")?;
    let rest_trim = rest.trim();
    let mut old_key = None;
    if !rest_trim.is_empty() {
        let ru = rest_trim.to_uppercase();
        if !ru.starts_with("OLD KEY") {
            return Err(format!("構文エラー: 認識できない指定があります: '{}'", rest_trim));
        }
        let after_key = rest_trim[7..].trim_start();
        let (k, tail) = take_quoted(after_key)
            .ok_or("構文エラー: OLD KEY の値はシングルクォートで囲んでください")?;
        if !tail.trim().is_empty() {
            return Err(format!("構文エラー: OLD KEY の後に余分な指定があります: '{}'", tail.trim()));
        }
        old_key = Some(k);
    }
    if archive.trim().is_empty() {
        return Err("構文エラー: バックアップファイルのパスが空です".to_string());
    }
    Ok(BackupStatement::Restore { archive, old_key })
}

/// 先頭がシングルクォートで始まる文字列から、閉じクォートまでを取り出す。
/// 戻り値は `(中身, 残り)`。'' はエスケープとして ' 1個に畳む。
fn take_quoted(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'\'') {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                out.push('\'');
                i += 2;
                continue;
            }
            return Some((out, &s[i + 1..]));
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// 実行
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct BackupOutcome {
    pub path: PathBuf,
    pub manifest: Manifest,
    pub size: u64,
    pub pruned: Vec<String>,
}

/// `BACKUP TO` を実行する。呼び出し側は事前に現在の状態を `.kdb`/JSON へ flush（auto_save）
/// しておくこと。`db_path` は `AppStateInner::db_path`、`auth_path` は認証ファイルのパス。
#[allow(clippy::too_many_arguments)]
pub fn run_backup(
    db_path: &str,
    auth_path: &str,
    dest: &str,
    with_key: bool,
    include_auth: bool,
    actor: &str,
    record_count: u64,
    settings: &BackupSettings,
) -> Result<BackupOutcome, String> {
    let files = eng::data_file_specs(db_path, Some(auth_path), include_auth);
    if files.is_empty() {
        return Err("収録対象のファイルが見つかりません（.kdb 本体が存在しません）".to_string());
    }

    let out_path = resolve_backup_dest(dest, settings, db_path);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("保存先ディレクトリを作成できません（{}）: {}", parent.display(), e))?;
    }

    let is_kdb = db_path.ends_with(".kdb");
    let key_fingerprint = if is_kdb {
        db_engine::kdb_store::master_key_fingerprint()
    } else {
        "n/a(json)".to_string()
    };
    let embedded_key = if with_key && is_kdb {
        Some(db_engine::kdb_store::master_key_hex())
    } else {
        None
    };

    let spec = NewBackup {
        kagura_version: KAGURA_VERSION.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: Some(actor.to_string()),
        record_count,
        key_fingerprint,
        embedded_key,
        files: &files,
    };
    let manifest = eng::create(&out_path, spec).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);

    // 世代剪定（保存先ディレクトリが対象）
    let prune_dir = out_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let pruned = eng::prune_generations(&prune_dir, settings.max_generations, settings.retention_days)
        .unwrap_or_default();

    Ok(BackupOutcome { path: out_path, manifest, size, pruned })
}

/// `dest` がディレクトリ（既存 or 末尾 `/`）なら自動命名、そうでなければファイル名として扱う。
/// `dest` が空なら設定の保存先ディレクトリを使う。
fn resolve_backup_dest(dest: &str, settings: &BackupSettings, db_path: &str) -> PathBuf {
    let name = format!("kagura-backup-{}.kbak", timestamp_slug());
    let dest = dest.trim();
    if dest.is_empty() {
        return settings.resolved_dir(db_path).join(name);
    }
    let p = PathBuf::from(dest);
    let looks_like_dir = dest.ends_with('/') || dest.ends_with(std::path::MAIN_SEPARATOR) || p.is_dir();
    if looks_like_dir {
        p.join(name)
    } else if p.extension().is_none() {
        // 拡張子なし = ディレクトリ指定とみなす
        p.join(name)
    } else {
        p
    }
}

fn timestamp_slug() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

#[derive(Debug)]
pub struct RestoreOutcome {
    pub manifest: Manifest,
    pub rekeyed: bool,
    pub pre_restore_dir: PathBuf,
}

/// `RESTORE FROM` を実行する。ファイルを配置するだけで、稼働中プロセスのメモリは差し替えない
/// （呼び出し側で `restore_pending` を立て、以後の書き込みを止め、再起動を促すこと）。
pub fn run_restore(
    db_path: &str,
    auth_path: &str,
    archive: &str,
    old_key: Option<&str>,
) -> Result<RestoreOutcome, String> {
    let archive_path = PathBuf::from(archive);
    if !archive_path.is_file() {
        return Err(format!("バックアップファイルが見つかりません: {}", archive));
    }
    let work_parent = PathBuf::from(db_path)
        .parent()
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));

    let report = eng::restore(RestorePlan {
        archive: &archive_path,
        db_path,
        auth_path: Some(auth_path),
        old_key_hex: old_key,
        current_version: KAGURA_VERSION,
        work_parent: &work_parent,
    })
    .map_err(|e| e.to_string())?;

    Ok(RestoreOutcome {
        manifest: report.manifest,
        rekeyed: report.rekeyed,
        pre_restore_dir: report.pre_restore_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_backup_basic() {
        let s = parse_backup_statement("BACKUP TO '/var/backups/'").unwrap().unwrap();
        assert_eq!(
            s,
            BackupStatement::Backup { dest: "/var/backups/".into(), with_key: false, include_auth: true }
        );
    }

    #[test]
    fn parse_backup_with_options() {
        let s = parse_backup_statement("backup to '/tmp/b.kbak' WITH KEY WITHOUT AUTH;").unwrap().unwrap();
        assert_eq!(
            s,
            BackupStatement::Backup { dest: "/tmp/b.kbak".into(), with_key: true, include_auth: false }
        );
    }

    #[test]
    fn parse_restore_basic_and_old_key() {
        let s = parse_backup_statement("RESTORE FROM '/tmp/b.kbak'").unwrap().unwrap();
        assert_eq!(s, BackupStatement::Restore { archive: "/tmp/b.kbak".into(), old_key: None });

        let s = parse_backup_statement("restore from '/tmp/b.kbak' OLD KEY 'deadbeef'").unwrap().unwrap();
        assert_eq!(
            s,
            BackupStatement::Restore { archive: "/tmp/b.kbak".into(), old_key: Some("deadbeef".into()) }
        );
    }

    #[test]
    fn parse_rejects_garbage_and_non_backup() {
        assert!(parse_backup_statement("SELECT * FROM label.x").is_none());
        assert!(parse_backup_statement("BACKUP TO '/x' FOO BAR").unwrap().is_err());
        assert!(parse_backup_statement("BACKUP '/x'").unwrap().is_err());
        assert!(parse_backup_statement("RESTORE FROM /x").unwrap().is_err());
    }

    #[test]
    fn take_quoted_handles_escapes() {
        let (v, rest) = take_quoted("'it''s here' WITH KEY").unwrap();
        assert_eq!(v, "it's here");
        assert_eq!(rest.trim(), "WITH KEY");
    }

    #[test]
    fn resolve_dest_dir_vs_file() {
        let s = BackupSettings::default();
        let d = resolve_backup_dest("/tmp/backups/", &s, "/db/x.kdb");
        assert!(d.to_string_lossy().starts_with("/tmp/backups/kagura-backup-"));
        let fpath = resolve_backup_dest("/tmp/my.kbak", &s, "/db/x.kdb");
        assert_eq!(fpath, PathBuf::from("/tmp/my.kbak"));
        let def = resolve_backup_dest("", &s, "/var/lib/kagura-db/db_data.kdb");
        assert!(def.to_string_lossy().starts_with("/var/lib/kagura-db/backups/kagura-backup-"));
    }
}
