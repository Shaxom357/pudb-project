// db_engine/src/lib.rs
// 列指向ストレージ（Column-Oriented Storage）＋ラベル検索エンジン

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use serde::{Deserialize, Serialize};

pub mod codec;
pub mod crypto;
pub mod kdb_store;
pub mod memory;

pub use memory::{MemorySizeSpec, MemoryPolicy, MemoryStats};

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
    /// レコードに既に付与されているラベルを重複して付与しようとした
    DuplicateLabel(String),
    /// 既にそのカラムに二次インデックスが存在する
    IndexAlreadyExists(String),
    /// そのカラムに二次インデックスが存在しない
    IndexNotFound(String),
    /// 指定したラベル（またはラベル内の指定カラム）にスキーマ定義が存在しない
    SchemaNotFound(String),
    /// INSERT/UPDATE がスキーマ制約（型・NOT NULL・UNIQUE）に違反した
    SchemaViolation(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::RecordNotFound(id)  => write!(f, "Record id={} not found", id),
            DatabaseError::DuplicateId(id)     => write!(f, "Record id={} already exists", id),
            DatabaseError::ColumnNotFound(col) => write!(f, "Column '{}' not found", col),
            DatabaseError::DuplicateLabel(label) =>
                write!(f, "Label '{}' is already attached to this record", label),
            DatabaseError::IndexAlreadyExists(col) =>
                write!(f, "Index on column '{}' already exists", col),
            DatabaseError::IndexNotFound(col) =>
                write!(f, "Index on column '{}' does not exist", col),
            DatabaseError::SchemaNotFound(what) =>
                write!(f, "Schema not found: {}", what),
            DatabaseError::SchemaViolation(msg) =>
                write!(f, "Schema violation: {}", msg),
        }
    }
}

/// labels 内に重複するラベル名が無いかを検査し、最初に見つかった重複ラベル名を返す
fn find_duplicate_label(labels: &[String]) -> Option<&String> {
    let mut seen = std::collections::HashSet::new();
    labels.iter().find(|l| !seen.insert(l.as_str()))
}

/// ラベル→行番号の索引を保持する内部構造体（カラム値の実体は`Database.records`が
/// 唯一の保持元。以前はここにも列ごとの値を複製して持っていたが、二重管理になっており
/// オンメモリ容量制限機能の実装の妨げにもなるため廃止した）。
#[derive(Debug, Default)]
struct ColumnStore {
    label_index: HashMap<String, Vec<usize>>,
}

impl ColumnStore {
    fn new() -> Self {
        ColumnStore { label_index: HashMap::new() }
    }
}

/// オンメモリ容量制限（`MemoryPolicy.all_in_memory == false`）のときに使う、
/// 内部可変なキャッシュ状態。`Database` の公開メソッドの `&self`/`&mut self` シグネチャを
/// 変えずに、参照系メソッドからも「`.kdb` から読み直してメモリに載せる／FIFO で追い出す」を
/// 行えるようにするため `Mutex` で包んで持つ。
///
/// 不変条件:
/// - `all_in_memory == true`: `cols` は空。列の実体は `records[idx].columns` にある（高速パス。
///   このキャッシュには一切触れない）。
/// - `all_in_memory == false`: `records[idx].columns` は常に空。メモリに載っている行の列は
///   必ず `cols[idx]` にあり、載っていない行は `.kdb` から読み直す。合計サイズは上限以下。
#[derive(Debug, Default)]
struct MemCache {
    /// 行番号 → 列データ（オンメモリ容量制限モードでの唯一の実体）
    cols: HashMap<usize, HashMap<String, DataType>>,
    /// `cols` に載っている列データの推定バイト数の合計
    resident_bytes: u64,
    /// 立ち退き順を決めるための単調増加カウンタ
    tick: u64,
    /// tick → 行番号（昇順＝古い順。先頭が最も長く使われていない）
    order: BTreeMap<u64, usize>,
    /// 行番号 → 現在の tick（`order` からの逆引き）
    pos: HashMap<usize, u64>,
}

impl MemCache {
    /// 行 `idx` を「今使った」ものとして立ち退き順の最後尾へ移動する（再アクセスでLRU更新）。
    fn touch(&mut self, idx: usize) {
        if let Some(old) = self.pos.remove(&idx) {
            self.order.remove(&old);
        }
        self.tick += 1;
        self.order.insert(self.tick, idx);
        self.pos.insert(idx, self.tick);
    }

    /// 行 `idx` の列データを載せる（`touch` も行う）。
    fn insert(&mut self, idx: usize, cols: HashMap<String, DataType>) {
        if let Some(old) = self.cols.remove(&idx) {
            self.resident_bytes = self.resident_bytes.saturating_sub(estimate_columns_bytes(&old));
        }
        self.resident_bytes += estimate_columns_bytes(&cols);
        self.cols.insert(idx, cols);
        self.touch(idx);
    }

    /// 行 `idx` をメモリから外す。
    fn evict_one(&mut self, idx: usize) {
        if let Some(old) = self.cols.remove(&idx) {
            self.resident_bytes = self.resident_bytes.saturating_sub(estimate_columns_bytes(&old));
        }
        if let Some(t) = self.pos.remove(&idx) {
            self.order.remove(&t);
        }
    }

