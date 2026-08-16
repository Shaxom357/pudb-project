// sql_engine/src/parser.rs
// 手書きの再帰下降パーサー（外部ライブラリ不使用）

use crate::ast::*;

// ---------------------------------------------------------------------------
// トークン定義
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // キーワード
    Select, From, Where, And, Or, Not,
    OrderBy, // ORDER BY は2語だが1トークンとして扱う
    Limit, Like, Asc, Desc, Null, True, False,
    Insert, Into, Value,
    // 記号
    Star, Comma, Dot,
    Eq, Ne, Lt, Le, Gt, Ge,
    LParen, RParen,
    // リテラル
    StringLit(String),
    IntLit(i64),
    FloatLit(f64),
    // 識別子
    Ident(String),
    // EOF
    Eof,
}

// ---------------------------------------------------------------------------
// パースエラー
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    UnexpectedChar(char),
    UnexpectedToken { got: String, expected: String },
    UnterminatedString,
    EmptyQuery,
    UnsupportedSyntax(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnexpectedChar(c) =>
                write!(f, "Unexpected character: '{}'", c),
            ParseError::UnexpectedToken { got, expected } =>
                write!(f, "Expected {}, but got '{}'", expected, got),
            ParseError::UnterminatedString =>
                write!(f, "Unterminated string literal"),
            ParseError::EmptyQuery =>
                write!(f, "Empty query"),
            ParseError::UnsupportedSyntax(s) =>
                write!(f, "Unsupported syntax: {}", s),
        }
    }
}


// ---------------------------------------------------------------------------
// レキサー（字句解析器）
// ---------------------------------------------------------------------------

pub struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Lexer { input, pos: 0 }
    }

    fn remaining(&self) -> &str { &self.input[self.pos..] }
    fn peek_char(&self) -> Option<char> { self.remaining().chars().next() }
    fn advance_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }
    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() { self.advance_char(); } else { break; }
        }
    }

    fn read_string(&mut self) -> Result<Token, ParseError> {
        let mut s = String::new();
        loop {
            match self.advance_char() {
                None => return Err(ParseError::UnterminatedString),
                Some('\'') => {
                    if self.peek_char() == Some('\'') { self.advance_char(); s.push('\''); }
                    else { break; }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Token::StringLit(s))
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut s = String::from(first);
        let mut is_float = false;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() { s.push(c); self.advance_char(); }
            else if c == '.' && !is_float {
                let after_dot = self.input.get(self.pos + 1..)
                    .and_then(|r| r.chars().next())
                    .map_or(false, |x| x.is_ascii_digit());
                if after_dot { is_float = true; s.push(c); self.advance_char(); } else { break; }
            } else { break; }
        }
        if is_float { Token::FloatLit(s.parse().unwrap_or(0.0)) }
        else        { Token::IntLit(s.parse().unwrap_or(0))     }
    }

    fn read_ident(&mut self, first: char) -> Token {
        let mut s = String::from(first);
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' { s.push(c); self.advance_char(); }
            else { break; }
        }
        match s.to_uppercase().as_str() {
            "SELECT" => Token::Select,
            "FROM"   => Token::From,
            "WHERE"  => Token::Where,
            "AND"    => Token::And,
            "OR"     => Token::Or,
            "NOT"    => Token::Not,
            "ORDER"  => {
                let saved = self.pos;
                self.skip_whitespace();
                let mut kw = String::new();
                while let Some(c) = self.peek_char() {
                    if c.is_alphabetic() { kw.push(c); self.advance_char(); } else { break; }
                }
                if kw.to_uppercase() == "BY" { Token::OrderBy }
                else { self.pos = saved; Token::Ident(s) }
            }
            "LIMIT"  => Token::Limit,
            "LIKE"   => Token::Like,
            "ASC"    => Token::Asc,
            "DESC"   => Token::Desc,
            "NULL"   => Token::Null,
            "TRUE"   => Token::True,
            "FALSE"  => Token::False,
            "INSERT" => Token::Insert,
            "INTO"   => Token::Into,
            "VALUE" | "VALUES" => Token::Value,
            _        => Token::Ident(s),
        }
    }

    pub fn next_token(&mut self) -> Result<Token, ParseError> {
        self.skip_whitespace();
        if self.remaining().starts_with("--") {
            while let Some(c) = self.advance_char() { if c == '\n' { break; } }
            return self.next_token();
        }
        match self.advance_char() {
            None       => Ok(Token::Eof),
            Some('*')  => Ok(Token::Star),
            Some(',')  => Ok(Token::Comma),
            Some('.')  => Ok(Token::Dot),
            Some('(')  => Ok(Token::LParen),
            Some(')')  => Ok(Token::RParen),
            Some('=')  => Ok(Token::Eq),
            Some('!')  => {
                if self.peek_char() == Some('=') { self.advance_char(); Ok(Token::Ne) }
                else { Err(ParseError::UnexpectedChar('!')) }
            }
            Some('<')  => {
                if      self.peek_char() == Some('=') { self.advance_char(); Ok(Token::Le) }
                else if self.peek_char() == Some('>') { self.advance_char(); Ok(Token::Ne) }
                else { Ok(Token::Lt) }
            }
            Some('>')  => {
                if self.peek_char() == Some('=') { self.advance_char(); Ok(Token::Ge) }
                else { Ok(Token::Gt) }
            }
            Some('\'') => self.read_string(),
            Some(c) if c.is_ascii_digit() => Ok(self.read_number(c)),
            Some('-') => {
                if self.peek_char().map_or(false, |d| d.is_ascii_digit()) {
                    let d = self.advance_char().unwrap();
                    match self.read_number(d) {
                        Token::IntLit(n)   => Ok(Token::IntLit(-n)),
                        Token::FloatLit(f) => Ok(Token::FloatLit(-f)),
                        other => Ok(other),
                    }
                } else { Err(ParseError::UnexpectedChar('-')) }
            }
            Some(c) if c.is_alphabetic() || c == '_' => Ok(self.read_ident(c)),
            Some(';') => Ok(Token::Eof),
            Some(c)   => Err(ParseError::UnexpectedChar(c)),
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok == Token::Eof;
            tokens.push(tok);
            if is_eof { break; }
        }
        Ok(tokens)
    }
}

