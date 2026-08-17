// src/logging.rs
// ログ出力保管機能
//
// KAGURA DB の稼働ログ（起動/停止・HTTPリクエスト・SQL実行・DB操作・エラー）を
// JSON Lines (1行1JSONオブジェクト) 形式でファイルへ永続保存しつつ、
// これまで通り標準出力にも要約を出す。
//
// 保存先は環境変数 KAGURA_LOG_FILE で変更可能（既定: logs/kagura.log）。
// GET /logs エンドポイントから直近のログを取得できる。

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    /// メモリ使用量が上限の80%(既定)を超えた場合など、Warnよりも重大な注意喚起
    Alert,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogCategory {
    /// 起動・停止
    Lifecycle,
    /// Webクライアントからの HTTP リクエスト
    Http,
    /// SQL クエリ実行
    Sql,
    /// レコード/ラベル操作・永続化などの一般的な DB 操作ログ
    Db,
    /// ディスクI/O等、ハードウェア/ストレージに起因すると見られる異常
    Hardware,
    /// メモリ使用量の監視(設定上限に対する消費率のWarning/Alert)
    Memory,
}

/// プロセス停止理由。
/// - Normal:   SIGINT/SIGTERM を受けた正常終了
/// - Error:    アプリケーションロジック上のエラー（設定不備・バインド失敗等）による停止
/// - Hardware: ディスクI/Oエラー等、HW/ストレージに起因すると見られる異常による停止
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopReason {
    Normal,
    Error,
    Hardware,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub category: LogCategory,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
}

/// ディスクI/O由来のエラーが HW/ストレージ障害らしいかどうかを判定する。
/// EIO (I/O error) / ENOSPC (No space left) / EROFS (Read-only fs) /
/// ENODEV,ENXIO (No such device) を HW起因とみなす。
pub fn is_hardware_io_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 6 | 19 | 28 | 30))
}

pub struct Logger {
    file: Mutex<Option<File>>,
    path: String,
}

