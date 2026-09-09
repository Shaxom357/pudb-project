// db_engine/src/backup.rs
// `.kbak` バックアップアーカイブの読み書き（依存ライブラリなしの独自コンテナ形式）。
//
// KAGURA DB を別サーバーや新規／再インストール環境でも復旧できるよう、
// `.kdb` 本体・サイドカーファイル（`*.indexes.json` / `*.schema.json` / `*.memory.json`）・
// `auth.json` を1ファイルにまとめる。各ファイルは SHA-256 で完全性を検証する。
//
// ファイル構造:
//   magic[6]        = b"KBAK1\n"
//   manifest_len[4] = マニフェスト JSON のバイト長（u32 LE）
//   manifest[...]   = マニフェスト JSON（UTF-8）
//   繰り返し（manifest.files の順）:
//     blob_len[8]   = ファイル内容のバイト長（u64 LE）
//     blob[...]     = ファイル内容（.kdb は暗号化されたまま）
//
// マスターキー本体はアーカイブに含めない（`embedded_key` は `WITH KEY` 明示時のみ）。
// `key_fingerprint` で作成時と復元先のマスターキー一致を突き合わせる。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sha256::sha256_hex;

pub const MAGIC: &[u8; 6] = b"KBAK1\n";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum BackupError {
    Io(std::io::Error),
    BadFormat(String),
    ChecksumMismatch { logical: String },
    Json(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Io(e) => write!(f, "IO: {}", e),
            BackupError::BadFormat(s) => write!(f, "バックアップ形式エラー: {}", s),
            BackupError::ChecksumMismatch { logical } => {
                write!(f, "チェックサム不一致（{} が破損しています）", logical)
            }
            BackupError::Json(s) => write!(f, "マニフェスト JSON エラー: {}", s),
        }
    }
}
impl std::error::Error for BackupError {}
impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self { BackupError::Io(e) }
}

/// アーカイブ内の1ファイル。`logical` は用途を表す識別子（`kdb` / `indexes` /
/// `schema` / `memory` / `auth`）で、復元時はこの名前で展開する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub logical: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    /// バックアップを作成した KAGURA DB のバージョン（例 `"4.10.0"`）。
    pub kagura_version: String,
    /// RFC3339 の作成日時。
    pub created_at: String,
    #[serde(default)]
    pub created_by: Option<String>,
    /// `.kdb` に入っていたレコード件数。
    pub record_count: u64,
    /// 作成時のマスターキー指紋（`kdb_store::master_key_fingerprint` と同方式）。
    pub key_fingerprint: String,
    /// `auth.json` を同梱しているか。
    pub includes_auth: bool,
    /// `WITH KEY` 指定時のみ設定される、作成時マスターキーの hex64。
    /// 復元先のマスターキーが異なる場合の自動リキーに使う。
    #[serde(default)]
    pub embedded_key: Option<String>,
    pub files: Vec<ManifestFile>,
}

impl Manifest {
    /// 復元先の KAGURA メジャーバージョンと互換かどうか（`A` が一致すれば互換）。
    pub fn is_compatible_with(&self, current_version: &str) -> bool {
        major_of(&self.kagura_version) == major_of(current_version)
    }
}

fn major_of(v: &str) -> &str {
    v.split('.').next().unwrap_or(v)
}

/// バックアップ作成の入力。
pub struct NewBackup<'a> {
    pub kagura_version: String,
    pub created_at: String,
    pub created_by: Option<String>,
    pub record_count: u64,
    pub key_fingerprint: String,
    /// `WITH KEY` 指定時のみ Some（hex64）。
    pub embedded_key: Option<String>,
    /// `(logical, ソースファイルパス)` の一覧。`logical == "auth"` があれば `includes_auth = true`。
    pub files: &'a [(String, PathBuf)],
}

