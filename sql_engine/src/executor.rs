// sql_engine/src/executor.rs
// SELECT 文の実行エンジン

use db_engine::{Database, DataType, Record};
use crate::ast::*;

/// クエリ実行結果
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// カラム名のリスト（SELECT * なら全カラム）
    pub columns: Vec<String>,
    /// 行データ（columns と同順）
    pub rows: Vec<Vec<CellValue>>,
    /// LIMIT前のヒット件数
    pub total_matched: usize,
}

/// セル値（NULL対応）
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

impl CellValue {
    pub fn from_data_type(dt: &DataType) -> Self {
        match dt {
            DataType::Text(s)    => CellValue::Text(s.clone()),
            DataType::Integer(n) => CellValue::Integer(*n),
            DataType::Float(f)   => CellValue::Float(*f),
            DataType::Boolean(b) => CellValue::Boolean(*b),
            DataType::Null       => CellValue::Null,
        }
    }
    pub fn display(&self) -> String {
        match self {
            CellValue::Text(s)    => s.clone(),
            CellValue::Integer(n) => n.to_string(),
            CellValue::Float(f)   => f.to_string(),
            CellValue::Boolean(b) => b.to_string(),
            CellValue::Null       => "NULL".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// SELECT 実行
// ---------------------------------------------------------------------------

pub fn execute_select(db: &Database, stmt: &SelectStatement) -> QueryResult {
    let mut records: Vec<&Record> = filter_by_from(db, &stmt.from);

    if let Some(ref where_expr) = stmt.where_clause {
        records.retain(|r| eval_where(r, where_expr));
    }

    let total_matched = records.len();

    if !stmt.order_by.is_empty() { sort_records(&mut records, &stmt.order_by); }
    if let Some(limit) = stmt.limit { records.truncate(limit as usize); }

    // SELECT するカラムを決定
    // SELECT * の場合は id 列を先頭、labels 列（カンマ区切り）を末尾に付与する
    let (display_cols, internal_cols): (Vec<String>, Vec<String>) = match &stmt.columns {
        SelectColumns::All => {
            let mut cols = db.list_all_columns();
            cols.sort();
            let internal = std::iter::once("__id".to_string())
                .chain(cols.iter().cloned())
                .chain(std::iter::once("__labels".to_string()))
                .collect();
            let display = std::iter::once("id".to_string())
                .chain(cols.into_iter())
                .chain(std::iter::once("labels".to_string()))
                .collect();
            (display, internal)
        }
        SelectColumns::Named(names) => (names.clone(), names.clone()),
    };

    let rows: Vec<Vec<CellValue>> = records.iter().map(|r| {
        internal_cols.iter().map(|col| {
            if col == "__id" { CellValue::Integer(r.id as i64) }
            else if col == "__labels" { CellValue::Text(r.labels.join(",")) }
            else { r.columns.get(col).map(CellValue::from_data_type).unwrap_or(CellValue::Null) }
        }).collect()
    }).collect();

    QueryResult { columns: display_cols, rows, total_matched }
}

// ---------------------------------------------------------------------------
// FROM句フィルタ
// ---------------------------------------------------------------------------

fn filter_by_from<'a>(db: &'a Database, from: &FromClause) -> Vec<&'a Record> {
    match from {
        FromClause::Label(t) => filter_by_target(db, t),
        FromClause::And(ts)  => {
            if ts.is_empty() { return vec![]; }
            let mut ids: std::collections::HashSet<u64> =
                filter_by_target(db, &ts[0]).iter().map(|r| r.id).collect();
            for t in &ts[1..] {
                let next: std::collections::HashSet<u64> =
                    filter_by_target(db, t).iter().map(|r| r.id).collect();
                ids = ids.intersection(&next).cloned().collect();
            }
            let mut v: Vec<u64> = ids.into_iter().collect(); v.sort();
            v.iter().filter_map(|&id| db.get(id).ok()).collect()
        }
        FromClause::Or(ts)   => {
            let mut ids = std::collections::HashSet::new();
            for t in ts { for r in filter_by_target(db, t) { ids.insert(r.id); } }
            let mut v: Vec<u64> = ids.into_iter().collect(); v.sort();
            v.iter().filter_map(|&id| db.get(id).ok()).collect()
        }
    }
}

fn filter_by_target<'a>(db: &'a Database, target: &LabelTarget) -> Vec<&'a Record> {
    match target {
        LabelTarget::All             => db.list_all(),
        LabelTarget::LabelName(name) => db.get_by_label(name),
    }
}

