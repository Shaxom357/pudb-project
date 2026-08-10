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
    let (display_cols, internal_cols): (Vec<String>, Vec<String>) = match &stmt.columns {
        SelectColumns::All => {
            let mut cols = db.list_all_columns();
            cols.sort();
            let internal = std::iter::once("__id".to_string()).chain(cols.iter().cloned()).collect();
            let display  = std::iter::once("id".to_string()).chain(cols.into_iter()).collect();
            (display, internal)
        }
        SelectColumns::Named(names) => (names.clone(), names.clone()),
    };

    let rows: Vec<Vec<CellValue>> = records.iter().map(|r| {
        internal_cols.iter().map(|col| {
            if col == "__id" { CellValue::Integer(r.id as i64) }
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