    /// `budget` バイト以下になるまで、最も長く使われていない行から外す。
    fn evict_until_within(&mut self, budget: u64) {
        while self.resident_bytes > budget {
            let Some((&t, &victim)) = self.order.iter().next() else { break };
            self.order.remove(&t);
            self.pos.remove(&victim);
            if let Some(old) = self.cols.remove(&victim) {
                self.resident_bytes = self.resident_bytes.saturating_sub(estimate_columns_bytes(&old));
            }
        }
    }
}

/// 列データの推定メモリ使用量（バイト）。`HashMap` の内部オーバーヘッドや `String` の
/// ヒープ確保分をおおよそ見積もる（厳密なRSSではなく、上限判定に使う目安）。
fn estimate_columns_bytes(cols: &HashMap<String, DataType>) -> u64 {
    let mut total = 0u64;
    for (k, v) in cols {
        total += k.len() as u64 + 24 + 16; // キー文字列 + String構造体 + slotオーバーヘッド概算
        total += match v {
            DataType::Text(s) => s.len() as u64 + 24,
            DataType::Integer(_) | DataType::Float(_) => 8,
            DataType::Boolean(_) => 1,
            DataType::Null => 0,
        };
    }
    total
}

/// 二次インデックス（あるカラムの値 → そのカラムがその値を持つ行番号一覧）。
/// `label_index` と同じ発想で、キーがラベル名の代わりにカラムの値になったもの。
/// ラベルをまたいで、そのカラム名を持つ全レコードが対象になる（スキーマレスな
/// 設計に合わせ、インデックスは「ラベル単位」ではなく「カラム単位」で持つ）。
/// `NULL`・未設定の値は対象外（等値検索のみ対応。IS NULL の高速化は対象外）。
#[derive(Debug, Default)]
struct SecondaryIndex {
    entries: HashMap<String, Vec<usize>>,
}

/// カラムの値を二次インデックスのキー用に正規化する（`Text("1")` と `Integer(1)` を
/// 別物として区別するため、型ごとにタグを付ける）。sql_engine 側の GROUP BY キー生成
/// （`group_key`）と同じ発想だが、クレートが違う（db_engine → sql_engine の一方向依存）ため
/// 個別に実装している。
fn index_value_key(v: &DataType) -> String {
    match v {
        DataType::Text(s)    => format!("T:{}", s),
        DataType::Integer(n) => format!("I:{}", n),
        DataType::Float(f)   => format!("F:{}", f),
        DataType::Boolean(b) => format!("B:{}", b),
        DataType::Null       => "\u{0}NULL".to_string(),
    }
}

/// 二次インデックスへ1行分の値を追加する（`NULL` は対象外）。対象カラムにインデックスが
/// 無ければ何もしない。
fn secondary_index_add(indexes: &mut HashMap<String, SecondaryIndex>, col: &str, val: &DataType, row_idx: usize) {
    if matches!(val, DataType::Null) { return; }
    if let Some(idx) = indexes.get_mut(col) {
        idx.entries.entry(index_value_key(val)).or_default().push(row_idx);
    }
}

/// 二次インデックスから1行分の値を除去する（UPDATE/DELETE で古い値を外すのに使う）。
fn secondary_index_remove(indexes: &mut HashMap<String, SecondaryIndex>, col: &str, val: &DataType, row_idx: usize) {
    if matches!(val, DataType::Null) { return; }
    if let Some(idx) = indexes.get_mut(col) {
        if let Some(v) = idx.entries.get_mut(&index_value_key(val)) {
            v.retain(|&i| i != row_idx);
        }
    }
}

// ---------------------------------------------------------------------------
// 任意スキーマ層（ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA）
// ---------------------------------------------------------------------------

/// カラムに宣言できる型（`DataType` の NULL を除く4種に対応）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaType {
    Text,
    Integer,
    Float,
    Boolean,
}

/// 値が宣言された型に適合するか（`Integer` の値は `Float` 宣言にも適合させる。
/// 桁落ちする逆方向＝`Float`の値を`Integer`宣言に、は適合させない）
fn value_matches_type(v: &DataType, ty: SchemaType) -> bool {
    matches!((v, ty),
        (DataType::Text(_), SchemaType::Text)
        | (DataType::Integer(_), SchemaType::Integer)
        | (DataType::Float(_), SchemaType::Float)
        | (DataType::Integer(_), SchemaType::Float)
        | (DataType::Boolean(_), SchemaType::Boolean)
    )
}

/// 1カラムぶんのスキーマ定義（`ALTER LABEL ... DEFINE COLUMN` 1回に対応）
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSchema {
    pub ty: SchemaType,
    pub not_null: bool,
    pub default: Option<DataType>,
    pub unique: bool,
}

/// 1ラベルぶんのスキーマ（列挙したカラム定義 + 強制の有効/無効）。
/// `enabled` の既定値は `false`：`DEFINE COLUMN` しただけでは強制されず、
/// `ENABLE SCHEMA` して初めて INSERT/UPDATE で検証されるようになる
/// （定義してすぐ強制されて既存データが壊れる、という事故を避けるため）。
#[derive(Debug, Clone, Default)]
struct LabelSchema {
    columns: HashMap<String, ColumnSchema>,
    enabled: bool,
}

