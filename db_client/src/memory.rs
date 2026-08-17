// src/memory.rs
// メモリ使用量監視・上限設定
//
// db_engine の Database は全レコードをオンメモリ(Vec/HashMap)で保持するため、
// データ量に応じてプロセスメモリを際限なく消費しうる。
// ここでは OS 搭載メモリ量とプロセスの実メモリ使用量(RSS)を /proc から読み取り、
// 設定された上限に対する消費率を監視するための型・関数を提供する。
//
// 上限は「OS搭載メモリに対する割合(%, 1%刻み)」または「絶対値(バイト数)」で指定でき、
// 設定は JSON ファイルへ永続化してプロセス再起動後も引き継ぐ。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;

/// メモリ上限の指定方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryLimitMode {
    /// OS搭載メモリに対する割合(1〜100, 1%刻み)
    Percent,
    /// 絶対値(バイト数)
    Absolute,
}

/// メモリ上限設定
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MemoryLimitConfig {
    pub mode: MemoryLimitMode,
    /// mode = Percent の場合に使用する割合(1〜100)
    pub percent: u32,
    /// mode = Absolute の場合に使用するバイト数
    pub absolute_bytes: u64,
}

/// 既定の割合: OS搭載メモリの60%
pub const DEFAULT_PERCENT: u32 = 60;
/// Warning ログを出す消費率のしきい値(上限に対する割合)
pub const WARNING_RATIO: f64 = 0.6;
/// Alert ログを出す消費率のしきい値(上限に対する割合)
pub const ALERT_RATIO: f64 = 0.8;

impl Default for MemoryLimitConfig {
    fn default() -> Self {
        MemoryLimitConfig { mode: MemoryLimitMode::Percent, percent: DEFAULT_PERCENT, absolute_bytes: 0 }
    }
}

impl MemoryLimitConfig {
    /// OS搭載メモリ量(バイト)から実効上限(バイト)を計算する。
    /// Percentモードで OS搭載メモリ量が取得できていない場合は None を返す
    /// (Absoluteモードは OS搭載メモリ量に依存しないため常に値を返す)。
    pub fn effective_limit_bytes(&self, os_total_bytes: Option<u64>) -> Option<u64> {
        match self.mode {
            MemoryLimitMode::Percent => {
                os_total_bytes.map(|t| ((t as f64) * (self.percent as f64) / 100.0) as u64)
            }
            MemoryLimitMode::Absolute => Some(self.absolute_bytes),
        }
    }

    /// 設定ファイルから読み込む。存在しない/内容が不正な場合は None。
    pub fn load(path: &str) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// 設定ファイルへ保存する(アトミック書き込み: tmpファイル経由でrename)。
    pub fn save(&self, path: &str) -> io::Result<()> {
        let tmp_path = format!("{}.tmp", path);
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(&tmp_path, data)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

/// OS搭載メモリ量(バイト)を取得する。/proc/meminfo の MemTotal を読む(Linux)。
/// 取得できない環境では None。
pub fn os_total_memory_bytes() -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo_total_kb(&content).map(|kb| kb * 1024)
}

fn parse_meminfo_total_kb(content: &str) -> Option<u64> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb_str = rest.trim().split_whitespace().next()?;
            return kb_str.parse::<u64>().ok();
        }
    }
    None
}

/// 現在のプロセスの実メモリ使用量(RSS, バイト)を取得する。
/// /proc/self/status の VmRSS を読む(Linux)。取得できない環境では None。
pub fn process_rss_bytes() -> Option<u64> {
    let content = fs::read_to_string("/proc/self/status").ok()?;
    parse_status_vmrss_kb(&content).map(|kb| kb * 1024)
}

fn parse_status_vmrss_kb(content: &str) -> Option<u64> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb_str = rest.trim().split_whitespace().next()?;
            return kb_str.parse::<u64>().ok();
        }
    }
    None
}

