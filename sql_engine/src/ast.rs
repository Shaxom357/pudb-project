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

/// INSERT文全体
/// 例: INSERT INTO (label.employee) VALUE ('田中', 24, 'developer')
///     INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')
#[derive(Debug, Clone, PartialEq)]
pub struct InsertStatement {
    /// 追加先ラベル（1つ以上。空や `label.*` は不可）
    pub labels: Vec<String>,
    /// 追加する値
    pub values: Vec<LiteralValue>,
    /// 値に対応するカラム名（ラベル指定の直後に丸括弧で明示指定された場合のみ Some）。
    /// None の場合は DB内の既存カラムをソート順で対応付ける。
    pub columns: Option<Vec<String>>,
}

/// UPDATE文全体（データ更新 or ラベルのリネーム）
/// 例: UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'
///     UPDATE LABEL label.employee SET label.staff
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatement {
    /// UPDATE label.name SET col=val, ... [WHERE ...]
    Data(UpdateDataStatement),
    /// UPDATE LABEL label.old SET label.new
    Label(UpdateLabelStatement),
}

/// データ更新: UPDATE label.name SET col=val, ... [WHERE ...]
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateDataStatement {
    /// 更新対象ラベル
    pub target: LabelTarget,
    /// SET句のカラム=値の割り当て（1つ以上）
    pub assignments: Vec<Assignment>,
    /// WHERE句（省略時は対象ラベルの全レコードが更新される）
    pub where_clause: Option<WhereExpr>,
}

/// SET句の1項目: column = value
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub column: String,
    pub value: LiteralValue,
}

/// ラベルのリネーム: UPDATE LABEL label.old SET label.new [WHERE ...]
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateLabelStatement {
    pub old_label: String,
    pub new_label: String,
    /// WHERE句（省略時は old_label が付いた全レコードが対象）
    pub where_clause: Option<WhereExpr>,
}
