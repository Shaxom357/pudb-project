// src/backup_settings.rs
// バックアップの保存先・世代管理設定のサイドカー永続化。
//
// 二次インデックス定義（<DB_FILE>.indexes.json）・スキーマ定義（<DB_FILE>.schema.json）・
// 「全データオンメモリ」設定（<DB_FILE>.memory.json）と同じ方式で、`.kdb`/JSON 本体には
// 手を入れず `<DB_FILE>.backup.json` に保存する。運用上の恒久設定なので再起動をまたいで保持する。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 既定の世代保持数。
pub const DEFAULT_MAX_GENERATIONS: u32 = 7;
/// 既定の保持日数。
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupSettings {
    /// `BACKUP TO` にディレクトリを渡したときの既定保存先。空なら `<DB_FILE のディレクトリ>/backups`。
    #[serde(default)]
    pub backup_dir: String,
    /// サーバーローカル保存先に残す世代数の上限（0 = 無制限）。
    #[serde(default = "default_max_generations")]
    pub max_generations: u32,
    /// これより古いバックアップを剪定する日数（0 = 無制限）。
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_max_generations() -> u32 { DEFAULT_MAX_GENERATIONS }
fn default_retention_days() -> u32 { DEFAULT_RETENTION_DAYS }

impl Default for BackupSettings {
    fn default() -> Self {
        BackupSettings {
            backup_dir: String::new(),
            max_generations: DEFAULT_MAX_GENERATIONS,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

impl BackupSettings {
    /// 実際の保存先ディレクトリを解決する（`backup_dir` が空なら `<db_path のディレクトリ>/backups`）。
    pub fn resolved_dir(&self, db_path: &str) -> PathBuf {
        if !self.backup_dir.trim().is_empty() {
            return PathBuf::from(self.backup_dir.trim());
        }
        let parent = PathBuf::from(db_path)
            .parent()
            .map(|p| p.to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        parent.join("backups")
    }
}

/// `<DB_FILE>.backup.json` のパス
pub fn sidecar_path(db_path: &str) -> String {
    format!("{db_path}.backup.json")
}

/// サイドカーから設定を読む。無ければ `None`（＝既定値）。
pub fn load(db_path: &str) -> Option<BackupSettings> {
    let content = std::fs::read_to_string(sidecar_path(db_path)).ok()?;
    match serde_json::from_str::<BackupSettings>(&content) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("[WARN] {} の解析に失敗しました（既定設定で続行）: {}", sidecar_path(db_path), e);
            None
        }
    }
}

/// サイドカーが無ければ既定値を返す。
pub fn load_or_default(db_path: &str) -> BackupSettings {
    load(db_path).unwrap_or_default()
}

/// サイドカーへ設定を書き出す。
pub fn save(db_path: &str, settings: &BackupSettings) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(settings).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(sidecar_path(db_path), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("kdb_bkset_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("t.kdb");
        let db_path = db_path.to_str().unwrap();

        assert!(load(db_path).is_none());
        assert_eq!(load_or_default(db_path), BackupSettings::default());

        let s = BackupSettings { backup_dir: "/var/backups/kagura".into(), max_generations: 10, retention_days: 90 };
        save(db_path, &s).unwrap();
        assert_eq!(load(db_path), Some(s.clone()));
        assert_eq!(s.resolved_dir(db_path), PathBuf::from("/var/backups/kagura"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolved_dir_defaults_to_sibling_backups() {
        let s = BackupSettings::default();
        assert_eq!(s.resolved_dir("/var/lib/kagura-db/db_data.kdb"), PathBuf::from("/var/lib/kagura-db/backups"));
        assert_eq!(s.resolved_dir("db_data.kdb"), PathBuf::from("./backups"));
    }
}