// ---------------------------------------------------------------------------
// WHERE句評価
// ---------------------------------------------------------------------------

fn eval_where(r: &Record, expr: &WhereExpr) -> bool {
    match expr {
        WhereExpr::Comparison(cmp) => eval_comparison(r, cmp),
        WhereExpr::And(l, rr)      => eval_where(r, l) && eval_where(r, rr),
        WhereExpr::Or(l, rr)       => eval_where(r, l) || eval_where(r, rr),
        WhereExpr::Not(e)          => !eval_where(r, e),
    }
}

fn eval_comparison(r: &Record, cmp: &Comparison) -> bool {
    let cell = r.columns.get(&cmp.column);
    if cmp.op == CompareOp::Like {
        if let (Some(DataType::Text(text)), LiteralValue::Text(pat)) = (cell, &cmp.value) {
            return like_match(text, pat);
        }
        return false;
    }
    if cmp.value == LiteralValue::Null {
        return match cmp.op {
            CompareOp::Eq => cell.map_or(true, |v| v == &DataType::Null),
            CompareOp::Ne => cell.map_or(false, |v| v != &DataType::Null),
            _ => false,
        };
    }
    let Some(data) = cell else { return false; };
    match (&cmp.op, data, &cmp.value) {
        (op, DataType::Text(a),    LiteralValue::Text(b))    => cmp_ord(a.as_str(), b.as_str(), op),
        (op, DataType::Integer(a), LiteralValue::Integer(b)) => cmp_ord(a, b, op),
        (op, DataType::Float(a),   LiteralValue::Float(b))   => cmp_f64(*a, *b, op),
        (op, DataType::Integer(a), LiteralValue::Float(b))   => cmp_f64(*a as f64, *b, op),
        (op, DataType::Float(a),   LiteralValue::Integer(b)) => cmp_f64(*a, *b as f64, op),
        (CompareOp::Eq, DataType::Boolean(a), LiteralValue::Boolean(b)) => a == b,
        (CompareOp::Ne, DataType::Boolean(a), LiteralValue::Boolean(b)) => a != b,
        _ => false,
    }
}

fn cmp_ord<T: PartialOrd>(a: T, b: T, op: &CompareOp) -> bool {
    match op {
        CompareOp::Eq=>a==b, CompareOp::Ne=>a!=b,
        CompareOp::Lt=>a<b,  CompareOp::Le=>a<=b,
        CompareOp::Gt=>a>b,  CompareOp::Ge=>a>=b,
        CompareOp::Like=>false,
    }
}
fn cmp_f64(a: f64, b: f64, op: &CompareOp) -> bool {
    match op {
        CompareOp::Eq=>(a-b).abs()<f64::EPSILON, CompareOp::Ne=>(a-b).abs()>=f64::EPSILON,
        CompareOp::Lt=>a<b, CompareOp::Le=>a<=b,
        CompareOp::Gt=>a>b, CompareOp::Ge=>a>=b,
        CompareOp::Like=>false,
    }
}

/// SQL LIKE パターンマッチ（% = 0文字以上, _ = 任意1文字）
fn like_match(text: &str, pattern: &str) -> bool {
    fn inner(t: &[char], p: &[char]) -> bool {
        match (t, p) {
            (_, [])               => t.is_empty(),
            (_, ['%', rest @ ..]) => (0..=t.len()).any(|i| inner(&t[i..], rest)),
            ([], _)               => false,
            ([tc, tr @ ..], [pc, pr @ ..]) => (*pc == '_' || *tc == *pc) && inner(tr, pr),
        }
    }
    inner(&text.chars().collect::<Vec<_>>(), &pattern.chars().collect::<Vec<_>>())
}

