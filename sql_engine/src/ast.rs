// sql_engine/src/ast.rs
// SQL の抽象構文木（Abstract Syntax Tree）定義

/// SELECT文全体
#[derive(Debug, Clone, PartialEq)]
pub struct SelectStatement {
    /// 取得するカラム（`*` なら空 Vec）
    pub columns: SelectColumns,
    /// FROM句（ラベル指定）
    pub from: FromClause,
    /// WHERE句（省略可）
    pub where_clause: Option<WhereExpr>,
    /// ORDER BY句（省略可）
    pub order_by: Vec<OrderByItem>,
    /// LIMIT句（省略可）
    pub limit: Option<u64>,
}

/// SELECT で取得するカラムの指定
#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumns {
    /// SELECT *
    All,
    /// SELECT col1, col2, ...
    Named(Vec<String>),
}

/// FROM句：ラベル条件の組み合わせ
#[derive(Debug, Clone, PartialEq)]
pub enum FromClause {
    /// 単一ラベル: FROM label.name  |  FROM label.* (全レコード)
    Label(LabelTarget),
    /// AND結合: FROM label.a AND label.b
    And(Vec<LabelTarget>),
    /// OR結合:  FROM label.a OR label.b
    Or(Vec<LabelTarget>),
}

/// FROM句の個別ラベル指定
#[derive(Debug, Clone, PartialEq)]
pub enum LabelTarget {
    /// FROM label.employee  →  LabelName("employee")
    LabelName(String),
    /// FROM label.*  → 全レコードを対象にする
    All,
}

/// WHERE句の式（再帰的なAND/OR対応）
#[derive(Debug, Clone, PartialEq)]
pub enum WhereExpr {
    /// 単純な比較: column = 'value'
    Comparison(Comparison),
    /// AND結合
    And(Box<WhereExpr>, Box<WhereExpr>),
    /// OR結合
    Or(Box<WhereExpr>, Box<WhereExpr>),
    /// NOT
    Not(Box<WhereExpr>),
}

/// 比較式: column op value
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub column: String,
    pub op: CompareOp,
    pub value: LiteralValue,
}

/// 比較演算子
#[derive(Debug, Clone, PartialEq)]
pub enum CompareOp {
    Eq,  // =
    Ne,  // != or <>
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
    Like, // LIKE
}

/// リテラル値
#[derive(Debug, Clone, PartialEq)]
pub enum LiteralValue {
    Text(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Null,
}

/// ORDER BY の1項目
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByItem {
    pub column: String,
    pub direction: OrderDirection,
}

/// ソート方向
#[derive(Debug, Clone, PartialEq)]
pub enum OrderDirection {
    Asc,
    Desc,
}