/// `.kbak` アーカイブを `out_path` へ原子的に書き出す（`.tmp` へ書いてから rename）。
pub fn create(out_path: &Path, spec: NewBackup) -> Result<Manifest, BackupError> {
    let mut entries = Vec::with_capacity(spec.files.len());
    let mut blobs = Vec::with_capacity(spec.files.len());
    for (logical, src) in spec.files {
        let data = fs::read(src)?;
        entries.push(ManifestFile {
            logical: logical.clone(),
            sha256: sha256_hex(&data),
            size: data.len() as u64,
        });
        blobs.push(data);
    }

    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        kagura_version: spec.kagura_version,
        created_at: spec.created_at,
        created_by: spec.created_by,
        record_count: spec.record_count,
        key_fingerprint: spec.key_fingerprint,
        includes_auth: entries.iter().any(|e| e.logical == "auth"),
        embedded_key: spec.embedded_key,
        files: entries,
    };

    let mj = serde_json::to_vec_pretty(&manifest).map_err(|e| BackupError::Json(e.to_string()))?;

    let tmp = tmp_path(out_path);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&(mj.len() as u32).to_le_bytes())?;
        f.write_all(&mj)?;
        for b in &blobs {
            f.write_all(&(b.len() as u64).to_le_bytes())?;
            f.write_all(b)?;
        }
        f.flush()?;
    }
    fs::rename(&tmp, out_path)?;
    Ok(manifest)
}

fn tmp_path(out_path: &Path) -> PathBuf {
    let mut s = out_path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

fn read_header(f: &mut fs::File) -> Result<Manifest, BackupError> {
    let mut magic = [0u8; 6];
    f.read_exact(&mut magic)
        .map_err(|_| BackupError::BadFormat("ファイルが短すぎます".into()))?;
    if &magic != MAGIC {
        return Err(BackupError::BadFormat("KAGURA バックアップファイル（.kbak）ではありません".into()));
    }
    let mut lb = [0u8; 4];
    f.read_exact(&mut lb)?;
    let mlen = u32::from_le_bytes(lb) as usize;
    let mut mj = vec![0u8; mlen];
    f.read_exact(&mut mj)?;
    let m: Manifest = serde_json::from_slice(&mj).map_err(|e| BackupError::Json(e.to_string()))?;
    if m.format_version != FORMAT_VERSION {
        return Err(BackupError::BadFormat(format!(
            "未対応のバックアップ形式バージョン: {}（このビルドは v{} まで対応）",
            m.format_version, FORMAT_VERSION
        )));
    }
    Ok(m)
}

/// マニフェストだけを読む（ファイル本体は展開しない）。互換性チェックや一覧表示に使う。
pub fn read_manifest(archive: &Path) -> Result<Manifest, BackupError> {
    let mut f = fs::File::open(archive)?;
    read_header(&mut f)
}

/// アーカイブを `dest_dir` へ展開する。各ファイルは logical 名で置かれる。
/// すべてのファイルの SHA-256 を検証し、1つでも不一致なら `ChecksumMismatch`。
/// 戻り値は `(マニフェスト, [(logical, 展開先パス)])`。
pub fn extract(
    archive: &Path,
    dest_dir: &Path,
) -> Result<(Manifest, Vec<(String, PathBuf)>), BackupError> {
    let mut f = fs::File::open(archive)?;
    let m = read_header(&mut f)?;
    fs::create_dir_all(dest_dir)?;

    let mut out = Vec::with_capacity(m.files.len());
    for entry in &m.files {
        let mut szb = [0u8; 8];
        f.read_exact(&mut szb)
            .map_err(|_| BackupError::BadFormat(format!("{} の読み出し中に終端に達しました", entry.logical)))?;
        let sz = u64::from_le_bytes(szb);
        if sz != entry.size {
            return Err(BackupError::BadFormat(format!(
                "{}: 記録サイズ {} がマニフェストの {} と一致しません",
                entry.logical, sz, entry.size
            )));
        }
        let mut data = vec![0u8; sz as usize];
        f.read_exact(&mut data)
            .map_err(|_| BackupError::BadFormat(format!("{} のデータが不足しています", entry.logical)))?;
        if sha256_hex(&data) != entry.sha256 {
            return Err(BackupError::ChecksumMismatch { logical: entry.logical.clone() });
        }
        let p = dest_dir.join(&entry.logical);
        fs::write(&p, &data)?;
        out.push((entry.logical.clone(), p));
    }
    Ok((m, out))
}

// ---------------------------------------------------------------------------
// KAGURA DB 向けの高水準ヘルパー（db_client と kdb CLI の両方から使う）
// ---------------------------------------------------------------------------

/// `db_path`（`.kdb` または JSON 本体）に対応するファイル群の `(logical, パス)` を返す。
/// サイドカーの命名は db_client の規約（`<DB_FILE>.indexes.json` など）に合わせている。
/// `include_auth` かつ `auth_path` があれば `auth.json` も含める。存在しないファイルは省く。
pub fn data_file_specs(
    db_path: &str,
    auth_path: Option<&str>,
    include_auth: bool,
) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = Vec::new();
    let candidates = [
        ("kdb", db_path.to_string()),
        ("indexes", format!("{db_path}.indexes.json")),
        ("schema", format!("{db_path}.schema.json")),
        ("memory", format!("{db_path}.memory.json")),
    ];
    for (logical, path) in candidates {
        let p = PathBuf::from(&path);
        if p.is_file() {
            v.push((logical.to_string(), p));
        }
    }
    if include_auth {
        if let Some(ap) = auth_path {
            let p = PathBuf::from(ap);
            if p.is_file() {
                v.push(("auth".to_string(), p));
            }
        }
    }
    v
}