// ---------------------------------------------------------------------------
// ORDER BY
// ---------------------------------------------------------------------------

fn sort_records(records: &mut Vec<&Record>, order_by: &[OrderByItem]) {
    records.sort_by(|a, b| {
        for item in order_by {
            let cmp = cmp_dt(a.columns.get(&item.column), b.columns.get(&item.column));
            let cmp = if item.direction == OrderDirection::Desc { cmp.reverse() } else { cmp };
            if cmp != std::cmp::Ordering::Equal { return cmp; }
        }
        a.id.cmp(&b.id)
    });
}

// ---------------------------------------------------------------------------
// INSERT 実行
// ---------------------------------------------------------------------------

/// INSERT実行結果
#[derive(Debug, Clone, PartialEq)]
pub struct InsertResult {
    /// 発行されたレコードID
    pub id: u64,
    /// 実際に使用されたカラム名（値との対応順）
    pub columns: Vec<String>,
    /// 付与されたラベル
    pub labels: Vec<String>,
}

/// INSERT実行エラー
#[derive(Debug, PartialEq)]
pub enum InsertExecError {
    /// VALUE の個数とカラム名の個数が一致しない
    ColumnValueCountMismatch { columns: usize, values: usize },
    /// カラム順が指定されておらず、DBにも既存カラムが1つも無いため推測できない
    NoColumnsToInfer,
    /// db_engine 側のエラー（主に ID 重複）
    Database(db_engine::DatabaseError),
}

impl std::fmt::Display for InsertExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InsertExecError::ColumnValueCountMismatch { columns, values } => write!(
                f, "column count ({}) does not match value count ({})", columns, values
            ),
            InsertExecError::NoColumnsToInfer => write!(
                f, "cannot infer column order: no existing columns in database; specify INSERT INTO (...) (col1, col2, ...) VALUE (...)"
            ),
            InsertExecError::Database(e) => write!(f, "{}", e),
        }
    }
}

impl From<db_engine::DatabaseError> for InsertExecError {
    fn from(e: db_engine::DatabaseError) -> Self { InsertExecError::Database(e) }
}

fn literal_to_data_type(v: &LiteralValue) -> DataType {
    match v {
        LiteralValue::Text(s)    => DataType::Text(s.clone()),
        LiteralValue::Integer(n) => DataType::Integer(*n),
        LiteralValue::Float(f)   => DataType::Float(*f),
        LiteralValue::Boolean(b) => DataType::Boolean(*b),
        LiteralValue::Null       => DataType::Null,
    }
}

/// INSERT文からカラム順を確定し、Recordを組み立てる（db.insert/insert_fast共通の下ごしらえ）
fn build_insert_record(db: &Database, stmt: &InsertStatement) -> Result<(Record, Vec<String>), InsertExecError> {
    // カラム順が明示されていなければ、DB内の既存カラムをソート順で採用する
    // （SELECT * の列順と同じ規則）
    let columns: Vec<String> = match &stmt.columns {
        Some(cols) => cols.clone(),
        None => {
            let cols = db.list_all_columns();
            if cols.is_empty() { return Err(InsertExecError::NoColumnsToInfer); }
            cols
        }
    };

    if columns.len() != stmt.values.len() {
        return Err(InsertExecError::ColumnValueCountMismatch {
            columns: columns.len(), values: stmt.values.len(),
        });
    }

    let mut record = Record::new(0);
    for (col, val) in columns.iter().zip(stmt.values.iter()) {
        record.set(col.clone(), literal_to_data_type(val));
    }
    for label in &stmt.labels {
        record.add_label(label.clone());
    }

    Ok((record, columns))
}

pub fn execute_insert(db: &mut Database, stmt: &InsertStatement) -> Result<InsertResult, InsertExecError> {
    let (record, columns) = build_insert_record(db, stmt)?;
    let id = db.insert(record)?;
    Ok(InsertResult { id, columns, labels: stmt.labels.clone() })
}