/// DBエンジン本体
pub struct Database {
    records: Vec<Option<Record>>,
    id_to_index: HashMap<u64, usize>,
    store: ColumnStore,
    next_id: u64,
    /// カラム名 → 二次インデックス（`CREATE INDEX` で作成。永続化はせず、
    /// 起動時に呼び出し側が定義を読み直して `create_index` で再構築する想定）
    secondary_indexes: HashMap<String, SecondaryIndex>,
    /// ラベル名 → 任意スキーマ定義（同様に永続化はせず、呼び出し側が定義を読み直して
    /// `define_column`/`enable_schema` で再構築する想定）
    schemas: HashMap<String, LabelSchema>,
    /// スキーマ強制の全体スイッチ（既定 `true`＝各ラベルの `enabled` 設定を尊重する）。
    /// `false` にすると、個々のラベルの `ENABLE SCHEMA` 状態に関わらず一時的に全ての
    /// スキーマ検証を止められる（大量バックフィル時の緊急退避用）。サーバー再起動のたびに
    /// `true` へ戻り、意図せず無効なままになることを防ぐ（`.kdb`/JSON・サイドカーどちらにも
    /// 永続化しない）。
    schema_enforcement_enabled: bool,
    /// レコードID → `.kdb`上の最新エントリのバイトオフセット（`.kdb`利用時のみ設定）。
    /// オンメモリ容量制限で追い出したレコードを`.kdb`から単体で読み直すために使う基盤。
    /// 永続化はせず、`load_kdb`・`insert_fast`のWAL追記・呼び出し側が行った`compact`の
    /// 結果（`sync_record_offsets`）から都度反映する。
    record_offsets: HashMap<u64, u64>,
    /// `.kdb` ファイルのパス（`load_kdb`/`load_or_new_kdb` で設定。オンメモリ容量制限で
    /// 追い出した行をこのファイルから読み直す）。JSON モード／純メモリ運用時は `None`。
    db_path: Option<String>,
    /// DB全体で使われたことのあるカラム名の集合（`list_all_columns` を全件走査せず返すため。
    /// 削除はしない＝行が全部消えてもカラム名は残る。旧 `ColumnStore.columns` と同じ挙動）。
    all_columns: std::collections::HashSet<String>,
    /// 「全データオンメモリ」設定（既定 = 有効・無制限）
    memory_policy: MemoryPolicy,
    /// 行番号 → 「`records[idx].columns` が完全か」。`true` の行はインラインの列を
    /// そのまま使う（高速パス）。`false` の行は列を持っておらず、`mem` または `.kdb` から得る。
    loaded: Vec<bool>,
    /// オンメモリ容量制限モードのキャッシュ状態（内部可変）
    mem: Mutex<MemCache>,
}

impl Database {
    pub fn new() -> Self {
        Database {
            records: Vec::new(),
            id_to_index: HashMap::new(),
            store: ColumnStore::new(),
            next_id: 1,
            secondary_indexes: HashMap::new(),
            schemas: HashMap::new(),
            schema_enforcement_enabled: true,
            record_offsets: HashMap::new(),
            db_path: None,
            all_columns: std::collections::HashSet::new(),
            memory_policy: MemoryPolicy::default(),
            loaded: Vec::new(),
            mem: Mutex::new(MemCache::default()),
        }
    }

    fn mem_lock(&self) -> std::sync::MutexGuard<'_, MemCache> {
        self.mem.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn resolve_budget(&self) -> u64 {
        self.memory_policy.limit.resolve_bytes().unwrap_or(u64::MAX)
    }

    /// 「全データオンメモリ」設定を取得する
    pub fn memory_policy(&self) -> MemoryPolicy {
        self.memory_policy
    }

    /// 現在のオンメモリ状況（観測用）
    pub fn memory_stats(&self) -> MemoryStats {
        let mem = self.mem_lock();
        let total_rows = self.records.iter().filter(|r| r.is_some()).count();
        let resident_rows = if self.memory_policy.all_in_memory {
            self.loaded.iter().zip(self.records.iter())
                .filter(|(l, r)| **l && r.is_some()).count()
        } else {
            mem.cols.len()
        };
        MemoryStats {
            all_in_memory: self.memory_policy.all_in_memory,
            limit_bytes: self.memory_policy.limit.resolve_bytes(),
            resident_bytes: mem.resident_bytes,
            resident_rows,
            total_rows,
        }
    }

    /// 「全データオンメモリ」設定を切り替える。
    /// - 有効→無効: 現在インラインで保持している列を退避対象へ移し、上限バイト数まで
    ///   FIFO/LRU で削る。
    /// - 無効→有効: 退避してあった列をインラインへ戻す。既に追い出されている行は
    ///   `.kdb` 依存のまま（アクセス時に読み直す）＝一括ロードはしない。
    pub fn set_memory_policy(&mut self, mut policy: MemoryPolicy) {
        // 退avした行を読み直すランダムアクセス機構は `.kdb` にしかない。`.kdb` バックエンドが
        // 無い（JSONモード・純メモリ運用）場合、容量制限モードにすると追い出した行が失われて
        // しまうため、強制的に「全データオンメモリ有効」に倒す。
        if self.db_path.is_none() {
            policy.all_in_memory = true;
        }
        let was_all_in_memory = self.memory_policy.all_in_memory;
        self.memory_policy = policy;
        let budget = self.resolve_budget();

        if !policy.all_in_memory {
            let n = self.records.len();
            let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            for idx in 0..n {
                if self.loaded.get(idx).copied().unwrap_or(false) {
                    if let Some(Some(rec)) = self.records.get_mut(idx) {
                        let cols = std::mem::take(&mut rec.columns);
                        mem.insert(idx, cols);
                    }
                    if let Some(f) = self.loaded.get_mut(idx) { *f = false; }
                }
            }
            mem.evict_until_within(budget);
        } else if !was_all_in_memory {
            let entries: Vec<(usize, HashMap<String, DataType>)> = {
                let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
                let taken: Vec<(usize, HashMap<String, DataType>)> = mem.cols.drain().collect();
                mem.resident_bytes = 0;
                mem.order.clear();
                mem.pos.clear();
                taken
            };
            for (idx, cols) in entries {
                if let Some(Some(rec)) = self.records.get_mut(idx) {
                    rec.columns = cols;
                    if let Some(f) = self.loaded.get_mut(idx) { *f = true; }
                }
            }
        }
    }