// ---------------------------------------------------------------------------
// パーサー本体
// ---------------------------------------------------------------------------

pub struct Parser { tokens: Vec<Token>, pos: usize }

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Parser { tokens, pos: 0 } }
    fn peek(&self) -> &Token { self.tokens.get(self.pos).unwrap_or(&Token::Eof) }
    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1; t
    }
    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "identifier".into() }),
        }
    }
    fn expect(&mut self, e: Token) -> Result<(), ParseError> {
        let t = self.advance();
        if t == e { Ok(()) } else { Err(ParseError::UnexpectedToken {
            got: format!("{:?}", t), expected: format!("{:?}", e) }) }
    }
    /// 文の終端で余分なトークンが残っていないことを確認する
    fn expect_eof(&mut self) -> Result<(), ParseError> {
        if self.peek() == &Token::Eof { Ok(()) }
        else { Err(ParseError::UnexpectedToken {
            got: format!("{:?}", self.peek()), expected: "end of statement".into() }) }
    }

    pub fn parse_select(&mut self) -> Result<SelectStatement, ParseError> {
        self.expect(Token::Select)?;
        let columns = self.parse_select_columns()?;
        self.expect(Token::From)?;
        let from = self.parse_from_clause()?;
        let where_clause = if self.peek() == &Token::Where {
            self.advance(); Some(self.parse_where_expr()?)
        } else { None };
        let order_by = if self.peek() == &Token::OrderBy {
            self.advance(); self.parse_order_by()?
        } else { vec![] };
        let limit = if self.peek() == &Token::Limit {
            self.advance();
            match self.advance() {
                Token::IntLit(n) if n > 0 => Some(n as u64),
                o => return Err(ParseError::UnexpectedToken {
                    got: format!("{:?}", o), expected: "positive integer".into() }),
            }
        } else { None };
        Ok(SelectStatement { columns, from, where_clause, order_by, limit })
    }

    fn parse_select_columns(&mut self) -> Result<SelectColumns, ParseError> {
        if self.peek() == &Token::Star { self.advance(); return Ok(SelectColumns::All); }
        let mut cols = vec![self.expect_ident()?];
        while self.peek() == &Token::Comma { self.advance(); cols.push(self.expect_ident()?); }
        Ok(SelectColumns::Named(cols))
    }

    fn parse_from_clause(&mut self) -> Result<FromClause, ParseError> {
        let first = self.parse_label_target()?;
        let is_and = match self.peek() {
            Token::And => Some(true), Token::Or => Some(false), _ => None,
        };
        if is_and.is_none() { return Ok(FromClause::Label(first)); }
        let is_and = is_and.unwrap();
        let mut targets = vec![first];
        while (is_and && self.peek()==&Token::And)||(!is_and && self.peek()==&Token::Or) {
            self.advance(); targets.push(self.parse_label_target()?);
        }
        if is_and { Ok(FromClause::And(targets)) } else { Ok(FromClause::Or(targets)) }
    }

    fn parse_label_target(&mut self) -> Result<LabelTarget, ParseError> {
        match self.advance() {
            Token::Ident(s) if s.to_lowercase() == "label" => {}
            o => return Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "'label'".into() }),
        }
        self.expect(Token::Dot)?;
        match self.advance() {
            Token::Star        => Ok(LabelTarget::All),
            Token::Ident(name) => Ok(LabelTarget::LabelName(name)),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "label name or *".into() }),
        }
    }

    fn parse_where_expr(&mut self) -> Result<WhereExpr, ParseError> { self.parse_or_expr() }

    fn parse_or_expr(&mut self) -> Result<WhereExpr, ParseError> {
        let mut left = self.parse_and_expr()?;
        while self.peek() == &Token::Or {
            self.advance(); let r = self.parse_and_expr()?;
            left = WhereExpr::Or(Box::new(left), Box::new(r));
        }
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<WhereExpr, ParseError> {
        let mut left = self.parse_not_expr()?;
        while self.peek() == &Token::And {
            self.advance(); let r = self.parse_not_expr()?;
            left = WhereExpr::And(Box::new(left), Box::new(r));
        }
        Ok(left)
    }

    fn parse_not_expr(&mut self) -> Result<WhereExpr, ParseError> {
        if self.peek() == &Token::Not {
            self.advance();
            return Ok(WhereExpr::Not(Box::new(self.parse_primary_expr()?)));
        }
        self.parse_primary_expr()
    }

    fn parse_primary_expr(&mut self) -> Result<WhereExpr, ParseError> {
        if self.peek() == &Token::LParen {
            self.advance(); let e = self.parse_where_expr()?;
            self.expect(Token::RParen)?; return Ok(e);
        }
        Ok(WhereExpr::Comparison(self.parse_comparison()?))
    }

    fn parse_comparison(&mut self) -> Result<Comparison, ParseError> {
        let column = self.expect_ident()?;
        let op = match self.advance() {
            Token::Eq   => CompareOp::Eq,  Token::Ne   => CompareOp::Ne,
            Token::Lt   => CompareOp::Lt,  Token::Le   => CompareOp::Le,
            Token::Gt   => CompareOp::Gt,  Token::Ge   => CompareOp::Ge,
            Token::Like => CompareOp::Like,
            o => return Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "comparison operator".into() }),
        };
        let value = self.parse_literal()?;
        Ok(Comparison { column, op, value })
    }

    fn parse_literal(&mut self) -> Result<LiteralValue, ParseError> {
        match self.advance() {
            Token::StringLit(s) => Ok(LiteralValue::Text(s)),
            Token::IntLit(n)    => Ok(LiteralValue::Integer(n)),
            Token::FloatLit(f)  => Ok(LiteralValue::Float(f)),
            Token::True         => Ok(LiteralValue::Boolean(true)),
            Token::False        => Ok(LiteralValue::Boolean(false)),
            Token::Null         => Ok(LiteralValue::Null),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "literal value".into() }),
        }
    }

    fn parse_order_by(&mut self) -> Result<Vec<OrderByItem>, ParseError> {
        let mut items = vec![];
        loop {
            let col = self.expect_ident()?;
            let dir = match self.peek() {
                Token::Desc => { self.advance(); OrderDirection::Desc }
                Token::Asc  => { self.advance(); OrderDirection::Asc  }
                _           => OrderDirection::Asc,
            };
            items.push(OrderByItem { column: col, direction: dir });
            if self.peek() != &Token::Comma { break; } self.advance();
        }
        Ok(items)
    }

    // -----------------------------------------------------------------
    // INSERT
    // -----------------------------------------------------------------

    /// INSERT INTO (label.a, ...) [(col1, col2, ...)] VALUE (v1, v2, ...)
    /// カラム名の丸括弧はラベル指定の直後・VALUEの前に置く（標準SQLの
    /// `INSERT INTO table (col1, col2) VALUES (...)` に準じる語順）。省略時は
    /// DB内の既存カラムをソート順で対応付ける。
    pub fn parse_insert(&mut self) -> Result<InsertStatement, ParseError> {
        self.expect(Token::Insert)?;
        self.expect(Token::Into)?;
        self.expect(Token::LParen)?;
        let labels = self.parse_insert_label_list()?;
        self.expect(Token::RParen)?;
        let columns = if self.peek() == &Token::LParen {
            self.advance();
            let cols = self.parse_ident_list()?;
            self.expect(Token::RParen)?;
            Some(cols)
        } else { None };
        self.expect(Token::Value)?;
        self.expect(Token::LParen)?;
        let values = self.parse_value_list()?;
        self.expect(Token::RParen)?;
        Ok(InsertStatement { labels, values, columns })
    }

    /// INSERT INTO の丸括弧内: label.name を1つ以上、カンマ区切りで受け取る。
    /// `label.name`（識別子形式、SELECTと同じ）と `'label.name'`（文字列リテラル形式）の
    /// 両方を受理する。
    fn parse_insert_label_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut labels = vec![self.parse_insert_label_item()?];
        while self.peek() == &Token::Comma {
            self.advance();
            labels.push(self.parse_insert_label_item()?);
        }
        Ok(labels)
    }

    fn parse_insert_label_item(&mut self) -> Result<String, ParseError> {
        if let Token::StringLit(_) = self.peek() {
            let s = match self.advance() { Token::StringLit(s) => s, _ => unreachable!() };
            let name = s.strip_prefix("label.")
                .ok_or_else(|| ParseError::UnsupportedSyntax(
                    format!("label must be in 'label.<name>' form, got '{}'", s)))?;
            return validate_insert_label_name(name);
        }
        match self.advance() {
            Token::Ident(s) if s.to_lowercase() == "label" => {}
            o => return Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "'label' or 'label.<name>' string".into() }),
        }
        self.expect(Token::Dot)?;
        match self.advance() {
            Token::Star => Err(ParseError::UnsupportedSyntax(
                "INSERT label name cannot be empty or '*'".into())),
            Token::Ident(name) => validate_insert_label_name(&name),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "label name".into() }),
        }
    }

    fn parse_value_list(&mut self) -> Result<Vec<LiteralValue>, ParseError> {
        let mut values = vec![self.parse_literal()?];
        while self.peek() == &Token::Comma {
            self.advance();
            values.push(self.parse_literal()?);
        }
        Ok(values)
    }

    fn parse_ident_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut items = vec![self.expect_ident()?];
        while self.peek() == &Token::Comma {
            self.advance();
            items.push(self.expect_ident()?);
        }
        Ok(items)
    }
}

