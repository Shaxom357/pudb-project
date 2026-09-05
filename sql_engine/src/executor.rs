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

    // total_matched は WHERE 後・GROUP BY/LIMIT/OFFSET 前の一致件数（従来どおり集計前の生レコード数）
    let total_matched = records.len();

    if is_aggregate_query(stmt) {
        // GROUP BY / 集計関数を含むクエリは、グループごとに合成した Record を1行として扱う。
        // こちらは新規に Record を組み立てる都合上、参照ではなく所有データとして持つ。
        let mut grouped = build_grouped_records(&records, stmt);
        if let Some(ref having) = stmt.having {
            grouped.retain(|r| eval_where(r, having));
        }
        let refs: Vec<&Record> = grouped.iter().collect();
        finish_select(db, refs, stmt, total_matched)
    } else {
        finish_select(db, records, stmt, total_matched)
    }
}

/// GROUP BY 句があるか、集計関数（`COUNT`/`SUM`/`AVG`/`MIN`/`MAX`）が1つでも SELECT に
/// 含まれるか、または HAVING 句があるかを判定する（このいずれかがあれば集計クエリとして扱う）。
fn is_aggregate_query(stmt: &SelectStatement) -> bool {
    if !stmt.group_by.is_empty() || stmt.having.is_some() { return true; }
    matches!(&stmt.columns, SelectColumns::Named(items)
        if items.iter().any(|it| matches!(it, SelectItem::Aggregate(_))))
}

/// ORDER BY・LIMIT/OFFSET・DISTINCT・カラム投影を適用して最終結果を組み立てる
/// （集計クエリ・非集計クエリの両方から共通で呼ばれる）。
fn finish_select(db: &Database, mut records: Vec<&Record>, stmt: &SelectStatement, total_matched: usize) -> QueryResult {
    // ORDER BY はカラム名のほか、SELECT ... AS で付けたエイリアスでも指定できるようにする
    // （元のカラム名に解決してから並べ替える）。
    let order_by = resolve_order_by_aliases(&stmt.columns, &stmt.order_by);
    if !order_by.is_empty() { sort_records(&mut records, &order_by); }

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
        SelectColumns::Named(items) => (
            items.iter().map(select_item_output_name).collect(),
            items.iter().map(select_item_internal_key).collect(),
        ),
    };

    let mut rows: Vec<Vec<CellValue>> = records.iter().map(|r| {
        internal_cols.iter().map(|col| {
            if col == "__id" { CellValue::Integer(r.id as i64) }
            else if col == "__labels" { CellValue::Text(r.labels.join(",")) }
            else { r.columns.get(col).map(CellValue::from_data_type).unwrap_or(CellValue::Null) }
        }).collect()
    }).collect();

    // DISTINCT は投影後の出力行（表示カラムの値の組）に対して重複排除する。
    // LIMIT/OFFSET より前に適用しないと、重複排除で件数が減った分だけ結果がずれてしまう。
    if stmt.distinct {
        let mut seen: std::collections::HashSet<Vec<String>> = std::collections::HashSet::new();
        rows.retain(|row| seen.insert(row.iter().map(CellValue::display).collect()));
    }

    if let Some(offset) = stmt.offset {
        let skip = (offset as usize).min(rows.len());
        rows.drain(0..skip);
    }
    if let Some(limit) = stmt.limit { rows.truncate(limit as usize); }

    QueryResult { columns: display_cols, rows, total_matched }
}

/// SELECT 項目の表示名（`AS` エイリアスがあればそれ、無ければ既定名）
fn select_item_output_name(item: &SelectItem) -> String {
    match item {
        SelectItem::Column(c)    => c.alias.clone().unwrap_or_else(|| c.column.clone()),
        SelectItem::Aggregate(a) => aggregate_output_name(a),
    }
}

/// SELECT 項目の値を Record から引くための内部キー（素のカラムはカラム名そのもの、
/// 集計関数は `build_grouped_records` が合成レコードへ書き込むキーと一致させる）
fn select_item_internal_key(item: &SelectItem) -> String {
    match item {
        SelectItem::Column(c)    => c.column.clone(),
        SelectItem::Aggregate(a) => aggregate_output_name(a),
    }
}