/// logical 名から復元先の実パスを決める（`data_file_specs` の逆写像）。
fn target_path(logical: &str, db_path: &str, auth_path: Option<&str>) -> Option<PathBuf> {
    match logical {
        "kdb" => Some(PathBuf::from(db_path)),
        "indexes" => Some(PathBuf::from(format!("{db_path}.indexes.json"))),
        "schema" => Some(PathBuf::from(format!("{db_path}.schema.json"))),
        "memory" => Some(PathBuf::from(format!("{db_path}.memory.json"))),
        "auth" => auth_path.map(PathBuf::from),
        _ => None,
    }
}

pub struct RestorePlan<'a> {
    pub archive: &'a Path,
    pub db_path: &'a str,
    pub auth_path: Option<&'a str>,
    /// 別途渡された旧マスターキー（hex64）。manifest.embedded_key が優先。
    pub old_key_hex: Option<&'a str>,
    /// 復元を実行している KAGURA DB のバージョン（メジャー互換チェック用）。
    pub current_version: &'a str,
    /// 作業用ディレクトリの親（通常は `.kdb` があるディレクトリ）。
    pub work_parent: &'a Path,
}

#[derive(Debug)]
pub struct RestoreReport {
    pub manifest: Manifest,
    /// 復元先のマスターキーが作成時と異なり、リキーを行ったか。
    pub rekeyed: bool,
    /// 置き換え前の現ファイルを退避したディレクトリ。
    pub pre_restore_dir: PathBuf,
    /// 実際に配置したファイルの logical 名。
    pub placed: Vec<String>,
}