fn validate_insert_label_name(name: &str) -> Result<String, ParseError> {
    if name.is_empty() || name == "*" {
        return Err(ParseError::UnsupportedSyntax(
            "INSERT label name cannot be empty or '*'".into()));
    }
    Ok(name.to_string())
}

/// SELECT文をパースする公開エントリーポイント
pub fn parse_select(sql: &str) -> Result<SelectStatement, ParseError> {
    if sql.trim().is_empty() { return Err(ParseError::EmptyQuery); }
    let tokens = Lexer::new(sql).tokenize()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_select()?;
    parser.expect_eof()?;
    Ok(stmt)
}

/// INSERT文をパースする公開エントリーポイント
pub fn parse_insert(sql: &str) -> Result<InsertStatement, ParseError> {
    if sql.trim().is_empty() { return Err(ParseError::EmptyQuery); }
    let tokens = Lexer::new(sql).tokenize()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_insert()?;
    parser.expect_eof()?;
    Ok(stmt)
}

// ---------------------------------------------------------------------------
// テスト
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_star_from_label() {
        let s = parse_select("SELECT * FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::All);
        assert_eq!(s.from, FromClause::Label(LabelTarget::LabelName("employee".into())));
    }
    #[test]
    fn test_select_from_label_star() {
        let s = parse_select("SELECT * FROM label.*").unwrap();
        assert_eq!(s.from, FromClause::Label(LabelTarget::All));
    }
    #[test]
    fn test_select_and_labels() {
        let s = parse_select("SELECT * FROM label.product AND label.drink").unwrap();
        assert_eq!(s.from, FromClause::And(vec![
            LabelTarget::LabelName("product".into()),
            LabelTarget::LabelName("drink".into()),
        ]));
    }
    #[test]
    fn test_select_or_labels() {
        let s = parse_select("SELECT * FROM label.employee OR label.manager").unwrap();
        assert_eq!(s.from, FromClause::Or(vec![
            LabelTarget::LabelName("employee".into()),
            LabelTarget::LabelName("manager".into()),
        ]));
    }
    #[test]
    fn test_select_with_where_eq() {
        let s = parse_select("SELECT * FROM label.employee WHERE name = 'Tanaka'").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Comparison(Comparison {
            column: "name".into(), op: CompareOp::Eq,
            value: LiteralValue::Text("Tanaka".into()),
        })));
    }
    #[test]
    fn test_select_with_where_and() {
        let s = parse_select(
            "SELECT * FROM label.employee WHERE age > 30 AND city = 'Tokyo'"
        ).unwrap();
        assert!(matches!(s.where_clause, Some(WhereExpr::And(_, _))));
    }
    #[test]
    fn test_select_named_columns() {
        let s = parse_select("SELECT name, age FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::Named(vec!["name".into(), "age".into()]));
    }
    #[test]
    fn test_select_order_by_limit() {
        let s = parse_select(
            "SELECT * FROM label.employee ORDER BY age DESC LIMIT 10"
        ).unwrap();
        assert_eq!(s.order_by.len(), 1);
        assert_eq!(s.order_by[0].direction, OrderDirection::Desc);
        assert_eq!(s.limit, Some(10));
    }
    #[test]
    fn test_select_case_insensitive() {
        let s = parse_select("select * from label.employee where name = 'Alice'").unwrap();
        assert!(s.where_clause.is_some());
    }
    #[test]
    fn test_select_with_like() {
        let s = parse_select("SELECT * FROM label.employee WHERE name LIKE 'Tana%'").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Comparison(Comparison {
            column: "name".into(), op: CompareOp::Like,
            value: LiteralValue::Text("Tana%".into()),
        })));
    }
    #[test]
    fn test_select_with_ne_op() {
        let s = parse_select("SELECT * FROM label.* WHERE status != 'inactive'").unwrap();
        assert!(matches!(s.where_clause, Some(WhereExpr::Comparison(Comparison { op: CompareOp::Ne, .. }))));
    }
    #[test]
    fn test_select_with_paren_expr() {
        let s = parse_select(
            "SELECT * FROM label.* WHERE (age > 20 AND age < 40) OR city = 'Osaka'"
        ).unwrap();
        assert!(matches!(s.where_clause, Some(WhereExpr::Or(_, _))));
    }
    #[test]
    fn test_semicolon_terminator() {
        let s = parse_select("SELECT * FROM label.employee;").unwrap();
        assert_eq!(s.from, FromClause::Label(LabelTarget::LabelName("employee".into())));
    }

    // -- INSERT --

    #[test]
    fn test_insert_quoted_label_no_column_order() {
        let s = parse_insert(
            "INSERT INTO ('label.employee') VALUE ('田中', '24', 'developer');"
        ).unwrap();
        assert_eq!(s.labels, vec!["employee".to_string()]);
        assert_eq!(s.values, vec![
            LiteralValue::Text("田中".into()),
            LiteralValue::Text("24".into()),
            LiteralValue::Text("developer".into()),
        ]);
        assert_eq!(s.columns, None);
    }

    #[test]
    fn test_insert_with_explicit_column_order() {
        let s = parse_insert(
            "INSERT INTO ('label.employee') (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')"
        ).unwrap();
        assert_eq!(s.columns, Some(vec![
            "employee_name".into(), "employee_age".into(), "employee_department".into(),
        ]));
        assert_eq!(s.values[1], LiteralValue::Integer(24));
    }

    #[test]
    fn test_insert_identifier_label_form() {
        let s = parse_insert("INSERT INTO (label.employee) VALUES (1, 2)").unwrap();
        assert_eq!(s.labels, vec!["employee".to_string()]);
    }

    #[test]
    fn test_insert_multiple_labels() {
        let s = parse_insert(
            "INSERT INTO (label.employee, label.manager) VALUE ('田中', 24)"
        ).unwrap();
        assert_eq!(s.labels, vec!["employee".to_string(), "manager".to_string()]);
    }

    #[test]
    fn test_insert_rejects_empty_label() {
        assert!(parse_insert("INSERT INTO ('label.') VALUE (1)").is_err());
    }

    #[test]
    fn test_insert_rejects_star_label() {
        assert!(parse_insert("INSERT INTO (label.*) VALUE (1)").is_err());
    }

    #[test]
    fn test_insert_rejects_non_label_prefixed_string() {
        assert!(parse_insert("INSERT INTO ('employee') VALUE (1)").is_err());
    }

    #[test]
    fn test_insert_rejects_trailing_tokens() {
        // 旧構文（VALUE の後に WHERE でカラム順指定）は廃止されたため、
        // 末尾に余分なトークンが残っている場合は明示的にエラーとする
        assert!(parse_insert(
            "INSERT INTO (label.employee) VALUE ('X', 1, 'Y') WHERE (a, b, c)"
        ).is_err());
    }

    #[test]
    fn test_select_rejects_trailing_tokens() {
        assert!(parse_select("SELECT * FROM label.employee GARBAGE").is_err());
    }
}


