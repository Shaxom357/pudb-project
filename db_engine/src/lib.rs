// db_engine/src/lib.rs
// 列指向ストレージ（Column-Oriented Storage）＋ラベル検索エンジン

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// カラムに格納できる値の型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataType {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataType::Text(s)    => write!(f, "{}", s),
            DataType::Integer(n) => write!(f, "{}", n),
            DataType::Float(v)   => write!(f, "{}", v),
            DataType::Boolean(b) => write!(f, "{}", b),
            DataType::Null       => write!(f, "NULL"),
        }
    }
}

/// 1行分のデータ（カラム名->値のマップ + ラベルリスト）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub columns: HashMap<String, DataType>,
    pub labels: Vec<String>,
}

impl Record {
    pub fn new(id: u64) -> Self {
        Record { id, columns: HashMap::new(), labels: Vec::new() }
    }
    pub fn set(&mut self, column: impl Into<String>, value: DataType) -> &mut Self {
        self.columns.insert(column.into(), value);
        self
    }
    pub fn add_label(&mut self, label: impl Into<String>) -> &mut Self {
        self.labels.push(label.into());
        self
    }
    pub fn get_col(&self, column: &str) -> Option<&DataType> {
        self.columns.get(column)
    }
}

/// エンジン操作で発生するエラー
#[derive(Debug, PartialEq)]
pub enum DatabaseError {
    RecordNotFound(u64),
    DuplicateId(u64),
    ColumnNotFound(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::RecordNotFound(id)  => write!(f, "Record id={} not found", id),
            DatabaseError::DuplicateId(id)     => write!(f, "Record id={} already exists", id),
            DatabaseError::ColumnNotFound(col) => write!(f, "Column '{}' not found", col),
        }
    }
}

/// 列ごとのデータを保持する内部構造体
#[derive(Debug, Default)]
struct ColumnStore {
    columns: HashMap<String, Vec<Option<DataType>>>,
    label_index: HashMap<String, Vec<usize>>,
}

impl ColumnStore {
    fn new() -> Self {
        ColumnStore { columns: HashMap::new(), label_index: HashMap::new() }
    }
    fn row_count(&self) -> usize {
        self.columns.values().next().map_or(0, |v| v.len())
    }
}

/// DBエンジン本体
pub struct Database {
    records: Vec<Option<Record>>,
    id_to_index: HashMap<u64, usize>,
    store: ColumnStore,
    next_id: u64,
}