/// WAL追記(insert_fast)でINSERTを実行する。.kdbモード用の高速パス（O(1)書き込み）。
pub fn execute_insert_fast(
    db: &mut Database,
    kdb: Option<&mut db_engine::kdb_store::KdbFile>,
    stmt: &InsertStatement,
) -> Result<InsertResult, InsertExecError> {
    let (record, columns) = build_insert_record(db, stmt)?;
    let id = db.insert_fast(record, kdb)?;
    Ok(InsertResult { id, columns, labels: stmt.labels.clone() })
}

// ---------------------------------------------------------------------------
// UPDATE 実行
// ---------------------------------------------------------------------------

/// UPDATE実行結果
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateResult {
    /// 更新されたレコード数（ラベルリネームの場合はリネームされたレコード数）
    pub updated_count: usize,
}

/// UPDATE実行エラー
#[derive(Debug, PartialEq)]
pub enum UpdateExecError {
    /// db_engine 側のエラー
    Database(db_engine::DatabaseError),
    /// UPDATE LABEL のリネーム先ラベル名が既にDB内に存在する（ラベル名の重複は禁止）
    DuplicateLabel(String),
}

impl std::fmt::Display for UpdateExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateExecError::Database(e) => write!(f, "{}", e),
            UpdateExecError::DuplicateLabel(label) =>
                write!(f, "Label '{}' already exists; cannot rename to a duplicate label name", label),
        }
    }
}

impl From<db_engine::DatabaseError> for UpdateExecError {
    fn from(e: db_engine::DatabaseError) -> Self { UpdateExecError::Database(e) }
}

pub fn execute_update(db: &mut Database, stmt: &UpdateStatement) -> Result<UpdateResult, UpdateExecError> {
    match stmt {
        UpdateStatement::Data(d)  => execute_update_data(db, d),
        UpdateStatement::Label(l) => execute_update_label(db, l),
    }
}

/// UPDATE label.name SET col=val, ... [WHERE ...]
/// 対象レコードの既存カラム・ラベルは保持したまま、SET句で指定されたカラムのみ上書きする
/// （db_engine::Database::update は渡した Record で丸ごと置き換える仕様のため、
///  既存カラムを消さないよう更新前レコードをクローンした上で SET 対象のみ書き換える）。
fn execute_update_data(db: &mut Database, stmt: &UpdateDataStatement) -> Result<UpdateResult, UpdateExecError> {
    let ids: Vec<u64> = filter_by_target(db, &stmt.target).iter().map(|r| r.id).collect();

    let mut updated_count = 0;
    for id in ids {
        let record = db.get(id)?.clone();
        if let Some(ref where_expr) = stmt.where_clause {
            if !eval_where(&record, where_expr) { continue; }
        }
        let mut new_record = record;
        for assign in &stmt.assignments {
            new_record.set(assign.column.clone(), literal_to_data_type(&assign.value));
        }
        db.update(id, new_record)?;
        updated_count += 1;
    }
    Ok(UpdateResult { updated_count })
}

/// UPDATE LABEL label.old SET label.new [WHERE ...]
/// old_label が付いているレコードのうち、WHERE に一致するものだけ old_label を外し new_label を
/// 付け直す（WHERE省略時は old_label が付いている全レコードが対象＝一括変更）。
/// new_label が既にDB内の他のレコードで使われている場合はラベル名の重複となるため、
/// （WHEREでの絞り込みに関わらず）何も変更せずエラーを返す（old_label への自己リネームを含む）。
fn execute_update_label(db: &mut Database, stmt: &UpdateLabelStatement) -> Result<UpdateResult, UpdateExecError> {
    if !db.get_by_label(&stmt.new_label).is_empty() {
        return Err(UpdateExecError::DuplicateLabel(stmt.new_label.clone()));
    }

    let ids: Vec<u64> = db.get_by_label(&stmt.old_label).iter()
        .filter(|r| stmt.where_clause.as_ref().map_or(true, |w| eval_where(r, w)))
        .map(|r| r.id)
        .collect();

    let mut updated_count = 0;
    for id in ids {
        db.detach_label(id, &stmt.old_label)?;
        db.attach_label(id, stmt.new_label.clone())?;
        updated_count += 1;
    }
    Ok(UpdateResult { updated_count })
}

