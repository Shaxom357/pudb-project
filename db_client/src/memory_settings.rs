// src/memory_settings.rs
// 「全データオンメモリ」設定（MemoryPolicy）のサイドカー永続化。
//
// 二次インデックス定義（<DB_FILE>.indexes.json）や任意スキーマ定義（<DB_FILE>.schema.json）と
// 同じ方式で、`.kdb`/JSON 本体には手を入れず、`<DB_FILE>.memory.json` に保存する。
// スキーマ強制の一時停止スイッチ（再起動で既定に戻す）と違い、こちらは運用上の恒久設定
// なので再起動をまたいで保持する。

use db_engine::MemoryPolicy;

/// `<DB_FILE>.memory.json` のパス
pub fn sidecar_path(db_path: &str) -> String {
    format!("{db_path}.memory.json")
}

/// サイドカーファイルから設定を読む（無ければ `None` ＝既定の `all_in_memory=true` のまま）
pub fn load(db_path: &str) -> Option<MemoryPolicy> {
    let content = std::fs::read_to_string(sidecar_path(db_path)).ok()?;
    match serde_json::from_str::<MemoryPolicy>(&content) {
        Ok(policy) => Some(policy),
        Err(e) => {
            eprintln!("[WARN] {} の解析に失敗しました（既定設定で続行）: {}", sidecar_path(db_path), e);
            None
        }
    }
}

/// サイドカーファイルへ設定を書き出す
pub fn save(db_path: &str, policy: &MemoryPolicy) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(policy).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(sidecar_path(db_path), json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_engine::MemorySizeSpec;

    #[test]
    fn roundtrip_bytes_policy() {
        let dir = std::env::temp_dir().join(format!("kdb_memset_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("t.kdb");
        let db_path = db_path.to_str().unwrap();

        assert!(load(db_path).is_none());

        let policy = MemoryPolicy { all_in_memory: false, limit: MemorySizeSpec::Bytes(5 * 1024 * 1024 * 1024) };
        save(db_path, &policy).unwrap();
        assert_eq!(load(db_path), Some(policy));

        let policy2 = MemoryPolicy { all_in_memory: true, limit: MemorySizeSpec::Percent(20.0) };
        save(db_path, &policy2).unwrap();
        assert_eq!(load(db_path), Some(policy2));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
