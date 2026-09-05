// db_engine/src/kdb_store.rs
// .kdb バイナリ暗号化ストレージ (WAL方式)
//
// ファイル構造:
//   [Header 64bytes 平文]
//   magic[4]="KGDB" version[2] flags[2] salt[16] next_id[8]
//   record_count[8] wal_entries[8] reserved[16]
//   ---
//   [WAL entries (追記ログ)]
//   [entry_len:u32][nonce:24][XChaCha20-Poly1305 ciphertext+tag]

use std::io::{Read, Write, Seek, SeekFrom};
use std::fs::{File, OpenOptions};
use crate::codec::{encode_record, decode_record};
use crate::crypto::{xchacha20poly1305_encrypt, xchacha20poly1305_decrypt, derive_key, CryptoError};
use crate::codec::CodecError;
use crate::Record;

const MAGIC: &[u8; 4] = b"KGDB";
const VERSION: u16    = 0x0001;
const HEADER_SIZE: usize = 64;

const DEFAULT_MASTER_KEY: [u8; 32] = [
    0x4B,0x41,0x47,0x55,0x52,0x41,0x5F,0x44,
    0x42,0x5F,0x4D,0x41,0x53,0x54,0x45,0x52,
    0x5F,0x4B,0x45,0x59,0x5F,0x56,0x31,0x5F,
    0x44,0x45,0x46,0x41,0x55,0x4C,0x54,0x21,
];

fn get_master_key() -> [u8; 32] {
    if let Ok(hex) = std::env::var("KAGURA_MASTER_KEY") {
        if hex.len() == 64 {
            let mut key = [0u8; 32];
            for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
                if let Ok(s) = std::str::from_utf8(chunk) {
                    if let Ok(b) = u8::from_str_radix(s, 16) { key[i] = b; }
                }
            }
            return key;
        }
    }
    DEFAULT_MASTER_KEY
}

#[derive(Debug)]
struct KdbHeader { salt: [u8; 16], next_id: u64, record_count: u64, wal_entries: u64 }

impl KdbHeader {
    fn new() -> Self { KdbHeader { salt: rnd16(), next_id: 1, record_count: 0, wal_entries: 0 } }

    fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut h = [0u8; HEADER_SIZE];
        h[0..4].copy_from_slice(MAGIC);
        h[4..6].copy_from_slice(&VERSION.to_le_bytes());
        h[6..8].copy_from_slice(&0x0001u16.to_le_bytes()); // FLAG_ENCRYPTED
        h[8..24].copy_from_slice(&self.salt);
        h[24..32].copy_from_slice(&self.next_id.to_le_bytes());
        h[32..40].copy_from_slice(&self.record_count.to_le_bytes());
        h[40..48].copy_from_slice(&self.wal_entries.to_le_bytes());
        h
    }

    fn from_bytes(b: &[u8; HEADER_SIZE]) -> Result<Self, KdbError> {
        if &b[0..4] != MAGIC { return Err(KdbError::InvalidMagic); }
        let version = u16::from_le_bytes(b[4..6].try_into().unwrap());
        if version != VERSION { return Err(KdbError::UnsupportedVersion(version)); }
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&b[8..24]);
        Ok(KdbHeader {
            salt,
            next_id:      u64::from_le_bytes(b[24..32].try_into().unwrap()),
            record_count: u64::from_le_bytes(b[32..40].try_into().unwrap()),
            wal_entries:  u64::from_le_bytes(b[40..48].try_into().unwrap()),
        })
    }
}

#[derive(Debug)]
pub enum KdbError {
    Io(std::io::Error), Codec(CodecError), Crypto(CryptoError),
    InvalidMagic, UnsupportedVersion(u16), Corrupted(String),
}
impl std::fmt::Display for KdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KdbError::Io(e)                => write!(f, "IO: {}", e),
            KdbError::Codec(e)             => write!(f, "Codec: {}", e),
            KdbError::Crypto(e)            => write!(f, "Crypto: {}", e),
            KdbError::InvalidMagic         => write!(f, "Not a KAGURA DB file"),
            KdbError::UnsupportedVersion(v)=> write!(f, "Unsupported KDB version: {}", v),
            KdbError::Corrupted(s)         => write!(f, "Corrupted: {}", s),
        }
    }
}
impl std::error::Error for KdbError {}
impl From<std::io::Error> for KdbError { fn from(e: std::io::Error) -> Self { KdbError::Io(e) } }
impl From<CodecError>     for KdbError { fn from(e: CodecError)     -> Self { KdbError::Codec(e) } }
impl From<CryptoError>    for KdbError { fn from(e: CryptoError)    -> Self { KdbError::Crypto(e) } }

// ---------------------------------------------------------------------------
// WAL エントリ入出力
// ---------------------------------------------------------------------------

fn wal_append_entry(file: &mut File, enc_key: &[u8; 32], record: &Record) -> Result<(), KdbError> {
    let plain = encode_record(record);
    let nonce = rnd24();
    let ct    = xchacha20poly1305_encrypt(enc_key, &nonce, b"kdb-wal", &plain);
    let entry_len = (24 + ct.len()) as u32;
    file.write_all(&entry_len.to_le_bytes())?;
    file.write_all(&nonce)?;
    file.write_all(&ct)?;
    Ok(())
}

