// db_engine/src/memory.rs
// 「全データオンメモリ」の ON/OFF と、OFF 時のオンメモリ容量制限（FIFO/LRU）に関する型。
//
// 既定は all_in_memory = true（＝無制限。従来どおり全レコードをメモリに保持する高速パス）。
// all_in_memory = false にすると、レコードの列データのうち、直近で使われていないものから
// メモリ上から解放し、指定した上限バイト数に収める。解放された行は、アクセスされた時に
// `.kdb` から単体で読み直して再びメモリに載せる（再アクセスで立ち退き順が更新される＝実質LRU）。

use serde::{Deserialize, Serialize};

/// オンメモリ容量の上限指定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum MemorySizeSpec {
    /// 絶対バイト数
    Bytes(u64),
    /// 搭載物理メモリに対する割合（%。例: 20.0 なら搭載メモリの20%）
    Percent(f64),
}

impl MemorySizeSpec {
    /// 実際の上限バイト数へ解決する。`Percent` で搭載メモリ量が取得できない場合は
    /// `None`（呼び出し側は「制限なし」として扱う）。
    pub fn resolve_bytes(&self) -> Option<u64> {
        match *self {
            MemorySizeSpec::Bytes(b) => Some(b),
            MemorySizeSpec::Percent(p) => {
                let total = total_physical_memory_bytes()?;
                Some(((total as f64) * (p / 100.0)).max(0.0) as u64)
            }
        }
    }
}

/// 「全データオンメモリ」設定
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MemoryPolicy {
    /// true（既定）= 無制限。全レコードをメモリに保持する（従来動作・オーバーヘッドなし）。
    /// false = `limit` の範囲でオンメモリ量を制限し、あふれた分は FIFO/LRU で解放する。
    pub all_in_memory: bool,
    /// all_in_memory = false のときの上限
    pub limit: MemorySizeSpec,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        // 既定は「全データオンメモリ有効」。limit は無効化されるまで意味を持たない値。
        MemoryPolicy {
            all_in_memory: true,
            limit: MemorySizeSpec::Bytes(1024 * 1024 * 1024),
        }
    }
}

/// 現在のオンメモリ状況（観測用。`GET /settings` や Web UI で表示する）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MemoryStats {
    pub all_in_memory: bool,
    /// 解決後の上限バイト数（`Percent` で搭載メモリ不明なら `None`）
    pub limit_bytes: Option<u64>,
    /// 現在メモリに載せている列データの推定バイト数
    pub resident_bytes: u64,
    /// メモリに実体を載せている行数
    pub resident_rows: usize,
    /// 有効な行数の合計
    pub total_rows: usize,
}

/// Linux の `/proc/meminfo` から搭載物理メモリ量（バイト）を取得する。
/// 取得できない環境（非Linux等）では `None`。
pub fn total_physical_memory_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// 数値＋単位（`500MB` / `2GB` / `20%`）の文字列をパースして `MemorySizeSpec` にする。
/// `kdb in-memory-size <値>` および Web UI からの指定で使う。
pub fn parse_size_spec(input: &str) -> Result<MemorySizeSpec, String> {
    let s = input.trim();
    if let Some(num) = s.strip_suffix('%') {
        let p: f64 = num.trim().parse().map_err(|_| format!("割合の数値を解釈できません: '{}'", input))?;
        if !(0.0..=100.0).contains(&p) {
            return Err(format!("割合は 0〜100 の範囲で指定してください: '{}'", input));
        }
        return Ok(MemorySizeSpec::Percent(p));
    }
    let upper = s.to_ascii_uppercase();
    let (num_part, mult) = if let Some(n) = upper.strip_suffix("GB") {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = upper.strip_suffix("MB") {
        (n, 1024u64 * 1024)
    } else if let Some(n) = upper.strip_suffix("KB") {
        (n, 1024u64)
    } else if let Some(n) = upper.strip_suffix('B') {
        (n, 1u64)
    } else {
        return Err(format!(
            "単位（MB / GB）または % を付けて指定してください: '{}'",
            input
        ));
    };
    let val: f64 = num_part.trim().parse().map_err(|_| format!("数値を解釈できません: '{}'", input))?;
    if val < 0.0 {
        return Err(format!("負の値は指定できません: '{}'", input));
    }
    Ok(MemorySizeSpec::Bytes((val * mult as f64) as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_spec_bytes_units() {
        assert_eq!(parse_size_spec("5GB").unwrap(), MemorySizeSpec::Bytes(5 * 1024 * 1024 * 1024));
        assert_eq!(parse_size_spec("500MB").unwrap(), MemorySizeSpec::Bytes(500 * 1024 * 1024));
        assert_eq!(parse_size_spec("2 gb").unwrap(), MemorySizeSpec::Bytes(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_size_spec_percent() {
        assert_eq!(parse_size_spec("20%").unwrap(), MemorySizeSpec::Percent(20.0));
        assert_eq!(parse_size_spec(" 12.5 % ").unwrap(), MemorySizeSpec::Percent(12.5));
    }

    #[test]
    fn parse_size_spec_rejects_bad_input() {
        assert!(parse_size_spec("5").is_err());
        assert!(parse_size_spec("abc").is_err());
        assert!(parse_size_spec("150%").is_err());
        assert!(parse_size_spec("-3GB").is_err());
    }

    #[test]
    fn bytes_spec_resolves_directly() {
        assert_eq!(MemorySizeSpec::Bytes(4096).resolve_bytes(), Some(4096));
    }
}