// ---------------------------------------------------------------------------
// DELETE 実行
// ---------------------------------------------------------------------------

/// DELETE実行結果
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteResult {
    /// 削除されたレコード数（ラベルのみ除去の場合はラベルを外したレコード数）
    pub deleted_count: usize,
}

/// DELETE実行エラー
#[derive(Debug, PartialEq)]
pub enum DeleteExecError {
    /// db_engine 側のエラー
    Database(db_engine::DatabaseError),
}

impl std::fmt::Display for DeleteExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteExecError::Database(e) => write!(f, "{}", e),
        }
    }
}

impl From<db_engine::DatabaseError> for DeleteExecError { fn from(e: db_engine::DatabaseError) -> Self { DeleteExecError::Database(e) } }

pub fn execute_delete(db: &mut Database, stmt: &DeleteStatement) -> Result<DeleteResult, DeleteExecError> {
    match stmt {
        DeleteStatement::Data(d)  => execute_delete_data(db, d),
        DeleteStatement::Label(l) => execute_delete_label(db, l),
    }
}

/// DELETE FROM label.name [WHERE ...]  |  DELETE FROM label.* [WHERE ...]
/// 対象ラベル（`label.*` なら全レコード）のうち WHERE に一致するレコードを完全に削除する
/// （WHERE省略時は対象ラベルの全レコードが一括削除される）。
fn execute_delete_data(db: &mut Database, stmt: &DeleteDataStatement) -> Result<DeleteResult, DeleteExecError> {
    let ids: Vec<u64> = filter_by_target(db, &stmt.target).iter()
        .filter(|r| stmt.where_clause.as_ref().map_or(true, |w| eval_where(r, w)))
        .map(|r| r.id)
        .collect();

    let mut deleted_count = 0;
    for id in ids {
        db.delete(id)?;
        deleted_count += 1;
    }
    Ok(DeleteResult { deleted_count })
}

/// DELETE LABEL FROM label.name [WHERE ...]
/// label が付いているレコードのうち WHERE に一致するものだけ label を外す。
/// レコード自体・他のラベル・カラムデータは変更しない（WHERE省略時は label が付いた全レコードが対象）。
fn execute_delete_label(db: &mut Database, stmt: &DeleteLabelStatement) -> Result<DeleteResult, DeleteExecError> {
    let ids: Vec<u64> = db.get_by_label(&stmt.label).iter()
        .filter(|r| stmt.where_clause.as_ref().map_or(true, |w| eval_where(r, w)))
        .map(|r| r.id)
        .collect();

    let mut deleted_count = 0;
    for id in ids {
        if db.detach_label(id, &stmt.label)? { deleted_count += 1; }
    }
    Ok(DeleteResult { deleted_count })
}

fn cmp_dt(a: Option<&DataType>, b: Option<&DataType>) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match (a, b) {
        (None, None)    => Equal,
        (None, Some(_)) => Greater,
        (Some(_), None) => Less,
        (Some(a), Some(b)) => match (a, b) {
            (DataType::Integer(x), DataType::Integer(y)) => x.cmp(y),
            (DataType::Float(x),   DataType::Float(y))   =>
                x.partial_cmp(y).unwrap_or(Equal),
            (DataType::Text(x),    DataType::Text(y))    => x.cmp(y),
            (DataType::Boolean(x), DataType::Boolean(y)) => x.cmp(y),
            (DataType::Integer(x), DataType::Float(y))   =>
                (*x as f64).partial_cmp(y).unwrap_or(Equal),
            (DataType::Float(x),   DataType::Integer(y)) =>
                x.partial_cmp(&(*y as f64)).unwrap_or(Equal),
            _ => Equal,
        }
    }
}

#[cfg(test)]
mod insert_tests {
    use super::*;
    use crate::parser::parse_insert;

