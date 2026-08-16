// src/models.rs
// HTTP API の JSON 入出力用 DTO（Data Transfer Object）

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use db_engine::DataType;

// ---------------------------------------------------------------------------
// DataType の JSON 表現
// { "type": "text", "value": "hello" } のような形式でやりとりする
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum DataValueDto {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

impl From<DataValueDto> for DataType {
    fn from(dto: DataValueDto) -> Self {
        match dto {
            DataValueDto::Text(s)    => DataType::Text(s),
            DataValueDto::Integer(n) => DataType::Integer(n),
            DataValueDto::Float(f)   => DataType::Float(f),
            DataValueDto::Boolean(b) => DataType::Boolean(b),
            DataValueDto::Null       => DataType::Null,
        }
    }
}

impl From<&DataType> for DataValueDto {
    fn from(dt: &DataType) -> Self {
        match dt {
            DataType::Text(s)    => DataValueDto::Text(s.clone()),
            DataType::Integer(n) => DataValueDto::Integer(*n),
            DataType::Float(f)   => DataValueDto::Float(*f),
            DataType::Boolean(b) => DataValueDto::Boolean(*b),
            DataType::Null       => DataValueDto::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// POST /records リクエストボディ
// ---------------------------------------------------------------------------
#[derive(Debug, Deserialize)]
pub struct CreateRecordRequest {
    /// 省略時は自動採番（0 として扱う）
    #[serde(default)]
    pub id: u64,
    /// カラム名 -> 値
    #[serde(default)]
    pub columns: HashMap<String, DataValueDto>,
    /// ラベルリスト
    #[serde(default)]
    pub labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// PUT /records/:id リクエストボディ
// ---------------------------------------------------------------------------
#[derive(Debug, Deserialize)]
pub struct UpdateRecordRequest {
    #[serde(default)]
    pub columns: HashMap<String, DataValueDto>,
    #[serde(default)]
    pub labels: Vec<String>,
}

// ---------------------------------------------------------------------------
// レコードのレスポンス表現
// ---------------------------------------------------------------------------
#[derive(Debug, Serialize)]
pub struct RecordResponse {
    pub id: u64,
    pub columns: HashMap<String, DataValueDto>,
    pub labels: Vec<String>,
}

impl RecordResponse {
    pub fn from_record(record: &db_engine::Record) -> Self {
        RecordResponse {
            id: record.id,
            columns: record
                .columns
                .iter()
                .map(|(k, v)| (k.clone(), DataValueDto::from(v)))
                .collect(),
            labels: record.labels.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// ラベル管理 API 用 DTO
// ---------------------------------------------------------------------------

/// POST /records/{id}/labels リクエストボディ
#[derive(Debug, Deserialize)]
pub struct AddLabelRequest {
    pub label: String,
}

/// POST /labels/search リクエストボディ
#[derive(Debug, Deserialize)]
pub struct LabelSearchRequest {
    /// 検索対象のラベルリスト
    pub labels: Vec<String>,
    /// "and"（デフォルト）または "or"
    pub mode: Option<String>,
}

/// PUT /labels/rename リクエストボディ
#[derive(Debug, Deserialize)]
pub struct RenameLabelRequest {
    pub old_label: String,
    pub new_label: String,
}

/// ラベル統計情報 DTO
#[derive(Debug, Serialize)]
pub struct LabelStatDto {
    pub label: String,
    pub record_count: usize,
}

/// GET /labels レスポンス
#[derive(Debug, Serialize)]
pub struct LabelListResponse {
    pub labels: Vec<String>,
    pub stats: Vec<LabelStatDto>,
}

/// POST /labels/search レスポンス
#[derive(Debug, Serialize)]
pub struct LabelSearchResponse {
    pub mode: String,
    pub matched_count: usize,
    pub records: Vec<RecordResponse>,
}

// ---------------------------------------------------------------------------
// DB情報 API 用 DTO
// ---------------------------------------------------------------------------

/// GET /db/info レスポンス
#[derive(Debug, Serialize)]
pub struct DbInfoResponse {
    /// エンジン名
    pub engine_name: String,
    /// アプリバージョン（A.B.C 形式）
    pub app_version: String,
    /// ストレージモード: "kdb" | "json"
    pub storage_mode: String,
    /// ストレージ形式の詳細説明
    pub storage_format: String,
    /// 暗号化されているか
    pub encrypted: bool,
    /// データファイルパス
    pub db_file_path: String,
    /// データファイルサイズ（bytes）。ファイルが存在しない場合は None
    pub db_file_size_bytes: Option<u64>,
    /// 有効なレコード数
    pub record_count: usize,
    /// 論理削除済みレコード数
    pub deleted_count: usize,
    /// アロケート済みスロット数
    pub slot_count: usize,
    /// ユニークラベル数
    pub label_count: usize,
    /// ユニークカラム数
    pub column_count: usize,
    /// カラム名一覧
    pub columns: Vec<String>,
    /// 次の自動採番ID
    pub next_id: u64,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

impl ErrorResponse {
    pub fn new(msg: impl Into<String>) -> Self {
        ErrorResponse { error: msg.into() }
    }
}