impl Logger {
    /// ログファイルを開く（無ければ作成）。開けない場合でも標準出力ログは継続する。
    pub fn init(path: &str) -> Self {
        if let Some(dir) = std::path::Path::new(path).parent() {
            if !dir.as_os_str().is_empty() {
                let _ = fs::create_dir_all(dir);
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| eprintln!("[WARN] ログファイル '{}' を開けませんでした: {} (標準出力のみに出力します)", path, e))
            .ok();
        Logger { file: Mutex::new(file), path: path.to_string() }
    }

    fn now() -> String {
        Local::now().to_rfc3339()
    }

    fn write_entry(&self, entry: LogEntry) {
        let prefix = match entry.level {
            LogLevel::Info => "[INFO]",
            LogLevel::Warn => "[WARN]",
            LogLevel::Alert => "[ALERT]",
            LogLevel::Error => "[ERROR]",
        };
        println!("{} [{:?}] {}", prefix, entry.category, entry.message);

        let line = match serde_json::to_string(&entry) {
            Ok(l) => l,
            Err(_) => return,
        };
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }

    fn log(
        &self,
        level: LogLevel,
        category: LogCategory,
        message: impl Into<String>,
        duration_ms: Option<u128>,
        success: Option<bool>,
        status_code: Option<u16>,
        stop_reason: Option<StopReason>,
    ) {
        self.write_entry(LogEntry {
            timestamp: Self::now(),
            level,
            category,
            message: message.into(),
            duration_ms,
            success,
            status_code,
            stop_reason,
        });
    }

    // ---- 起動 / 停止 -------------------------------------------------

    pub fn startup(&self, addr: &str, storage_mode: &str, db_path: &str) {
        self.log(
            LogLevel::Info,
            LogCategory::Lifecycle,
            format!("KAGURA DB started on http://{} (storage={}, file={})", addr, storage_mode, db_path),
            None, None, None, None,
        );
    }

    pub fn shutdown(&self, reason: StopReason, detail: impl Into<String>) {
        let level = if reason == StopReason::Normal { LogLevel::Info } else { LogLevel::Error };
        self.log(
            level,
            LogCategory::Lifecycle,
            format!("KAGURA DB stopped: {}", detail.into()),
            None, None, None, Some(reason),
        );
    }

    // ---- HTTP ----------------------------------------------------------

    pub fn http_request(&self, method: &str, path: &str, status: u16, duration_ms: u128) {
        let success = status < 400;
        let level = if success { LogLevel::Info } else { LogLevel::Warn };
        self.log(
            level,
            LogCategory::Http,
            format!("{} {} -> {}", method, path, status),
            Some(duration_ms), Some(success), Some(status), None,
        );
    }

    // ---- SQL -------------------------------------------------------------

    pub fn sql_query(&self, query: &str, success: bool, duration_ms: u128, error: Option<&str>) {
        let level = if success { LogLevel::Info } else { LogLevel::Warn };
        let message = match error {
            Some(e) => format!("SQL failed: {} | query={}", e, query),
            None => format!("SQL ok | query={}", query),
        };
        self.log(level, LogCategory::Sql, message, Some(duration_ms), Some(success), None, None);
    }

    // ---- DB (永続化・レコード/ラベル操作) --------------------------------

    pub fn db_info(&self, message: impl Into<String>) {
        self.log(LogLevel::Info, LogCategory::Db, message, None, Some(true), None, None);
    }

    pub fn db_warn(&self, message: impl Into<String>) {
        self.log(LogLevel::Warn, LogCategory::Db, message, None, Some(false), None, None);
    }

    // ---- メモリ使用量監視 -------------------------------------------------

    /// メモリ消費率が Warning しきい値(既定60%)を超えた際に呼ぶ。
    pub fn memory_warn(&self, message: impl Into<String>) {
        self.log(LogLevel::Warn, LogCategory::Memory, message, None, None, None, None);
    }

    /// メモリ消費率が Alert しきい値(既定80%)を超えた際に呼ぶ。
    pub fn memory_alert(&self, message: impl Into<String>) {
        self.log(LogLevel::Alert, LogCategory::Memory, message, None, None, None, None);
    }

    /// メモリ消費率が正常範囲に戻った際に呼ぶ。
    pub fn memory_info(&self, message: impl Into<String>) {
        self.log(LogLevel::Info, LogCategory::Memory, message, None, None, None, None);
    }

    /// I/Oエラーを検知した際に呼ぶ。HW起因と判定できる場合は Hardware カテゴリでも記録する。
    pub fn db_io_error(&self, context: &str, e: &std::io::Error) {
        self.db_warn(format!("{}: {}", context, e));
        if is_hardware_io_error(e) {
            self.log(
                LogLevel::Error,
                LogCategory::Hardware,
                format!("{}: {} (raw_os_error={:?})", context, e, e.raw_os_error()),
                None, Some(false), None, None,
            );
        }
    }

    // ---- 直近ログの取得（ログ出力保管機能の閲覧用） ------------------------

    /// ログファイルの末尾から最大 n 件を新しい順に返す。
    pub fn recent(&self, n: usize) -> Vec<LogEntry> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let mut entries: Vec<LogEntry> = reader
            .lines()
            .filter_map(|l| l.ok())
            .filter_map(|l| serde_json::from_str::<LogEntry>(&l).ok())
            .collect();
        let start = entries.len().saturating_sub(n);
        entries.drain(..start);
        entries.reverse();
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_path(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("kagura_logging_test_{}_{}.log", std::process::id(), name))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn test_is_hardware_io_error_classifies_eio_enospc_erofs() {
        let eio = std::io::Error::from_raw_os_error(5);
        let enospc = std::io::Error::from_raw_os_error(28);
        let erofs = std::io::Error::from_raw_os_error(30);
        assert!(is_hardware_io_error(&eio));
        assert!(is_hardware_io_error(&enospc));
        assert!(is_hardware_io_error(&erofs));
    }

    #[test]
    fn test_is_hardware_io_error_does_not_classify_generic_errors() {
        let not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let addr_in_use = std::io::Error::from_raw_os_error(98); // EADDRINUSE
        assert!(!is_hardware_io_error(&not_found));
        assert!(!is_hardware_io_error(&addr_in_use));
    }

    #[test]
    fn test_logger_writes_and_reads_back_entries() {
        let path = temp_log_path("roundtrip");
        let logger = Logger::init(&path);

        logger.startup("127.0.0.1:3000", "kdb", "test.kdb");
        logger.sql_query("SELECT * FROM label.*", true, 12, None);
        logger.sql_query("SELECT bogus", false, 3, Some("parse error"));
        logger.http_request("GET", "/db/info", 200, 5);
        logger.shutdown(StopReason::Normal, "test shutdown");

        let entries = logger.recent(10);
        assert_eq!(entries.len(), 5);
        // recent() は新しい順
        assert_eq!(entries[0].category, LogCategory::Lifecycle);
        assert_eq!(entries[0].stop_reason, Some(StopReason::Normal));
        assert!(entries.iter().any(|e| e.category == LogCategory::Sql && e.success == Some(false)));
        assert!(entries.iter().any(|e| e.category == LogCategory::Http && e.status_code == Some(200)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_logger_recent_respects_limit() {
        let path = temp_log_path("limit");
        let logger = Logger::init(&path);
        for i in 0..5 {
            logger.db_info(format!("event {}", i));
        }
        let entries = logger.recent(2);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "event 4");
        assert_eq!(entries[1].message, "event 3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_memory_warn_and_alert_write_expected_level_and_category() {
        let path = temp_log_path("memalert");
        let logger = Logger::init(&path);
        logger.memory_warn("usage at 65%");
        logger.memory_alert("usage at 85%");

        let entries = logger.recent(10);
        assert!(entries.iter().any(|e| e.category == LogCategory::Memory && e.level == LogLevel::Warn));
        assert!(entries.iter().any(|e| e.category == LogCategory::Memory && e.level == LogLevel::Alert));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_db_io_error_flags_hardware_category_for_hardware_errors() {
        let path = temp_log_path("hwio");
        let logger = Logger::init(&path);
        let eio = std::io::Error::from_raw_os_error(5);
        logger.db_io_error("save failed", &eio);

        let entries = logger.recent(10);
        assert!(entries.iter().any(|e| e.category == LogCategory::Hardware));
        assert!(entries.iter().any(|e| e.category == LogCategory::Db && e.level == LogLevel::Warn));

        let _ = std::fs::remove_file(&path);
    }
}
