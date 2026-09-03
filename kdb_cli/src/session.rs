// src/session.rs
// ログインセッション（サーバーアドレス・ユーザー名・トークン）の永続化。
// $XDG_CONFIG_HOME/kdb/session.json（既定: ~/.config/kdb/session.json）に保存する。
// トークンを含むため、ファイルのパーミッションは 600（所有者のみ読み書き可）に制限する。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub server: String,
    pub username: String,
    pub token: String,
}

fn session_path() -> io::Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "設定ディレクトリ(~/.config相当)が見つかりません"))?;
    Ok(base.join("kdb").join("session.json"))
}

impl Session {
    pub fn load() -> io::Result<Option<Session>> {
        let path = session_path()?;
        match fs::read_to_string(&path) {
            Ok(content) => {
                let session = serde_json::from_str(&content).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, format!("セッションファイルの読み込みに失敗しました: {}", e))
                })?;
                Ok(Some(session))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = session_path()?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&path, json)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    pub fn delete() -> io::Result<()> {
        let path = session_path()?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}