/// アーカイブを検証・展開し、必要ならリキーして `db_path` 等へ配置する。
///
/// - メジャーバージョン非互換なら `BadFormat` で中断（何も置き換えない）。
/// - `.kdb` モードで作成時と復元先のマスターキー指紋が異なる場合、旧キー
///   （`manifest.embedded_key` → `plan.old_key_hex` の順で解決）が得られれば
///   リキーし、得られなければ `BadFormat` で中断する。
/// - 置き換え前の現ファイルは `pre-restore-<ts>/` へ退避する。
pub fn restore(plan: RestorePlan) -> Result<RestoreReport, BackupError> {
    let manifest = read_manifest(plan.archive)?;
    if !manifest.is_compatible_with(plan.current_version) {
        return Err(BackupError::BadFormat(format!(
            "バックアップのバージョン {} は現在の {} とメジャーバージョンが異なり、互換性がありません",
            manifest.kagura_version, plan.current_version
        )));
    }

    let ts = unix_nanos();
    let staging = plan.work_parent.join(format!(".kagura-restore-{ts}"));
    let (manifest, extracted) = extract(plan.archive, &staging)?;

    let is_kdb = plan.db_path.ends_with(".kdb");
    let mut rekeyed = false;

    if is_kdb {
        let current_key = crate::kdb_store::resolve_master_key();
        let current_fp = crate::kdb_store::fingerprint_of_key(&current_key);
        if manifest.key_fingerprint != current_fp {
            // リキーが必要。旧キーを解決する。
            let old_hex = manifest
                .embedded_key
                .clone()
                .or_else(|| plan.old_key_hex.map(|s| s.to_string()));
            let Some(old_hex) = old_hex else {
                let _ = fs::remove_dir_all(&staging);
                return Err(BackupError::BadFormat(format!(
                    "このバックアップは別のマスターキー（指紋 {}）で作成されています。\
                     復元先の現在のキー（指紋 {}）と異なるため、OLD KEY '<hex64>' で作成時の\
                     マスターキーを指定してください",
                    manifest.key_fingerprint, current_fp
                )));
            };
            let old_key = crate::kdb_store::parse_master_key_hex(&old_hex)
                .map_err(BackupError::BadFormat)?;
            if crate::kdb_store::fingerprint_of_key(&old_key) != manifest.key_fingerprint {
                let _ = fs::remove_dir_all(&staging);
                return Err(BackupError::BadFormat(
                    "指定された旧マスターキーの指紋がバックアップの記録と一致しません".into(),
                ));
            }
            let staged_kdb = staging.join("kdb");
            if let Err(e) = crate::kdb_store::rekey_kdb(
                staged_kdb.to_str().unwrap_or_default(),
                &old_key,
                &current_key,
            ) {
                let _ = fs::remove_dir_all(&staging);
                return Err(BackupError::BadFormat(format!("リキーに失敗しました: {}", e)));
            }
            rekeyed = true;
        }
    }

    // 現ファイルを退避する。
    // - データ系（kdb・サイドカー）は常に退避＝復元後の状態をバックアップに完全一致させる
    //   （バックアップに無いサイドカー ＝ 作成時にそのインデックス／スキーマが無かった、の意味）。
    // - auth は「バックアップが同梱している場合のみ」置き換える（WITHOUT AUTH のときは現ユーザーを維持）。
    let has_auth = manifest.files.iter().any(|f| f.logical == "auth");
    let retire: &[&str] = if has_auth {
        &["kdb", "indexes", "schema", "memory", "auth"]
    } else {
        &["kdb", "indexes", "schema", "memory"]
    };
    let pre_restore_dir = plan.work_parent.join(format!("pre-restore-{ts}"));
    fs::create_dir_all(&pre_restore_dir)?;
    for logical in retire {
        if let Some(cur) = target_path(logical, plan.db_path, plan.auth_path) {
            if cur.is_file() {
                let name = cur.file_name().map(|n| n.to_owned()).unwrap_or_else(|| (*logical).into());
                let _ = fs::rename(&cur, pre_restore_dir.join(name));
            }
        }
    }

    // ステージングのファイルを配置
    let mut placed = Vec::new();
    for (logical, staged) in &extracted {
        if let Some(dst) = target_path(logical, plan.db_path, plan.auth_path) {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            // rename が跨デバイスで失敗する場合に備えて copy → remove にフォールバック
            if fs::rename(staged, &dst).is_err() {
                fs::copy(staged, &dst)?;
            }
            placed.push(logical.clone());
        }
    }

    let _ = fs::remove_dir_all(&staging);

    Ok(RestoreReport { manifest, rekeyed, pre_restore_dir, placed })
}

/// `dir` 内の `kagura-backup-*.kbak` を新しい順に並べ、`max_generations` を超える分と
/// `retention_days` より古い分を削除する。0 は「制限なし」。削除したファイル名を返す。
pub fn prune_generations(
    dir: &Path,
    max_generations: u32,
    retention_days: u32,
) -> std::io::Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut items: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("kagura-backup-") || !name.ends_with(".kbak") {
            continue;
        }
        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        items.push((path, mtime));
    }
    // 新しい順
    items.sort_by(|a, b| b.1.cmp(&a.1));

    let now = std::time::SystemTime::now();
    let retention = std::time::Duration::from_secs(u64::from(retention_days) * 86_400);
    let mut removed = Vec::new();
    for (i, (path, mtime)) in items.iter().enumerate() {
        let over_count = max_generations > 0 && i as u32 >= max_generations;
        let too_old = retention_days > 0
            && now.duration_since(*mtime).map(|d| d > retention).unwrap_or(false);
        if over_count || too_old {
            if fs::remove_file(path).is_ok() {
                removed.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
            }
        }
    }
    Ok(removed)
}