/// 集計関数呼び出しの既定の出力名（`AS` 省略時）。`AS` があればそちらを優先する。
/// この名前は表示名だけでなく、合成レコードの内部カラムキーとしても使う。
fn aggregate_output_name(agg: &AggregateItem) -> String {
    if let Some(alias) = &agg.alias { return alias.clone(); }
    let func = match agg.func {
        AggregateFunc::Count => "count", AggregateFunc::Sum => "sum",
        AggregateFunc::Avg   => "avg",   AggregateFunc::Min => "min", AggregateFunc::Max => "max",
    };
    match &agg.arg {
        AggregateArg::Star        => format!("{}(*)", func),
        AggregateArg::Column(col) => format!("{}({})", func, col),
    }
}

// ---------------------------------------------------------------------------
// GROUP BY / 集計関数
// ---------------------------------------------------------------------------

/// WHERE 適用後のレコードを GROUP BY 句でグルーピングし、グループごとに1つの合成
/// Record を組み立てる（`GROUP BY` 省略時は全件を1グループとして扱う）。
/// 合成 Record のカラムには GROUP BY 対象カラムの値（代表レコード＝先頭の値をそのまま採用）
/// と、集計関数の計算結果を `aggregate_output_name` のキーで格納する。GROUP BY に
/// 含まれない素のカラムを SELECT した場合も同様に代表レコードの値を採用する（標準SQLの
/// ように GROUP BY 対象外カラムの指定をエラーにはしない、緩めの仕様）。
fn build_grouped_records(records: &[&Record], stmt: &SelectStatement) -> Vec<Record> {
    let mut order: Vec<Vec<String>> = Vec::new();
    let mut members: std::collections::HashMap<Vec<String>, Vec<Record>> = std::collections::HashMap::new();
    for r in records {
        let key = group_key(r, &stmt.group_by);
        if !members.contains_key(&key) { order.push(key.clone()); }
        members.entry(key).or_default().push((*r).clone());
    }
    if records.is_empty() && stmt.group_by.is_empty() {
        // GROUP BY 無しで対象レコードが0件の場合でも、COUNT(*)=0 のような集計結果を
        // 1行返す（標準SQLの集計クエリと同じ挙動）。
        order.push(vec![]);
        members.insert(vec![], vec![]);
    }

    order.into_iter().map(|key| {
        let group = &members[&key];
        let mut out = Record::new(0);
        for col in &stmt.group_by {
            if let Some(first) = group.first() {
                if let Some(v) = first.columns.get(col) { out.set(col.clone(), v.clone()); }
            }
        }
        if let SelectColumns::Named(items) = &stmt.columns {
            for item in items {
                match item {
                    SelectItem::Aggregate(agg) => {
                        let value = eval_aggregate(agg, group);
                        out.set(aggregate_output_name(agg), cell_to_data_type(&value));
                    }
                    SelectItem::Column(c) => {
                        if let Some(first) = group.first() {
                            if let Some(v) = first.columns.get(&c.column) { out.set(c.column.clone(), v.clone()); }
                        }
                    }
                }
            }
        }
        out
    }).collect()
}

/// GROUP BY 対象カラムの値からグルーピングキーを作る（カラム未設定・NULL は1つの
/// グループにまとめる。型ごとにタグを付けて `Text("1")` と `Integer(1)` を別グループにする）。
fn group_key(r: &Record, group_by: &[String]) -> Vec<String> {
    group_by.iter().map(|c| match r.columns.get(c) {
        None | Some(DataType::Null) => "\u{0}NULL".to_string(),
        Some(DataType::Text(s))     => format!("T:{}", s),
        Some(DataType::Integer(n))  => format!("I:{}", n),
        Some(DataType::Float(f))    => format!("F:{}", f),
        Some(DataType::Boolean(b))  => format!("B:{}", b),
    }).collect()
}