    fn exec(db: &mut Database, sql: &str) -> Result<InsertResult, InsertExecError> {
        let stmt = parse_insert(sql).expect("parse should succeed");
        execute_insert(db, &stmt)
    }

    #[test]
    fn test_insert_with_explicit_columns_creates_record() {
        let mut db = Database::new();
        let result = exec(&mut db,
            "INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')"
        ).unwrap();
        assert_eq!(result.id, 1);
        assert_eq!(result.labels, vec!["employee".to_string()]);
        let record = db.get(1).unwrap();
        assert_eq!(record.columns.get("employee_name"), Some(&DataType::Text("田中".into())));
        assert_eq!(record.columns.get("employee_age"), Some(&DataType::Integer(24)));
        assert!(record.labels.contains(&"employee".to_string()));
    }

    #[test]
    fn test_insert_infers_column_order_from_existing_columns() {
        let mut db = Database::new();
        exec(&mut db,
            "INSERT INTO (label.employee) (age, name) VALUE (24, '田中')"
        ).unwrap();
        // 既存カラムは age, name (ソート順)。列指定なしならその順に値を対応させる。
        let result = exec(&mut db, "INSERT INTO (label.employee) VALUE (31, 'Suzuki')").unwrap();
        assert_eq!(result.columns, vec!["age".to_string(), "name".to_string()]);
        let record = db.get(result.id).unwrap();
        assert_eq!(record.columns.get("age"), Some(&DataType::Integer(31)));
        assert_eq!(record.columns.get("name"), Some(&DataType::Text("Suzuki".into())));
    }

    #[test]
    fn test_insert_without_columns_and_empty_db_errors() {
        let mut db = Database::new();
        let err = exec(&mut db, "INSERT INTO (label.employee) VALUE ('田中', 24)").unwrap_err();
        assert_eq!(err, InsertExecError::NoColumnsToInfer);
    }

    #[test]
    fn test_insert_column_value_count_mismatch_errors() {
        let mut db = Database::new();
        let err = exec(&mut db,
            "INSERT INTO (label.employee) (name) VALUE ('田中', 24)"
        ).unwrap_err();
        assert_eq!(err, InsertExecError::ColumnValueCountMismatch { columns: 1, values: 2 });
    }

    #[test]
    fn test_insert_multiple_labels_attached() {
        let mut db = Database::new();
        let result = exec(&mut db,
            "INSERT INTO (label.employee, label.manager) (name) VALUE ('田中')"
        ).unwrap();
        let record = db.get(result.id).unwrap();
        assert!(record.labels.contains(&"employee".to_string()));
        assert!(record.labels.contains(&"manager".to_string()));
    }

    #[test]
    fn test_insert_creates_new_label_on_the_fly() {
        let mut db = Database::new();
        assert!(db.list_all_labels().is_empty());
        exec(&mut db, "INSERT INTO (label.brandnew) (name) VALUE ('x')").unwrap();
        assert_eq!(db.list_all_labels(), vec!["brandnew".to_string()]);
    }

    #[test]
    fn test_insert_rejects_duplicate_label_in_same_statement() {
        let mut db = Database::new();
        let err = exec(&mut db,
            "INSERT INTO (label.employee, label.employee) (name) VALUE ('田中')"
        ).unwrap_err();
        assert_eq!(err, InsertExecError::Database(db_engine::DatabaseError::DuplicateLabel("employee".to_string())));
        assert_eq!(db.count(), 0);
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use crate::parser::{parse_insert, parse_update};

    fn insert(db: &mut Database, sql: &str) -> InsertResult {
        execute_insert(db, &parse_insert(sql).expect("parse should succeed")).unwrap()
    }
    fn update(db: &mut Database, sql: &str) -> Result<UpdateResult, UpdateExecError> {
        execute_update(db, &parse_update(sql).expect("parse should succeed"))
    }

    #[test]
    fn test_update_sets_matching_row_and_keeps_others() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (employee_name, employee_age) VALUE ('木邑', 24)");
        insert(&mut db, "INSERT INTO (label.employee) (employee_name, employee_age) VALUE ('田中', 30)");

        let result = update(&mut db,
            "UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'"
        ).unwrap();
        assert_eq!(result.updated_count, 1);

        let r1 = db.get(1).unwrap();
        assert_eq!(r1.columns.get("employee_name"), Some(&DataType::Text("木村".into())));
        // SET対象でないカラムは維持される
        assert_eq!(r1.columns.get("employee_age"), Some(&DataType::Integer(24)));
        // 一致しない行は変更されない
        let r2 = db.get(2).unwrap();
        assert_eq!(r2.columns.get("employee_name"), Some(&DataType::Text("田中".into())));
    }