fn unix_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kdb_backup_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn test_create_extract_roundtrip() {
        let dir = tmp_dir("roundtrip");
        let kdb = dir.join("db_data.kdb");
        let idx = dir.join("db_data.kdb.indexes.json");
        let auth = dir.join("auth.json");
        fs::write(&kdb, b"encrypted-kdb-bytes").unwrap();
        fs::write(&idx, br#"{"indexes":[]}"#).unwrap();
        fs::write(&auth, br#"{"users":[]}"#).unwrap();

        let files = vec![
            ("kdb".to_string(), kdb.clone()),
            ("indexes".to_string(), idx.clone()),
            ("auth".to_string(), auth.clone()),
        ];
        let out = dir.join("backup.kbak");
        let manifest = create(
            &out,
            NewBackup {
                kagura_version: "4.10.0".into(),
                created_at: "2026-09-08T12:00:00Z".into(),
                created_by: Some("kagura".into()),
                record_count: 3,
                key_fingerprint: "abcd1234".into(),
                embedded_key: None,
                files: &files,
            },
        )
        .unwrap();
        assert!(manifest.includes_auth);
        assert_eq!(manifest.files.len(), 3);

        let read = read_manifest(&out).unwrap();
        assert_eq!(read.record_count, 3);
        assert_eq!(read.kagura_version, "4.10.0");

        let dest = tmp_dir("roundtrip_dest");
        let (m2, extracted) = extract(&out, &dest).unwrap();
        assert_eq!(m2.files.len(), 3);
        assert_eq!(extracted.len(), 3);
        assert_eq!(fs::read(dest.join("kdb")).unwrap(), b"encrypted-kdb-bytes");
        assert_eq!(fs::read(dest.join("auth")).unwrap(), br#"{"users":[]}"#);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_extract_detects_corruption() {
        let dir = tmp_dir("corrupt");
        let kdb = dir.join("x.kdb");
        fs::write(&kdb, b"hello world payload").unwrap();
        let out = dir.join("b.kbak");
        create(
            &out,
            NewBackup {
                kagura_version: "4.10.0".into(),
                created_at: "t".into(),
                created_by: None,
                record_count: 0,
                key_fingerprint: "fp".into(),
                embedded_key: None,
                files: &[("kdb".to_string(), kdb.clone())],
            },
        )
        .unwrap();

        // アーカイブ末尾（＝ blob の中身）を1バイト書き換える
        let mut bytes = fs::read(&out).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&out, &bytes).unwrap();

        let dest = tmp_dir("corrupt_dest");
        match extract(&out, &dest) {
            Err(BackupError::ChecksumMismatch { logical }) => assert_eq!(logical, "kdb"),
            other => panic!("expected ChecksumMismatch, got {:?}", other),
        }

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_rejects_non_kbak() {
        let dir = tmp_dir("notkbak");
        let f = dir.join("random.bin");
        fs::write(&f, b"this is not a kbak file at all").unwrap();
        assert!(matches!(read_manifest(&f), Err(BackupError::BadFormat(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_major_version_compatibility() {
        let dir = tmp_dir("compat");
        let f = dir.join("a.kdb");
        fs::write(&f, b"x").unwrap();
        let out = dir.join("a.kbak");
        let m = create(
            &out,
            NewBackup {
                kagura_version: "4.10.0".into(),
                created_at: "t".into(),
                created_by: None,
                record_count: 0,
                key_fingerprint: "fp".into(),
                embedded_key: None,
                files: &[("kdb".to_string(), f.clone())],
            },
        )
        .unwrap();
        assert!(m.is_compatible_with("4.0.0"));
        assert!(m.is_compatible_with("4.99.5"));
        assert!(!m.is_compatible_with("5.0.0"));
        assert!(!m.is_compatible_with("3.9.0"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_restore_places_files_and_keeps_pre_restore_copy() {
        use crate::kdb_store::{fingerprint_of_key, resolve_master_key, KdbFile};
        use crate::{DataType, Record};

        let dir = tmp_dir("restore");
        let db_path = dir.join("db_data.kdb");
        let db_path_s = db_path.to_str().unwrap().to_string();
        let idx_path = dir.join("db_data.kdb.indexes.json");

        // 現行の .kdb（現マスターキーで作成）＋サイドカー
        {
            let mut kdb = KdbFile::create(&db_path_s).unwrap();
            let mut r = Record::new(1);
            r.set("v", DataType::Text("original".into()));
            kdb.compact(&[r], 2).unwrap();
        }
        fs::write(&idx_path, br#"{"indexes":["old"]}"#).unwrap();

        // バックアップ相当の .kdb を別途作り（同じ現マスターキー）、アーカイブ化
        let src_dir = tmp_dir("restore_src");
        let src_kdb = src_dir.join("k");
        {
            let mut kdb = KdbFile::create(src_kdb.to_str().unwrap()).unwrap();
            let mut r = Record::new(1);
            r.set("v", DataType::Text("from-backup".into()));
            let mut r2 = Record::new(2);
            r2.set("v", DataType::Text("second".into()));
            kdb.compact(&[r, r2], 3).unwrap();
        }
        let src_idx = src_dir.join("i");
        fs::write(&src_idx, br#"{"indexes":["restored"]}"#).unwrap();

        let archive = dir.join("b.kbak");
        create(
            &archive,
            NewBackup {
                kagura_version: "4.10.0".into(),
                created_at: "t".into(),
                created_by: None,
                record_count: 2,
                key_fingerprint: fingerprint_of_key(&resolve_master_key()),
                embedded_key: None,
                files: &[
                    ("kdb".to_string(), src_kdb.clone()),
                    ("indexes".to_string(), src_idx.clone()),
                ],
            },
        )
        .unwrap();

        let report = restore(RestorePlan {
            archive: &archive,
            db_path: &db_path_s,
            auth_path: None,
            old_key_hex: None,
            current_version: "4.10.5",
            work_parent: &dir,
        })
        .unwrap();

        assert!(!report.rekeyed, "同じマスターキーならリキー不要");
        assert!(report.placed.contains(&"kdb".to_string()));

        // 復元後の .kdb は 2 件で "from-backup" が入っている
        let mut kdb = KdbFile::open(&db_path_s).unwrap();
        let recs = kdb.read_all_records().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].1.columns.get("v").unwrap(), &DataType::Text("from-backup".into()));
        assert_eq!(fs::read(&idx_path).unwrap(), br#"{"indexes":["restored"]}"#);

        // 退避コピーに元データが残っている
        assert!(report.pre_restore_dir.is_dir());
        let saved_kdb = report.pre_restore_dir.join("db_data.kdb");
        assert!(saved_kdb.is_file());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&src_dir);
    }

    #[test]
    fn test_restore_rejects_incompatible_major_version() {
        let dir = tmp_dir("restore_incompat");
        let f = dir.join("k");
        fs::write(&f, b"x").unwrap();
        let archive = dir.join("b.kbak");
        create(
            &archive,
            NewBackup {
                kagura_version: "3.4.0".into(),
                created_at: "t".into(),
                created_by: None,
                record_count: 0,
                key_fingerprint: "fp".into(),
                embedded_key: None,
                files: &[("kdb".to_string(), f.clone())],
            },
        )
        .unwrap();
        let db_path = dir.join("db.json");
        fs::write(&db_path, b"[]").unwrap();
        let err = restore(RestorePlan {
            archive: &archive,
            db_path: db_path.to_str().unwrap(),
            auth_path: None,
            old_key_hex: None,
            current_version: "4.10.0",
            work_parent: &dir,
        })
        .unwrap_err();
        assert!(matches!(err, BackupError::BadFormat(_)));
        // 既存ファイルは触られていない
        assert_eq!(fs::read(&db_path).unwrap(), b"[]");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_generations_by_count() {
        let dir = tmp_dir("prune");
        for i in 0..5 {
            let p = dir.join(format!("kagura-backup-2026090{}-000000.kbak", i));
            fs::write(&p, b"x").unwrap();
            // mtime をずらす
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        fs::write(dir.join("unrelated.txt"), b"keep me").unwrap();

        let removed = prune_generations(&dir, 2, 0).unwrap();
        assert_eq!(removed.len(), 3, "5件中、最新2件を残して3件削除");
        let remaining: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".kbak"))
            .collect();
        assert_eq!(remaining.len(), 2);
        assert!(dir.join("unrelated.txt").is_file(), "無関係なファイルは消さない");
        let _ = fs::remove_dir_all(&dir);
    }
}