/// 1グループ分のレコード群に対して集計関数を1つ計算する。
/// `SUM`/`MIN`/`MAX` は対象カラムが全て整数なら `Integer` を、Float が1つでも混ざれば
/// `Float` を返す（`AVG` は常に `Float`）。対象カラムに数値が1件も無ければ `Null`
/// （`COUNT` は対象0件でも `Integer(0)` を返す。標準SQLの集計関数と同じ挙動）。
fn eval_aggregate(agg: &AggregateItem, group: &[Record]) -> CellValue {
    if agg.func == AggregateFunc::Count {
        return match &agg.arg {
            AggregateArg::Star => CellValue::Integer(group.len() as i64),
            AggregateArg::Column(col) => CellValue::Integer(
                group.iter().filter(|r| !matches!(r.columns.get(col.as_str()), None | Some(DataType::Null))).count() as i64
            ),
        };
    }
    // パーサーが SUM/AVG/MIN/MAX(*) を弾いているため、ここに到達する引数は必ず Column
    let AggregateArg::Column(col) = &agg.arg else {
        unreachable!("SUM/AVG/MIN/MAX with '*' is rejected by the parser");
    };
    let (values, all_integer) = numeric_column_values(group, col);
    if values.is_empty() { return CellValue::Null; }
    match agg.func {
        AggregateFunc::Sum => {
            let total: f64 = values.iter().sum();
            if all_integer { CellValue::Integer(total as i64) } else { CellValue::Float(total) }
        }
        AggregateFunc::Avg => CellValue::Float(values.iter().sum::<f64>() / values.len() as f64),
        AggregateFunc::Min => {
            let m = values.iter().cloned().fold(f64::INFINITY, f64::min);
            if all_integer { CellValue::Integer(m as i64) } else { CellValue::Float(m) }
        }
        AggregateFunc::Max => {
            let m = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if all_integer { CellValue::Integer(m as i64) } else { CellValue::Float(m) }
        }
        AggregateFunc::Count => unreachable!("handled above"),
    }
}

/// グループ内の指定カラムから数値（Integer/Float）だけを取り出す。
/// 戻り値の bool は「対象カラムの値が全て Integer だったか」（1つでも Float が
/// 混ざれば false）。テキスト・真偽値・NULL・未設定の行は集計対象から除外する。
fn numeric_column_values(group: &[Record], col: &str) -> (Vec<f64>, bool) {
    let mut values = Vec::new();
    let mut all_integer = true;
    for r in group {
        match r.columns.get(col) {
            Some(DataType::Integer(n)) => values.push(*n as f64),
            Some(DataType::Float(f))   => { values.push(*f); all_integer = false; }
            _ => {}
        }
    }
    (values, all_integer)
}

fn cell_to_data_type(v: &CellValue) -> DataType {
    match v {
        CellValue::Text(s)    => DataType::Text(s.clone()),
        CellValue::Integer(n) => DataType::Integer(*n),
        CellValue::Float(f)   => DataType::Float(*f),
        CellValue::Boolean(b) => DataType::Boolean(*b),
        CellValue::Null       => DataType::Null,
    }
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
        WhereExpr::In(e)           => eval_in(r, e),
        WhereExpr::Between(e)      => eval_between(r, e),
        WhereExpr::IsNull(e)       => eval_is_null(r, e),
        WhereExpr::And(l, rr)      => eval_where(r, l) && eval_where(r, rr),
        WhereExpr::Or(l, rr)       => eval_where(r, l) || eval_where(r, rr),
        WhereExpr::Not(e)          => !eval_where(r, e),
    }
}

/// `column IN (v1, v2, ...)`: いずれかの値と等価かどうかを、既存の `=` 比較（NULL特殊扱い込み）
/// に委譲して判定する。
fn eval_in(r: &Record, e: &InExpr) -> bool {
    e.values.iter().any(|v| {
        eval_comparison(r, &Comparison { column: e.column.clone(), op: CompareOp::Eq, value: v.clone() })
    })
}

/// `column BETWEEN low AND high`: `column >= low AND column <= high` と同義。
/// 数値・文字列の型混在の扱いは既存の `>=`/`<=` 比較にそのまま委譲する。
fn eval_between(r: &Record, e: &BetweenExpr) -> bool {
    let ge_low = eval_comparison(r, &Comparison { column: e.column.clone(), op: CompareOp::Ge, value: e.low.clone() });
    let le_high = eval_comparison(r, &Comparison { column: e.column.clone(), op: CompareOp::Le, value: e.high.clone() });
    ge_low && le_high
}