/// WAL全件を先頭から読む。各レコードについて、そのエントリの開始バイトオフセット
/// （`read_record_at` で単体再読込する際に使う）も併せて返す。
fn wal_read_all(file: &mut File, enc_key: &[u8; 32]) -> Result<Vec<(u64, Record)>, KdbError> {
    file.seek(SeekFrom::Start(HEADER_SIZE as u64))?;
    let mut records = Vec::new();
    loop {
        let offset = file.stream_position()?;
        let mut lbuf = [0u8; 4];
        match file.read_exact(&mut lbuf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(KdbError::Io(e)),
        }
        let elen = u32::from_le_bytes(lbuf) as usize;
        if elen < 24 { return Err(KdbError::Corrupted("entry too short".into())); }
        let mut entry = vec![0u8; elen];
        file.read_exact(&mut entry)?;
        let nonce: [u8; 24] = entry[0..24].try_into().unwrap();
        let plain = xchacha20poly1305_decrypt(enc_key, &nonce, b"kdb-wal", &entry[24..])?;
        records.push((offset, decode_record(&plain)?));
    }
    Ok(records)
}

/// 指定オフセットの1エントリだけを読んで復号する（コールドな行の再読込用）。
fn wal_read_one(file: &mut File, enc_key: &[u8; 32], offset: u64) -> Result<Record, KdbError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut lbuf = [0u8; 4];
    file.read_exact(&mut lbuf)?;
    let elen = u32::from_le_bytes(lbuf) as usize;
    if elen < 24 { return Err(KdbError::Corrupted("entry too short".into())); }
    let mut entry = vec![0u8; elen];
    file.read_exact(&mut entry)?;
    let nonce: [u8; 24] = entry[0..24].try_into().unwrap();
    let plain = xchacha20poly1305_decrypt(enc_key, &nonce, b"kdb-wal", &entry[24..])?;
    Ok(decode_record(&plain)?)
}

// ---------------------------------------------------------------------------
// KdbFile: 公開インターフェース
// ---------------------------------------------------------------------------

pub struct KdbFile { file: File, pub header: KdbHeader, pub enc_key: [u8; 32] }

impl KdbFile {
    pub fn create(path: &str) -> Result<Self, KdbError> {
        let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        let header  = KdbHeader::new();
        let enc_key = derive_key(&get_master_key(), &header.salt);
        file.write_all(&header.to_bytes())?; file.flush()?;
        Ok(KdbFile { file, header, enc_key })
    }

    pub fn open(path: &str) -> Result<Self, KdbError> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut hbuf = [0u8; HEADER_SIZE];
        file.read_exact(&mut hbuf)?;
        let header  = KdbHeader::from_bytes(&hbuf)?;
        let enc_key = derive_key(&get_master_key(), &header.salt);
        Ok(KdbFile { file, header, enc_key })
    }

    pub fn open_or_create(path: &str) -> Result<Self, KdbError> {
        if std::path::Path::new(path).exists() { Self::open(path) } else { Self::create(path) }
    }

    /// INSERT: WAL追記のみ → O(1) の高速書き込み。書き込んだエントリの開始バイトオフセット
    /// を返す（コールドな行を後から`read_record_at`で再読込する際のキーに使う）。
    pub fn append_record(&mut self, record: &Record) -> Result<u64, KdbError> {
        let offset = self.file.seek(SeekFrom::End(0))?;
        wal_append_entry(&mut self.file, &self.enc_key, record)?;
        self.file.flush()?;
        self.header.wal_entries  += 1;
        self.header.record_count += 1;
        self.flush_header()?;
        Ok(offset)
    }

    /// 全レコードを読み込む（オフセット付き）
    pub fn read_all_records(&mut self) -> Result<Vec<(u64, Record)>, KdbError> {
        wal_read_all(&mut self.file, &self.enc_key)
    }

    /// 指定オフセットの1レコードだけを読み直す（オンメモリ容量制限で退避されたレコードの
    /// 再読込用。オフセットは`read_all_records`/`append_record`/`compact`が返すもの）。
    pub fn read_record_at(&mut self, offset: u64) -> Result<Record, KdbError> {
        wal_read_one(&mut self.file, &self.enc_key, offset)
    }

    /// next_id をヘッダーに書き戻す
    pub fn update_next_id(&mut self, id: u64) -> Result<(), KdbError> {
        self.header.next_id = id;
        self.flush_header()
    }

    /// WAL圧縮：全レコードを書き直す（DELETEやUPDATE後に呼ぶ）。書き直した後の各レコードの
    /// バイトオフセットを、渡した`records`と同じ順序で返す（呼び出し側でIDと対応付けて使う）。
    pub fn compact(&mut self, records: &[Record], next_id: u64) -> Result<Vec<u64>, KdbError> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.header.next_id      = next_id;
        self.header.record_count = records.len() as u64;
        self.header.wal_entries  = records.len() as u64;
        self.file.write_all(&self.header.to_bytes())?;
        let enc_key = self.enc_key;
        let mut offsets = Vec::with_capacity(records.len());
        for r in records {
            offsets.push(self.file.stream_position()?);
            wal_append_entry(&mut self.file, &enc_key, r)?;
        }
        self.file.flush()?;
        Ok(offsets)
    }

    fn flush_header(&mut self) -> Result<(), KdbError> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&self.header.to_bytes())?;
        self.file.flush()?;
        Ok(())
    }

    pub fn next_id(&self)      -> u64 { self.header.next_id }
    pub fn record_count(&self) -> u64 { self.header.record_count }
}

// ---------------------------------------------------------------------------
// OS乱数
// ---------------------------------------------------------------------------

fn rnd16() -> [u8; 16] { let mut b=[0u8;16]; fill_rnd(&mut b); b }
fn rnd24() -> [u8; 24] { let mut b=[0u8;24]; fill_rnd(&mut b); b }

fn fill_rnd(buf: &mut [u8]) {
    if let Ok(mut f) = File::open("/dev/urandom") {
        if f.read_exact(buf).is_ok() { return; }
    }
    // フォールバック（開発環境用）
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    let mut s = t as u64 ^ 0xdeadbeef_cafebabe;
    for b in buf.iter_mut() {
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        *b = (s & 0xFF) as u8;
    }
}