impl Database {
    pub fn new() -> Self {
        Database {
            records: Vec::new(),
            id_to_index: HashMap::new(),
            store: ColumnStore::new(),
            next_id: 1,
        }
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// INSERT: id=0 なら自動採番。戻り値: 発行されたレコードID
    pub fn insert(&mut self, mut record: Record) -> Result<u64, DatabaseError> {
        if record.id == 0 {
            record.id = self.allocate_id();
        } else if self.id_to_index.contains_key(&record.id) {
            return Err(DatabaseError::DuplicateId(record.id));
        } else if record.id >= self.next_id {
            self.next_id = record.id + 1;
        }

        let id = record.id;
        let row_idx = self.records.len();
        let current_rows = self.store.row_count();

        for (col, val) in &record.columns {
            let column = self.store.columns.entry(col.clone()).or_insert_with(|| {
                vec![None; current_rows]
            });
            column.push(Some(val.clone()));
        }
        let record_cols: Vec<String> = record.columns.keys().cloned().collect();
        for (col, column) in self.store.columns.iter_mut() {
            if !record_cols.contains(col) {
                column.push(None);
            }
        }

        for label in &record.labels {
            self.store.label_index
                .entry(label.clone())
                .or_insert_with(Vec::new)
                .push(row_idx);
        }

        self.id_to_index.insert(id, row_idx);
        self.records.push(Some(record));
        Ok(id)
    }

    /// GET BY ID
    pub fn get(&self, id: u64) -> Result<&Record, DatabaseError> {
        self.id_to_index
            .get(&id)
            .and_then(|&idx| self.records.get(idx))
            .and_then(|r| r.as_ref())
            .ok_or(DatabaseError::RecordNotFound(id))
    }

    /// GET BY LABEL: ラベル名で複数レコードを取得
    pub fn get_by_label(&self, label: &str) -> Vec<&Record> {
        self.store.label_index
            .get(label)
            .map(|indices| {
                indices.iter()
                    .filter_map(|&idx| self.records.get(idx).and_then(|r| r.as_ref()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// UPDATE: 指定IDのレコードを更新する
    pub fn update(&mut self, id: u64, new_record: Record) -> Result<(), DatabaseError> {
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let old_labels: Vec<String> = self.records[row_idx].as_ref()
            .map(|r| r.labels.clone())
            .unwrap_or_default();
        for label in &old_labels {
            if let Some(indices) = self.store.label_index.get_mut(label) {
                indices.retain(|&i| i != row_idx);
            }
        }

        let current_rows = self.records.len();
        for (col, val) in &new_record.columns {
            let column = self.store.columns.entry(col.clone()).or_insert_with(|| {
                vec![None; current_rows]
            });
            if let Some(cell) = column.get_mut(row_idx) {
                *cell = Some(val.clone());
            }
        }

        for label in &new_record.labels {
            self.store.label_index
                .entry(label.clone())
                .or_insert_with(Vec::new)
                .push(row_idx);
        }

        let mut updated = new_record;
        updated.id = id;
        self.records[row_idx] = Some(updated);
        Ok(())
    }

    /// DELETE: 論理削除（None に置き換え）
    pub fn delete(&mut self, id: u64) -> Result<(), DatabaseError> {
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let labels: Vec<String> = self.records[row_idx].as_ref()
            .map(|r| r.labels.clone())
            .unwrap_or_default();
        for label in &labels {
            if let Some(indices) = self.store.label_index.get_mut(label) {
                indices.retain(|&i| i != row_idx);
            }
        }

        for column in self.store.columns.values_mut() {
            if let Some(cell) = column.get_mut(row_idx) {
                *cell = None;
            }
        }

        self.records[row_idx] = None;
        self.id_to_index.remove(&id);
        Ok(())
    }

    /// LIST ALL: 有効な全レコードを返す
    pub fn list_all(&self) -> Vec<&Record> {
        self.records.iter().filter_map(|r| r.as_ref()).collect()
    }

    /// SEARCH BY COLUMN: カラム名と値で絞り込み検索
    pub fn search_by_column(&self, column: &str, value: &DataType) -> Vec<&Record> {
        self.records.iter()
            .filter_map(|r| r.as_ref())
            .filter(|r| r.columns.get(column) == Some(value))
            .collect()
    }

    /// 有効なレコード数
    pub fn count(&self) -> usize {
        self.records.iter().filter(|r| r.is_some()).count()
    }

    /// ATTACH LABEL: 既存レコードにラベルを追加する（重複は無視）
    pub fn attach_label(&mut self, id: u64, label: impl Into<String>) -> Result<(), DatabaseError> {
        let label = label.into();
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let record = self.records[row_idx].as_mut().unwrap();
        // 既に同じラベルがあれば何もしない
        if record.labels.contains(&label) {
            return Ok(());
        }
        record.labels.push(label.clone());

        // ラベルインデックスも更新
        self.store.label_index
            .entry(label)
            .or_insert_with(Vec::new)
            .push(row_idx);

        Ok(())
    }

    /// DETACH LABEL: 既存レコードからラベルを削除する
    pub fn detach_label(&mut self, id: u64, label: &str) -> Result<bool, DatabaseError> {
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let record = self.records[row_idx].as_mut().unwrap();
        let before = record.labels.len();
        record.labels.retain(|l| l != label);
        let removed = record.labels.len() < before;

        // ラベルインデックスからも削除
        if removed {
            if let Some(indices) = self.store.label_index.get_mut(label) {
                indices.retain(|&i| i != row_idx);
            }
        }

        // 削除できたかどうかを返す（true=削除した、false=そのラベルは存在しなかった）
        Ok(removed)
    }

    /// LIST ALL LABELS: DB全体で使われているラベルの一覧（重複なし・ソート済み）
    pub fn list_all_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self.store.label_index
            .iter()
            .filter(|(_, indices)| indices.iter().any(|&i| self.records.get(i).and_then(|r| r.as_ref()).is_some()))
            .map(|(label, _)| label.clone())
            .collect();
        labels.sort();
        labels
    }
}

impl Default for Database {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// 永続化（Persistence）
// ---------------------------------------------------------------------------

/// 永続化操作で発生するエラー
#[derive(Debug)]
pub enum PersistError {
    /// ファイルI/Oエラー
    Io(std::io::Error),
    /// JSONシリアライズ/デシリアライズエラー
    Json(serde_json::Error),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Io(e)   => write!(f, "IO error: {}", e),
            PersistError::Json(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl From<std::io::Error> for PersistError {
    fn from(e: std::io::Error) -> Self { PersistError::Io(e) }
}

impl From<serde_json::Error> for PersistError {
    fn from(e: serde_json::Error) -> Self { PersistError::Json(e) }
}

/// JSONファイルに保存するスナップショット形式
/// （内部のインデックス類は Record から再構築するため保存しない）
#[derive(Serialize, Deserialize)]
struct DatabaseSnapshot {
    /// 有効なレコードのみ（論理削除済みは含まない）
    records: Vec<Record>,
    /// 次のID採番カウンタ
    next_id: u64,
}

impl Database {
    /// データベースの内容を JSON ファイルへ保存する
    ///
    /// # 引数
    /// * `path` - 保存先ファイルパス（存在しない場合は新規作成、既存は上書き）
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), PersistError> {
        let snapshot = DatabaseSnapshot {
            records: self.records.iter().filter_map(|r| r.clone()).collect(),
            next_id: self.next_id,
        };
        // アトミック書き込み: 一時ファイルに書いてからリネーム
        let path = path.as_ref();
        let tmp_path = path.with_extension("json.tmp");
        let file = std::fs::File::create(&tmp_path)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(writer, &snapshot)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// JSON ファイルからデータベースを復元する
    ///
    /// # 引数
    /// * `path` - 読み込むファイルパス
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, PersistError> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let snapshot: DatabaseSnapshot = serde_json::from_reader(reader)?;

        // 空のDBを作り、スナップショットのレコードを順に insert して
        // インデックス類（id_to_index, ColumnStore, label_index）を再構築する
        let mut db = Database::new();
        db.next_id = snapshot.next_id;
        for record in snapshot.records {
            // next_id は既に設定済みなので id を上書きしないよう手動 insert
            let id = record.id;
            let row_idx = db.records.len();
            let current_rows = db.store.row_count();

            for (col, val) in &record.columns {
                let column = db.store.columns
                    .entry(col.clone())
                    .or_insert_with(|| vec![None; current_rows]);
                column.push(Some(val.clone()));
            }
            let record_cols: Vec<String> = record.columns.keys().cloned().collect();
            for (col, column) in db.store.columns.iter_mut() {
                if !record_cols.contains(col) {
                    column.push(None);
                }
            }
            for label in &record.labels {
                db.store.label_index
                    .entry(label.clone())
                    .or_insert_with(Vec::new)
                    .push(row_idx);
            }
            db.id_to_index.insert(id, row_idx);
            db.records.push(Some(record));
        }
        Ok(db)
    }

    /// JSON ファイルが存在すればロード、なければ新規DBを返す
    pub fn load_or_new(path: impl AsRef<std::path::Path>) -> Result<Self, PersistError> {
        let path = path.as_ref();
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::new())
        }
    }
}

#[cfg(test)]
mod tests;