/// 上限バイト数と現在の使用量バイト数から、上限を超えているかどうかを判定する。
/// 上限・使用量のいずれかが取得できていない場合、または上限が 0(未設定扱い)の場合は false。
pub fn is_over_limit(limit_bytes: Option<u64>, usage_bytes: Option<u64>) -> bool {
    match (limit_bytes, usage_bytes) {
        (Some(limit), Some(usage)) => limit > 0 && usage >= limit,
        _ => false,
    }
}

/// 消費率に応じたアラート段階
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAlertLevel {
    Normal,
    Warning,
    Alert,
}

impl MemoryAlertLevel {
    pub fn from_ratio(ratio: f64) -> Self {
        if ratio >= ALERT_RATIO {
            MemoryAlertLevel::Alert
        } else if ratio >= WARNING_RATIO {
            MemoryAlertLevel::Warning
        } else {
            MemoryAlertLevel::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_meminfo_total_kb() {
        let sample = "MemTotal:        3927708 kB\nMemFree:          122776 kB\n";
        assert_eq!(parse_meminfo_total_kb(sample), Some(3927708));
    }

    #[test]
    fn test_parse_status_vmrss_kb() {
        let sample = "VmPeak:\t    1234 kB\nVmRSS:\t     804 kB\nVmData:\t    100 kB\n";
        assert_eq!(parse_status_vmrss_kb(sample), Some(804));
    }

    #[test]
    fn test_effective_limit_bytes_percent() {
        let cfg = MemoryLimitConfig { mode: MemoryLimitMode::Percent, percent: 60, absolute_bytes: 0 };
        assert_eq!(cfg.effective_limit_bytes(Some(1_000_000_000)), Some(600_000_000));
    }

    #[test]
    fn test_effective_limit_bytes_percent_without_os_total() {
        let cfg = MemoryLimitConfig { mode: MemoryLimitMode::Percent, percent: 60, absolute_bytes: 0 };
        assert_eq!(cfg.effective_limit_bytes(None), None);
    }

    #[test]
    fn test_effective_limit_bytes_absolute() {
        let cfg = MemoryLimitConfig { mode: MemoryLimitMode::Absolute, percent: 60, absolute_bytes: 123_456 };
        assert_eq!(cfg.effective_limit_bytes(Some(1_000_000_000)), Some(123_456));
        // Absoluteモードは OS搭載メモリ量が不明でも計算できる
        assert_eq!(cfg.effective_limit_bytes(None), Some(123_456));
    }

    #[test]
    fn test_is_over_limit() {
        assert!(is_over_limit(Some(100), Some(100)));
        assert!(is_over_limit(Some(100), Some(150)));
        assert!(!is_over_limit(Some(100), Some(99)));
        assert!(!is_over_limit(Some(0), Some(100)));
        assert!(!is_over_limit(None, Some(100)));
        assert!(!is_over_limit(Some(100), None));
    }

    #[test]
    fn test_alert_level_from_ratio() {
        assert_eq!(MemoryAlertLevel::from_ratio(0.5), MemoryAlertLevel::Normal);
        assert_eq!(MemoryAlertLevel::from_ratio(0.6), MemoryAlertLevel::Warning);
        assert_eq!(MemoryAlertLevel::from_ratio(0.75), MemoryAlertLevel::Warning);
        assert_eq!(MemoryAlertLevel::from_ratio(0.8), MemoryAlertLevel::Alert);
        assert_eq!(MemoryAlertLevel::from_ratio(1.0), MemoryAlertLevel::Alert);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let path = std::env::temp_dir()
            .join(format!("kagura_memcfg_test_{}.json", std::process::id()))
            .to_string_lossy()
            .to_string();
        let cfg = MemoryLimitConfig { mode: MemoryLimitMode::Absolute, percent: 60, absolute_bytes: 999 };
        cfg.save(&path).unwrap();
        let loaded = MemoryLimitConfig::load(&path).unwrap();
        assert_eq!(loaded.mode, MemoryLimitMode::Absolute);
        assert_eq!(loaded.absolute_bytes, 999);
        let _ = std::fs::remove_file(&path);
    }
}