    /// 行 `row_idx` の完全なレコードを組み立てて返す（列がメモリに無ければ `.kdb` から
    /// 読み直してメモリへ載せ、必要なら FIFO/LRU で古い行を追い出す）。
    /// 論理削除済み・範囲外なら `None`。
    fn hydrate_row(&self, row_idx: usize) -> Option<Record> {
        let slot = self.records.get(row_idx)?.as_ref()?;
        let mut rec = Record {
            id: slot.id,
            columns: HashMap::new(),
            labels: slot.labels.clone(),
        };
        if self.loaded.get(row_idx).copied().unwrap_or(false) {
            rec.columns = slot.columns.clone();
            return Some(rec);
        }
        let id = slot.id;
        {
            let mut mem = self.mem_lock();
            if let Some(cols) = mem.cols.get(&row_idx).cloned() {
                mem.touch(row_idx);
                rec.columns = cols;
                return Some(rec);
            }
        }
        // メモリに無い → `.kdb` から単体で読み直す
        let (Some(offset), Some(path)) = (self.record_offsets.get(&id).copied(), self.db_path.as_deref())
        else {
            return Some(rec); // `.kdb` バックエンドが無い／オフセット未記録なら、そのまま（列なし）
        };
        let loaded_cols = match kdb_store::KdbFile::open(path)
            .and_then(|mut k| k.read_record_at(offset))
        {
            Ok(r) => r.columns,
            Err(_) => return Some(rec),
        };
        rec.columns = loaded_cols.clone();
        let budget = self.resolve_budget();
        let mut mem = self.mem_lock();
        mem.insert(row_idx, loaded_cols);
        if !self.memory_policy.all_in_memory {
            mem.evict_until_within(budget);
        }
        Some(rec)
    }

    /// 行 `row_idx`（既に `self.records`/`self.loaded` に確保済み）の列データを、
    /// 現在の「全データオンメモリ」設定に従って配置する。
    fn place_columns(&mut self, row_idx: usize, cols: HashMap<String, DataType>) {
        for k in cols.keys() {
            self.all_columns.insert(k.clone());
        }
        if self.memory_policy.all_in_memory {
            {
                let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
                mem.evict_one(row_idx);
            }
            if let Some(Some(rec)) = self.records.get_mut(row_idx) {
                rec.columns = cols;
            }
            if let Some(f) = self.loaded.get_mut(row_idx) { *f = true; }
        } else {
            if let Some(Some(rec)) = self.records.get_mut(row_idx) {
                rec.columns = HashMap::new();
            }
            if let Some(f) = self.loaded.get_mut(row_idx) { *f = false; }
            let budget = self.resolve_budget();
            let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            mem.insert(row_idx, cols);
            mem.evict_until_within(budget);
        }
    }

    /// レコードIDに対応する`.kdb`上のバイトオフセットを記録する。
    pub fn set_record_offset(&mut self, id: u64, offset: u64) {
        self.record_offsets.insert(id, offset);
    }

    /// 外部で`KdbFile::compact`を実行した後、渡した`records`と同じ順序の`offsets`から
    /// オフセット索引を一括で反映する（`db_client`の`auto_save`・`db_ffi`のセッション保存など、
    /// `Database`の外で`.kdb`への書き直しを行う箇所から呼ぶ）。
    pub fn sync_record_offsets(&mut self, records: &[Record], offsets: &[u64]) {
        for (r, &offset) in records.iter().zip(offsets.iter()) {
            self.record_offsets.insert(r.id, offset);
        }
    }