/// `column IS NULL`: 既存の `= NULL` 特殊扱い（カラム未設定 or DataType::Null で真）に委譲する。
fn eval_is_null(r: &Record, e: &IsNullExpr) -> bool {
    eval_comparison(r, &Comparison { column: e.column.clone(), op: CompareOp::Eq, value: LiteralValue::Null })
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

/// `ORDER BY` の各項目が `SELECT ... AS alias` のエイリアス名を指している場合、
/// 元のカラム名に解決した新しい Vec を返す（該当しない項目はそのまま）。
fn resolve_order_by_aliases(columns: &SelectColumns, order_by: &[OrderByItem]) -> Vec<OrderByItem> {
    let SelectColumns::Named(items) = columns else { return order_by.to_vec(); };
    // 集計関数（`COUNT(*) AS n` 等）はエイリアス無しでは括弧を含む式になり ORDER BY から
    // 識別子として参照できないため、エイリアスが無い集計項目は対象外（内部キーへの解決不要）。
    let alias_map: std::collections::HashMap<&str, String> = items.iter()
        .filter_map(|it| match it {
            SelectItem::Column(c)    => c.alias.as_deref().map(|a| (a, c.column.clone())),
            SelectItem::Aggregate(a) => a.alias.as_deref().map(|alias| (alias, aggregate_output_name(a))),
        })
        .collect();
    if alias_map.is_empty() { return order_by.to_vec(); }
    order_by.iter().map(|item| OrderByItem {
        column: alias_map.get(item.column.as_str()).cloned().unwrap_or_else(|| item.column.clone()),
        direction: item.direction.clone(),
    }).collect()
}

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
mod select_tests {
    use super::*;
    use crate::parser::{parse_insert, parse_select};

    fn insert(db: &mut Database, sql: &str) -> InsertResult {
        execute_insert(db, &parse_insert(sql).expect("parse should succeed")).unwrap()
    }
    fn select(db: &Database, sql: &str) -> QueryResult {
        execute_select(db, &parse_select(sql).expect("parse should succeed"))
    }

    /// 田中(24, dev) / 鈴木(30, sales) / 佐藤(40, department未設定) の3件を持つDBを用意する
    fn setup() -> Database {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name, age, department) VALUE ('田中', 24, 'dev')");
        insert(&mut db, "INSERT INTO (label.employee) (name, age, department) VALUE ('鈴木', 30, 'sales')");
        insert(&mut db, "INSERT INTO (label.employee) (name, age) VALUE ('佐藤', 40)");
        db
    }

    #[test]
    fn test_select_in_matches_any_listed_value() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee WHERE department IN ('dev', 'sales')");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn test_select_not_in_treats_missing_column_as_not_equal() {
        let db = setup();
        // NOT IN は既存の `!=` 相当の二値ロジックに委譲しているため、department未設定の佐藤も
        // 「'dev'ではない」として一致する（標準SQLの三値論理とは異なる。既存の `!=`/`=` の NULL
        // 特殊扱いと挙動を揃えるための仕様）。
        let r = select(&db, "SELECT name FROM label.employee WHERE department NOT IN ('dev')");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn test_select_between_range() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee WHERE age BETWEEN 25 AND 35");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0], CellValue::Text("鈴木".into()));
    }

    #[test]
    fn test_select_not_between_excludes_range() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee WHERE age NOT BETWEEN 25 AND 35");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn test_select_is_null_matches_missing_column() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee WHERE department IS NULL");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0], CellValue::Text("佐藤".into()));
    }

    #[test]
    fn test_select_is_not_null_excludes_missing_column() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee WHERE department IS NOT NULL");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn test_select_alias_changes_output_column_name() {
        let db = setup();
        let r = select(&db, "SELECT age AS employee_age FROM label.employee WHERE name='田中'");
        assert_eq!(r.columns, vec!["employee_age".to_string()]);
        assert_eq!(r.rows[0][0], CellValue::Integer(24));
    }

    #[test]
    fn test_select_order_by_alias_resolves_to_underlying_column() {
        let db = setup();
        let r = select(&db, "SELECT name, age AS a FROM label.employee ORDER BY a DESC");
        let names: Vec<&CellValue> = r.rows.iter().map(|row| &row[0]).collect();
        assert_eq!(names, vec![
            &CellValue::Text("佐藤".into()), &CellValue::Text("鈴木".into()), &CellValue::Text("田中".into()),
        ]);
    }

    #[test]
    fn test_select_limit_offset_pages_through_results() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee ORDER BY age ASC LIMIT 1 OFFSET 1");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0], CellValue::Text("鈴木".into()));
        // total_matched は LIMIT/OFFSET を適用する前の件数
        assert_eq!(r.total_matched, 3);
    }

    #[test]
    fn test_select_offset_beyond_result_count_returns_empty() {
        let db = setup();
        let r = select(&db, "SELECT name FROM label.employee OFFSET 100");
        assert_eq!(r.rows.len(), 0);
    }

    #[test]
    fn test_select_bare_label_name_sugar_matches_label_dot_form() {
        let db = setup();
        let r = select(&db, "SELECT name FROM employee WHERE age = 24");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0], CellValue::Text("田中".into()));
    }

    // --- GROUP BY / 集計関数 / HAVING / DISTINCT ---

    /// 田中(dev,24) / 鈴木(dev,30) / 佐藤(sales,40) / 高橋(sales, age未設定) の4件を持つDB
    fn setup_departments() -> Database {
        let mut db = Database::new();
        insert(&mut db, "INSERT INTO (label.employee) (name, department, age) VALUE ('田中', 'dev', 24)");
        insert(&mut db, "INSERT INTO (label.employee) (name, department, age) VALUE ('鈴木', 'dev', 30)");
        insert(&mut db, "INSERT INTO (label.employee) (name, department, age) VALUE ('佐藤', 'sales', 40)");
        insert(&mut db, "INSERT INTO (label.employee) (name, department) VALUE ('高橋', 'sales')");
        db
    }

    #[test]
    fn test_select_count_star_group_by() {
        let db = setup_departments();
        let r = select(&db, "SELECT department, COUNT(*) AS n FROM label.employee GROUP BY department ORDER BY department");
        assert_eq!(r.columns, vec!["department".to_string(), "n".to_string()]);
        assert_eq!(r.rows, vec![
            vec![CellValue::Text("dev".into()), CellValue::Integer(2)],
            vec![CellValue::Text("sales".into()), CellValue::Integer(2)],
        ]);
    }

    #[test]
    fn test_select_sum_avg_min_max_per_group_all_integer_stays_integer() {
        let db = setup_departments();
        let r = select(&db,
            "SELECT department, SUM(age) AS s, AVG(age) AS a, MIN(age) AS lo, MAX(age) AS hi \
             FROM label.employee GROUP BY department ORDER BY department"
        );
        // dev: age=24,30 -> sum=54, avg=27.0, min=24, max=30（全て整数なのでSUM/MIN/MAXはInteger）
        assert_eq!(r.rows[0], vec![
            CellValue::Text("dev".into()), CellValue::Integer(54), CellValue::Float(27.0),
            CellValue::Integer(24), CellValue::Integer(30),
        ]);
        // sales: age=40 のみ（高橋はage未設定なので集計対象外）
        assert_eq!(r.rows[1], vec![
            CellValue::Text("sales".into()), CellValue::Integer(40), CellValue::Float(40.0),
            CellValue::Integer(40), CellValue::Integer(40),
        ]);
    }

    #[test]
    fn test_select_count_column_excludes_missing_and_null() {
        let db = setup_departments();
        let r = select(&db, "SELECT department, COUNT(age) AS n FROM label.employee GROUP BY department ORDER BY department");
        // dev: 2件とも age あり -> 2 / sales: 佐藤のみ age あり(高橋は未設定) -> 1
        assert_eq!(r.rows, vec![
            vec![CellValue::Text("dev".into()), CellValue::Integer(2)],
            vec![CellValue::Text("sales".into()), CellValue::Integer(1)],
        ]);
    }

    #[test]
    fn test_select_aggregate_without_group_by_returns_single_row() {
        let db = setup_departments();
        let r = select(&db, "SELECT COUNT(*) AS n FROM label.employee");
        assert_eq!(r.rows, vec![vec![CellValue::Integer(4)]]);
    }

    #[test]
    fn test_select_aggregate_over_empty_result_returns_zero_row() {
        let db = setup_departments();
        let r = select(&db, "SELECT COUNT(*) AS n, SUM(age) AS s FROM label.employee WHERE department = 'nonexistent'");
        assert_eq!(r.rows, vec![vec![CellValue::Integer(0), CellValue::Null]]);
    }

    #[test]
    fn test_select_having_filters_groups() {
        let db = setup_departments();
        let r = select(&db,
            "SELECT department, COUNT(*) AS n FROM label.employee GROUP BY department HAVING n > 2"
        );
        assert_eq!(r.rows.len(), 0);
        let r2 = select(&db,
            "SELECT department, COUNT(*) AS n FROM label.employee GROUP BY department HAVING n >= 2"
        );
        assert_eq!(r2.rows.len(), 2);
    }

    #[test]
    fn test_select_group_by_order_by_aggregate_alias() {
        let db = setup_departments();
        let r = select(&db,
            "SELECT department, SUM(age) AS total FROM label.employee GROUP BY department ORDER BY total DESC"
        );
        assert_eq!(r.rows[0][0], CellValue::Text("dev".into())); // sum=54 > sales(40)
    }

    #[test]
    fn test_select_distinct_deduplicates_rows() {
        let db = setup_departments();
        let r = select(&db, "SELECT DISTINCT department FROM label.employee ORDER BY department");
        assert_eq!(r.rows, vec![
            vec![CellValue::Text("dev".into())],
            vec![CellValue::Text("sales".into())],
        ]);
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

