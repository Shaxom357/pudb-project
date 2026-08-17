// src/error.rs
// dynamic_label_management のエラー型

use db_engine::DatabaseError;

#[derive(Debug, PartialEq)]
pub enum LabelError {
    /// 対象レコードが存在しない
    RecordNotFound(u64),
    /// 指定されたラベルが存在しない（操作対象として）
    LabelNotFound(String),
    /// ラベル名が空文字
    EmptyLabelName,
    /// ラベル名に使用できない文字が含まれている
    InvalidLabelName(String),
    /// 対象レコードに既にそのラベルが付与されている（重複付与は禁止）
    DuplicateLabel(String),
    /// DB エンジンからのエラー
    DbError(String),
}

impl std::fmt::Display for LabelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LabelError::RecordNotFound(id)    => write!(f, "Record id={} not found", id),
            LabelError::LabelNotFound(label)  => write!(f, "Label '{}' not found", label),
            LabelError::EmptyLabelName        => write!(f, "Label name must not be empty"),
            LabelError::InvalidLabelName(msg) => write!(f, "Invalid label name: {}", msg),
            LabelError::DuplicateLabel(label) => write!(f, "Label '{}' is already attached", label),
            LabelError::DbError(msg)          => write!(f, "Database error: {}", msg),
        }
    }
}

impl From<DatabaseError> for LabelError {
    fn from(e: DatabaseError) -> Self {
        match e {
            DatabaseError::RecordNotFound(id)   => LabelError::RecordNotFound(id),
            DatabaseError::DuplicateLabel(label) => LabelError::DuplicateLabel(label),
            other => LabelError::DbError(other.to_string()),
        }
    }
}