    #[test]
    fn test_update_preserves_labels() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee, label.manager) (name) VALUE ('田中')");
        update(&mut db, "UPDATE label.employee SET name='鈴木' WHERE name='田中'").unwrap();
        let record = db.get(1).unwrap();
        assert!(record.labels.contains(&"employee".to_string()));
        assert!(record.labels.contains(&"manager".to_string()));
    }

    #[test]
    fn test_update_without_where_updates_all_in_label() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('鈴木')");
        let result = update(&mut db, "UPDATE label.employee SET status='active'").unwrap();
        assert_eq!(result.updated_count, 2);
        assert_eq!(db.get(1).unwrap().columns.get("status"), Some(&DataType::Text("active".into())));
        assert_eq!(db.get(2).unwrap().columns.get("status"), Some(&DataType::Text("active".into())));
    }

    #[test]
    fn test_update_no_match_returns_zero_count() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        let result = update(&mut db, "UPDATE label.employee SET name='X' WHERE name='存在しない'").unwrap();
        assert_eq!(result.updated_count, 0);
    }

    #[test]
    fn test_update_label_rename_moves_records() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('鈴木')");

        let result = update(&mut db, "UPDATE LABEL label.employee SET label.staff").unwrap();
        assert_eq!(result.updated_count, 2);

        assert_eq!(db.get_by_label("employee").len(), 0);
        assert_eq!(db.get_by_label("staff").len(), 2);
    }

    #[test]
    fn test_update_label_rename_no_match_returns_zero_count() {
        let mut db = Database::new();
        let result = update(&mut db, "UPDATE LABEL label.nonexistent SET label.other").unwrap();
        assert_eq!(result.updated_count, 0);
    }

    #[test]
    fn test_update_label_rename_rejects_duplicate_target_and_does_not_change() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.staff) (name) VALUE ('鈴木')");

        let err = update(&mut db, "UPDATE LABEL label.employee SET label.staff").unwrap_err();
        assert_eq!(err, UpdateExecError::DuplicateLabel("staff".to_string()));

        // 何も変更されていないこと
        assert_eq!(db.get_by_label("employee").len(), 1);
        assert_eq!(db.get_by_label("staff").len(), 1);
    }

    #[test]
    fn test_update_label_rename_with_where_only_renames_matching_records() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('田中', 'sales')");
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('鈴木', 'dev')");

        let result = update(&mut db,
            "UPDATE LABEL label.employee SET label.sales_staff WHERE department='sales'"
        ).unwrap();
        assert_eq!(result.updated_count, 1);

        // 条件に一致した田中だけ sales_staff に移り、鈴木は employee のまま
        assert_eq!(db.get_by_label("employee").len(), 1);
        assert_eq!(db.get_by_label("employee")[0].columns.get("name"), Some(&DataType::Text("鈴木".into())));
        assert_eq!(db.get_by_label("sales_staff").len(), 1);
        assert_eq!(db.get_by_label("sales_staff")[0].columns.get("name"), Some(&DataType::Text("田中".into())));
    }

    #[test]
    fn test_update_label_rename_with_where_no_match_returns_zero_and_keeps_labels() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('田中', 'sales')");

        let result = update(&mut db,
            "UPDATE LABEL label.employee SET label.dev_staff WHERE department='dev'"
        ).unwrap();
        assert_eq!(result.updated_count, 0);
        assert_eq!(db.get_by_label("employee").len(), 1);
        assert_eq!(db.get_by_label("dev_staff").len(), 0);
    }

    #[test]
    fn test_update_label_rename_to_self_rejected_as_duplicate() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");

        let err = update(&mut db, "UPDATE LABEL label.employee SET label.employee").unwrap_err();
        assert_eq!(err, UpdateExecError::DuplicateLabel("employee".to_string()));
        assert_eq!(db.get_by_label("employee").len(), 1);
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use crate::parser::{parse_insert, parse_delete};

    fn insert(db: &mut Database, sql: &str) -> InsertResult {
        execute_insert(db, &parse_insert(sql).expect("parse should succeed")).unwrap()
    }
    fn delete(db: &mut Database, sql: &str) -> Result<DeleteResult, DeleteExecError> {
        execute_delete(db, &parse_delete(sql).expect("parse should succeed"))
    }

    #[test]
    fn test_delete_data_removes_matching_row_and_keeps_others() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (employee_name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.employee) (employee_name) VALUE ('鈴木')");

        let result = delete(&mut db, "DELETE FROM label.employee WHERE employee_name='田中'").unwrap();
        assert_eq!(result.deleted_count, 1);

        assert!(db.get(1).is_err());
        assert_eq!(db.get(2).unwrap().columns.get("employee_name"), Some(&DataType::Text("鈴木".into())));
        assert_eq!(db.get_by_label("employee").len(), 1);
    }

    #[test]
    fn test_delete_data_without_where_removes_all_in_label() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('鈴木')");
        insert(&mut db, "INSERT INTO (label.manager) (name) VALUE ('佐藤')");

        let result = delete(&mut db, "DELETE FROM label.employee").unwrap();
        assert_eq!(result.deleted_count, 2);

        assert_eq!(db.get_by_label("employee").len(), 0);
        // 別ラベルのレコードは影響を受けない
        assert_eq!(db.get_by_label("manager").len(), 1);
        assert_eq!(db.count(), 1);
    }

    #[test]
    fn test_delete_from_label_star_removes_everything() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");
        insert(&mut db, "INSERT INTO (label.manager) (name) VALUE ('佐藤')");

        let result = delete(&mut db, "DELETE FROM label.*").unwrap();
        assert_eq!(result.deleted_count, 2);
        assert_eq!(db.count(), 0);
    }

    #[test]
    fn test_delete_no_match_returns_zero_count() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");

        let result = delete(&mut db, "DELETE FROM label.employee WHERE name='存在しない'").unwrap();
        assert_eq!(result.deleted_count, 0);
        assert_eq!(db.count(), 1);
    }

    #[test]
    fn test_delete_label_only_detaches_label_and_keeps_data() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee, label.manager) (name) VALUE ('田中')");

        let result = delete(&mut db, "DELETE LABEL FROM label.employee").unwrap();
        assert_eq!(result.deleted_count, 1);

        // レコード自体は残っており、他のラベル・データも維持される
        let record = db.get(1).unwrap();
        assert!(!record.labels.contains(&"employee".to_string()));
        assert!(record.labels.contains(&"manager".to_string()));
        assert_eq!(record.columns.get("name"), Some(&DataType::Text("田中".into())));
        assert_eq!(db.get_by_label("employee").len(), 0);
    }

    #[test]
    fn test_delete_label_with_where_only_affects_matching_records() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('田中', 'sales')");
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('鈴木', 'dev')");

        let result = delete(&mut db,
            "DELETE LABEL FROM label.employee WHERE department='sales'"
        ).unwrap();
        assert_eq!(result.deleted_count, 1);

        assert_eq!(db.get_by_label("employee").len(), 1);
        assert_eq!(db.get_by_label("employee")[0].columns.get("name"), Some(&DataType::Text("鈴木".into())));
        // データ自体は削除されず残っている
        assert_eq!(db.count(), 2);
    }

    #[test]
    fn test_delete_label_no_match_returns_zero_count() {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name) VALUE ('田中')");

        let result = delete(&mut db, "DELETE LABEL FROM label.nonexistent").unwrap();
        assert_eq!(result.deleted_count, 0);
        assert_eq!(db.count(), 1);
    }
}