    /// レコードIDに対応する`.kdb`上のバイトオフセット（未記録なら`None`）
    pub fn record_offset(&self, id: u64) -> Option<u64> {
        self.record_offsets.get(&id).copied()
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// INSERT: id=0 なら自動採番。戻り値: 発行されたレコードID
    pub fn insert(&mut self, mut record: Record) -> Result<u64, DatabaseError> {
        if let Some(dup) = find_duplicate_label(&record.labels) {
            return Err(DatabaseError::DuplicateLabel(dup.clone()));
        }
        self.apply_schema(&mut record)?;
        self.check_unique_constraints(&record, None)?;
        if record.id == 0 {
            record.id = self.allocate_id();
        } else if self.id_to_index.contains_key(&record.id) {
            return Err(DatabaseError::DuplicateId(record.id));
        } else if record.id >= self.next_id {
            self.next_id = record.id + 1;
        }

        let id = record.id;
        let row_idx = self.records.len();

        for label in &record.labels {
            self.store.label_index
                .entry(label.clone())
                .or_insert_with(Vec::new)
                .push(row_idx);
        }
        for (col, val) in &record.columns {
            secondary_index_add(&mut self.secondary_indexes, col, val, row_idx);
        }

        self.id_to_index.insert(id, row_idx);
        let cols = std::mem::take(&mut record.columns);
        self.records.push(Some(record));
        self.loaded.push(false);
        self.place_columns(row_idx, cols);
        Ok(id)
    }

    /// GET BY ID
    pub fn get(&self, id: u64) -> Result<Record, DatabaseError> {
        let &idx = self.id_to_index.get(&id).ok_or(DatabaseError::RecordNotFound(id))?;
        self.hydrate_row(idx).ok_or(DatabaseError::RecordNotFound(id))
    }

    /// GET BY LABEL: ラベル名で複数レコードを取得
    pub fn get_by_label(&self, label: &str) -> Vec<Record> {
        self.store.label_index
            .get(label)
            .map(|indices| {
                indices.iter()
                    .filter_map(|&idx| self.hydrate_row(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// UPDATE: 指定IDのレコードを更新する
    pub fn update(&mut self, id: u64, mut new_record: Record) -> Result<(), DatabaseError> {
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;
        self.apply_schema(&mut new_record)?;
        self.check_unique_constraints(&new_record, Some(id))?;

        let old_record = self.hydrate_row(row_idx);
        let old_labels: Vec<String> = old_record.as_ref()
            .map(|r| r.labels.clone())
            .unwrap_or_default();
        for label in &old_labels {
            if let Some(indices) = self.store.label_index.get_mut(label) {
                indices.retain(|&i| i != row_idx);
            }
        }

        // 二次インデックスを更新するため、上書きする前に旧い値と比較する
        // （値が変わった／消えたカラムだけ古いインデックスエントリを外す）。
        if let Some(old) = &old_record {
            for (col, old_val) in &old.columns {
                let unchanged = new_record.columns.get(col).is_some_and(|v| v == old_val);
                if !unchanged {
                    secondary_index_remove(&mut self.secondary_indexes, col, old_val, row_idx);
                }
            }
        }
        for (col, val) in &new_record.columns {
            let unchanged = old_record.as_ref()
                .and_then(|o| o.columns.get(col))
                .is_some_and(|old_val| old_val == val);
            if !unchanged {
                secondary_index_add(&mut self.secondary_indexes, col, val, row_idx);
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
        let cols = std::mem::take(&mut updated.columns);
        self.records[row_idx] = Some(updated);
        self.place_columns(row_idx, cols);
        Ok(())
    }

    /// DELETE: 論理削除（None に置き換え）
    pub fn delete(&mut self, id: u64) -> Result<(), DatabaseError> {
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let old = self.hydrate_row(row_idx);
        self.records[row_idx] = None;
        if let Some(old) = &old {
            for label in &old.labels {
                if let Some(indices) = self.store.label_index.get_mut(label) {
                    indices.retain(|&i| i != row_idx);
                }
            }
            for (col, val) in &old.columns {
                secondary_index_remove(&mut self.secondary_indexes, col, val, row_idx);
            }
        }

        self.id_to_index.remove(&id);
        self.record_offsets.remove(&id);
        self.mem_lock().evict_one(row_idx);
        if let Some(f) = self.loaded.get_mut(row_idx) { *f = false; }
        Ok(())
    }

    /// LIST ALL: 有効な全レコードを返す
    pub fn list_all(&self) -> Vec<Record> {
        (0..self.records.len()).filter_map(|idx| self.hydrate_row(idx)).collect()
    }

    /// SEARCH BY COLUMN: カラム名と値で絞り込み検索
    pub fn search_by_column(&self, column: &str, value: &DataType) -> Vec<Record> {
        (0..self.records.len())
            .filter_map(|idx| self.hydrate_row(idx))
            .filter(|r| r.columns.get(column) == Some(value))
            .collect()
    }

    /// 有効なレコード数
    pub fn count(&self) -> usize {
        self.records.iter().filter(|r| r.is_some()).count()
    }

    /// ATTACH LABEL: 既存レコードにラベルを追加する（重複付与は禁止。既に付与済みならエラー）
    pub fn attach_label(&mut self, id: u64, label: impl Into<String>) -> Result<(), DatabaseError> {
        let label = label.into();
        let &row_idx = self.id_to_index.get(&id)
            .ok_or(DatabaseError::RecordNotFound(id))?;

        let record = self.records[row_idx].as_mut().unwrap();
        // 既に同じラベルがある場合は変更せずエラーを返す（ラベルの重複付与は禁止）
        if record.labels.contains(&label) {
            return Err(DatabaseError::DuplicateLabel(label));
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

    /// DB全体で使われているカラム名の一覧（重複なし・ソート済み）。
    /// 走査を避けるため `all_columns`（INSERT/UPDATE 時に加算、削除はしない）から返す。
    pub fn list_all_columns(&self) -> Vec<String> {
        let mut cols: Vec<String> = self.all_columns.iter().cloned().collect();
        cols.sort();
        cols
    }

    /// 論理削除済みを含む全スロット数（アロケート済み行数）
    pub fn slot_count(&self) -> usize {
        self.records.len()
    }

    /// 次に採番されるID
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// 論理削除されたレコード数
    pub fn deleted_count(&self) -> usize {
        self.records.iter().filter(|r| r.is_none()).count()
    }

    // -----------------------------------------------------------------
    // 二次インデックス（CREATE INDEX / DROP INDEX）
    // -----------------------------------------------------------------

    /// `column` に対する等値検索用の二次インデックスを作成する。
    /// 既存の全レコード（論理削除済みを除く）を1回走査して構築する
    /// （`label_index` と同様、中身は永続化せず必要に応じて再構築する想定）。
    /// 既にそのカラムにインデックスが存在する場合はエラーを返す。
    pub fn create_index(&mut self, column: &str) -> Result<(), DatabaseError> {
        if self.secondary_indexes.contains_key(column) {
            return Err(DatabaseError::IndexAlreadyExists(column.to_string()));
        }
        let mut index = SecondaryIndex::default();
        for row_idx in 0..self.records.len() {
            if let Some(r) = self.hydrate_row(row_idx) {
                if let Some(v) = r.columns.get(column) {
                    if !matches!(v, DataType::Null) {
                        index.entries.entry(index_value_key(v)).or_default().push(row_idx);
                    }
                }
            }
        }
        self.secondary_indexes.insert(column.to_string(), index);
        Ok(())
    }

    /// `column` の二次インデックスを削除する。存在しない場合はエラーを返す。
    pub fn drop_index(&mut self, column: &str) -> Result<(), DatabaseError> {
        if self.secondary_indexes.remove(column).is_none() {
            return Err(DatabaseError::IndexNotFound(column.to_string()));
        }
        Ok(())
    }

    /// `column` に二次インデックスが存在するか
    pub fn has_index(&self, column: &str) -> bool {
        self.secondary_indexes.contains_key(column)
    }

    /// インデックスが張られているカラム名の一覧（ソート済み）
    pub fn list_indexes(&self) -> Vec<String> {
        let mut cols: Vec<String> = self.secondary_indexes.keys().cloned().collect();
        cols.sort();
        cols
    }

    /// 二次インデックスを使って `column = value` に一致するレコードを取得する。
    /// `column` にインデックスが無ければ `None`（呼び出し側は全件走査にフォールバックする）。
    /// インデックスはあるが一致するレコードが無ければ `Some(vec![])`。
    pub fn get_by_index(&self, column: &str, value: &DataType) -> Option<Vec<Record>> {
        let index = self.secondary_indexes.get(column)?;
        let key = index_value_key(value);
        let indices: Vec<usize> = index.entries.get(&key).cloned().unwrap_or_default();
        Some(indices.iter().filter_map(|&i| self.hydrate_row(i)).collect())
    }

    // -----------------------------------------------------------------
    // 任意スキーマ層（ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA）
    // -----------------------------------------------------------------

    /// `label` の `column` にスキーマを定義する（既存の定義は上書きする）。
    /// `unique: true` の場合、まだ二次インデックスが無ければ自動的に作成する
    /// （UNIQUE 制約のチェックを O(1) に近い速度で行うため。既存のインデックスが
    /// あればそれをそのまま流用する）。定義しただけではまだ強制されない
    /// （`enable_schema` するまで、そのラベルは今までどおりスキーマレスのまま）。
    pub fn define_column(&mut self, label: &str, column: &str, schema: ColumnSchema) -> Result<(), DatabaseError> {
        if schema.unique && !self.has_index(column) {
            self.create_index(column)?;
        }
        self.schemas.entry(label.to_string()).or_default()
            .columns.insert(column.to_string(), schema);
        Ok(())
    }

    /// `label` の `column` のスキーマ定義を削除する（データそのもの・自動作成された
    /// 二次インデックスは削除しない）。定義が存在しない場合はエラー。
    pub fn drop_column_schema(&mut self, label: &str, column: &str) -> Result<(), DatabaseError> {
        let schema = self.schemas.get_mut(label)
            .ok_or_else(|| DatabaseError::SchemaNotFound(label.to_string()))?;
        if schema.columns.remove(column).is_none() {
            return Err(DatabaseError::SchemaNotFound(format!("{}.{}", label, column)));
        }
        Ok(())
    }

    /// `label` のスキーマ強制を有効化する（スキーマが未定義でも空のスキーマとして
    /// 有効化できる。既存データの整合性は自動チェックしない。事前に `validate_label`
    /// で確認しておくことを推奨する）。
    pub fn enable_schema(&mut self, label: &str) {
        self.schemas.entry(label.to_string()).or_default().enabled = true;
    }

    /// `label` のスキーマ強制を無効化する（定義自体は保持したまま、検証だけ止める）。
    pub fn disable_schema(&mut self, label: &str) {
        self.schemas.entry(label.to_string()).or_default().enabled = false;
    }

    /// `label` のスキーマ定義を取得する（`(強制が有効か, カラム名でソートしたカラム定義一覧)`）。
    /// スキーマが未定義（一度も `define_column`/`enable_schema` していない）なら `None`。
    pub fn describe_label(&self, label: &str) -> Option<(bool, Vec<(String, ColumnSchema)>)> {
        let schema = self.schemas.get(label)?;
        let mut cols: Vec<(String, ColumnSchema)> = schema.columns.iter()
            .map(|(k, v)| (k.clone(), v.clone())).collect();
        cols.sort_by(|a, b| a.0.cmp(&b.0));
        Some((schema.enabled, cols))
    }

    /// スキーマが定義されているラベル名の一覧（ソート済み）
    pub fn list_schema_labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self.schemas.keys().cloned().collect();
        labels.sort();
        labels
    }

    /// `label` の現在のデータが、定義済みスキーマ（強制が無効でも定義があれば対象）に
    /// 違反していないかを確認する。違反があれば1件ごとに説明文を返す（空なら違反なし）。
    /// `ENABLE SCHEMA` する前の事前確認に使う想定で、これ自体は何も変更しない。
    pub fn validate_label(&self, label: &str) -> Vec<String> {
        let Some(schema) = self.schemas.get(label) else { return Vec::new(); };
        let mut violations = Vec::new();
        for record in self.get_by_label(label) {
            for (col, def) in &schema.columns {
                match record.columns.get(col) {
                    None => {
                        // DEFAULT はこれから INSERT する行にのみ適用され、既存データを遡って
                        // 埋めるわけではないため、DEFAULT の有無に関わらず現状を報告する。
                        if def.not_null {
                            violations.push(format!("record id={}: column '{}' is missing (NOT NULL)", record.id, col));
                        }
                    }
                    Some(DataType::Null) => {
                        if def.not_null {
                            violations.push(format!("record id={}: column '{}' is NULL (NOT NULL)", record.id, col));
                        }
                    }
                    Some(v) => {
                        if !value_matches_type(v, def.ty) {
                            violations.push(format!(
                                "record id={}: column '{}' has type {:?}, expected {:?}", record.id, col, v, def.ty
                            ));
                        }
                    }
                }
            }
            if let Some(dups) = self.unique_conflicts(label, &record, schema) {
                violations.extend(dups);
            }
        }
        violations
    }

    /// `record` が `schema` の UNIQUE 制約に違反していないかを、同じラベルを持つ
    /// 他のレコードと比較して確認する（`validate_label` 専用の内部ヘルパー）。
    fn unique_conflicts(&self, label: &str, record: &Record, schema: &LabelSchema) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for (col, def) in &schema.columns {
            if !def.unique { continue; }
            let Some(val) = record.columns.get(col) else { continue; };
            if matches!(val, DataType::Null) { continue; }
            let conflict = self.get_by_index(col, val)
                .unwrap_or_default()
                .iter()
                .any(|r| r.id != record.id && r.labels.iter().any(|l| l == label));
            if conflict {
                out.push(format!("record id={}: column '{}' value is not unique", record.id, col));
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// スキーマ強制の全体スイッチを設定する（`false` で全ラベルの検証を一時停止）
    pub fn set_schema_enforcement_enabled(&mut self, enabled: bool) {
        self.schema_enforcement_enabled = enabled;
    }

    /// スキーマ強制の全体スイッチが有効か
    pub fn is_schema_enforcement_enabled(&self) -> bool {
        self.schema_enforcement_enabled
    }

    /// INSERT/UPDATE の前段でスキーマを適用する: `DEFAULT` の補完と、型・NOT NULL の検証を行う。
    /// 強制が無効（全体スイッチ OFF、またはそのラベルが未定義／`DISABLE SCHEMA`）なラベルは
    /// 素通りする。スキーマが1件も定義されていなければ即座に抜ける（無関係なクエリへの
    /// オーバーヘッドをゼロに近づけるため）。
    fn apply_schema(&self, record: &mut Record) -> Result<(), DatabaseError> {
        if !self.schema_enforcement_enabled || self.schemas.is_empty() { return Ok(()); }
        let labels = record.labels.clone();
        for label in &labels {
            let Some(schema) = self.schemas.get(label) else { continue; };
            if !schema.enabled { continue; }
            for (col, def) in &schema.columns {
                match record.columns.get(col) {
                    None => {
                        if let Some(default) = &def.default {
                            record.columns.insert(col.clone(), default.clone());
                        } else if def.not_null {
                            return Err(DatabaseError::SchemaViolation(format!(
                                "label '{}': column '{}' is required (NOT NULL)", label, col
                            )));
                        }
                    }
                    Some(DataType::Null) => {
                        if def.not_null {
                            return Err(DatabaseError::SchemaViolation(format!(
                                "label '{}': column '{}' cannot be NULL", label, col
                            )));
                        }
                    }
                    Some(v) => {
                        if !value_matches_type(v, def.ty) {
                            return Err(DatabaseError::SchemaViolation(format!(
                                "label '{}': column '{}' expects type {:?}, got '{}'", label, col, def.ty, v
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// INSERT/UPDATE の前段で UNIQUE 制約を検証する（`apply_schema` で DEFAULT 補完済みの
    /// `record` に対して行う）。`exclude_id` は UPDATE で自分自身を重複扱いしないために使う。
    fn check_unique_constraints(&self, record: &Record, exclude_id: Option<u64>) -> Result<(), DatabaseError> {
        if !self.schema_enforcement_enabled || self.schemas.is_empty() { return Ok(()); }
        for label in &record.labels {
            let Some(schema) = self.schemas.get(label) else { continue; };
            if !schema.enabled { continue; }
            for (col, def) in &schema.columns {
                if !def.unique { continue; }
                let Some(val) = record.columns.get(col) else { continue; };
                if matches!(val, DataType::Null) { continue; }
                let conflict = match self.get_by_index(col, val) {
                    Some(hits) => hits.iter().any(|r| Some(r.id) != exclude_id && r.labels.iter().any(|l| l == label)),
                    // UNIQUE 指定時に自動でインデックスを作るため通常はここに来ないが、
                    // 保険として全件走査でも確認する
                    None => (0..self.records.len()).filter_map(|i| self.hydrate_row(i))
                        .any(|r| Some(r.id) != exclude_id && r.labels.iter().any(|l| l == label) && r.columns.get(col) == Some(val)),
                };
                if conflict {
                    return Err(DatabaseError::SchemaViolation(format!(
                        "label '{}': column '{}' must be unique, value already exists", label, col
                    )));
                }
            }
        }
        Ok(())
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
            records: self.list_all(),
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

            for label in &record.labels {
                db.store.label_index
                    .entry(label.clone())
                    .or_insert_with(Vec::new)
                    .push(row_idx);
            }
            for col in record.columns.keys() {
                db.all_columns.insert(col.clone());
            }
            db.id_to_index.insert(id, row_idx);
            db.records.push(Some(record));
            db.loaded.push(true);
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

// ---------------------------------------------------------------------------
// .kdb バイナリ暗号化フォーマットの永続化
// ---------------------------------------------------------------------------

/// .kdb 永続化エラー
#[derive(Debug)]
pub enum KdbPersistError {
    Kdb(kdb_store::KdbError),
}
impl std::fmt::Display for KdbPersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { KdbPersistError::Kdb(e) => write!(f, "KDB: {}", e) }
    }
}
impl std::error::Error for KdbPersistError {}
impl From<kdb_store::KdbError> for KdbPersistError {
    fn from(e: kdb_store::KdbError) -> Self { KdbPersistError::Kdb(e) }
}

impl Database {
    /// .kdb ファイルへ保存（コンパクション: 全レコード書き直し）
    /// UPDATE/DELETE 後に呼ぶ
    pub fn save_kdb(&mut self, path: &str) -> Result<(), KdbPersistError> {
        let mut kdb = kdb_store::KdbFile::open_or_create(path)?;
        let records: Vec<Record> = self.list_all();
        let offsets = kdb.compact(&records, self.next_id)?;
        self.sync_record_offsets(&records, &offsets);
        Ok(())
    }

    /// .kdb ファイルからロード
    pub fn load_kdb(path: &str) -> Result<Self, KdbPersistError> {
        let mut kdb = kdb_store::KdbFile::open(path)?;
        let entries = kdb.read_all_records()?;
        let mut db  = Database::new();
        db.next_id  = kdb.next_id();
        db.db_path  = Some(path.to_string());
        for (offset, record) in entries {
            let id      = record.id;
            let row_idx = db.records.len();
            for label in &record.labels {
                db.store.label_index.entry(label.clone()).or_default().push(row_idx);
            }
            for col in record.columns.keys() {
                db.all_columns.insert(col.clone());
            }
            db.id_to_index.insert(id, row_idx);
            db.record_offsets.insert(id, offset);
            db.records.push(Some(record));
            db.loaded.push(true);
        }
        Ok(db)
    }

    /// .kdb ファイルが存在すればロード、なければ新規DB（`db_path` は常に設定する）
    pub fn load_or_new_kdb(path: &str) -> Result<Self, KdbPersistError> {
        if std::path::Path::new(path).exists() {
            Self::load_kdb(path)
        } else {
            let mut db = Self::new();
            db.db_path = Some(path.to_string());
            Ok(db)
        }
    }

    /// INSERT を WAL 追記で高速に行う（O(1)書き込み）
    /// kdb が None の場合は通常 insert としてメモリのみ更新
    pub fn insert_fast(
        &mut self,
        mut record: Record,
        kdb: Option<&mut kdb_store::KdbFile>,
    ) -> Result<u64, DatabaseError> {
        if let Some(dup) = find_duplicate_label(&record.labels) {
            return Err(DatabaseError::DuplicateLabel(dup.clone()));
        }
        self.apply_schema(&mut record)?;
        self.check_unique_constraints(&record, None)?;
        // ID採番
        if record.id == 0 {
            record.id = self.next_id;
            self.next_id += 1;
        } else if self.id_to_index.contains_key(&record.id) {
            return Err(DatabaseError::DuplicateId(record.id));
        } else if record.id >= self.next_id {
            self.next_id = record.id + 1;
        }
        let id      = record.id;
        let row_idx = self.records.len();
        for label in &record.labels {
            self.store.label_index.entry(label.clone()).or_default().push(row_idx);
        }
        for (col, val) in &record.columns {
            secondary_index_add(&mut self.secondary_indexes, col, val, row_idx);
        }
        self.id_to_index.insert(id, row_idx);
        // WAL追記（kdb モードなら高速保存）
        if let Some(kdb) = kdb {
            let _ = kdb.update_next_id(self.next_id);
            if let Ok(offset) = kdb.append_record(&record) {
                self.record_offsets.insert(id, offset);
            }
        }
        let cols = std::mem::take(&mut record.columns);
        self.records.push(Some(record));
        self.loaded.push(false);
        self.place_columns(row_idx, cols);
        Ok(id)
    }
}

#[cfg(test)]
mod tests;

