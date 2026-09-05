// sql_engine/src/parser.rs
// 手書きの再帰下降パーサー（外部ライブラリ不使用）

use crate::ast::*;

// ---------------------------------------------------------------------------
// トークン定義
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // キーワード
    Select, Distinct, From, Where, And, Or, Not,
    OrderBy, // ORDER BY は2語だが1トークンとして扱う
    GroupBy, // GROUP BY も同様に2語で1トークン
    Having,
    Limit, Offset, Like, Asc, Desc, Null, True, False,
    Is, In, Between, As,
    Insert, Into, Value,
    Update, Set,
    Delete,
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
            "GROUP"  => {
                let saved = self.pos;
                self.skip_whitespace();
                let mut kw = String::new();
                while let Some(c) = self.peek_char() {
                    if c.is_alphabetic() { kw.push(c); self.advance_char(); } else { break; }
                }
                if kw.to_uppercase() == "BY" { Token::GroupBy }
                else { self.pos = saved; Token::Ident(s) }
            }
            "HAVING"  => Token::Having,
            "DISTINCT" => Token::Distinct,
            "LIMIT"   => Token::Limit,
            "OFFSET"  => Token::Offset,
            "LIKE"    => Token::Like,
            "ASC"     => Token::Asc,
            "DESC"    => Token::Desc,
            "NULL"    => Token::Null,
            "TRUE"    => Token::True,
            "FALSE"   => Token::False,
            "IS"      => Token::Is,
            "IN"      => Token::In,
            "BETWEEN" => Token::Between,
            "AS"      => Token::As,
            "INSERT" => Token::Insert,
            "INTO"   => Token::Into,
            "VALUE" | "VALUES" => Token::Value,
            "UPDATE" => Token::Update,
            "SET"    => Token::Set,
            "DELETE" => Token::Delete,
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
    fn peek_at(&self, offset: usize) -> &Token { self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof) }
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
        let distinct = if self.peek() == &Token::Distinct { self.advance(); true } else { false };
        let columns = self.parse_select_columns()?;
        self.expect(Token::From)?;
        let from = self.parse_from_clause()?;
        let where_clause = if self.peek() == &Token::Where {
            self.advance(); Some(self.parse_where_expr()?)
        } else { None };
        let group_by = if self.peek() == &Token::GroupBy {
            self.advance(); self.parse_ident_list()?
        } else { vec![] };
        let having = if self.peek() == &Token::Having {
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
        let offset = if self.peek() == &Token::Offset {
            self.advance();
            match self.advance() {
                Token::IntLit(n) if n >= 0 => Some(n as u64),
                o => return Err(ParseError::UnexpectedToken {
                    got: format!("{:?}", o), expected: "non-negative integer".into() }),
            }
        } else { None };

        // GROUP BY / HAVING は集計目的の構文なので、カラムを列挙しない SELECT * とは
        // 組み合わせられない（列挙されていないと、どのカラムがグループ単位の代表値で
        // どれが集計結果なのか区別できないため）。
        if matches!(columns, SelectColumns::All) && (!group_by.is_empty() || having.is_some()) {
            return Err(ParseError::UnsupportedSyntax(
                "SELECT * cannot be combined with GROUP BY / HAVING; list columns explicitly".into()));
        }

        Ok(SelectStatement { distinct, columns, from, where_clause, group_by, having, order_by, limit, offset })
    }

    fn parse_select_columns(&mut self) -> Result<SelectColumns, ParseError> {
        if self.peek() == &Token::Star { self.advance(); return Ok(SelectColumns::All); }
        let mut items = vec![self.parse_select_item()?];
        while self.peek() == &Token::Comma { self.advance(); items.push(self.parse_select_item()?); }
        Ok(SelectColumns::Named(items))
    }

    /// `col [AS alias]` または `COUNT(*)`/`SUM(col)`/`AVG(col)`/`MIN(col)`/`MAX(col) [AS alias]`
    /// の1項目をパースする。集計関数名は予約語化せず、識別子の直後が `(` のときだけ
    /// 集計関数呼び出しとして扱う（`count` 等をそのまま列名として使えるようにするため）。
    fn parse_select_item(&mut self) -> Result<SelectItem, ParseError> {
        if let Token::Ident(name) = self.peek().clone() {
            if let Some(func) = aggregate_func_from_name(&name) {
                if self.peek_at(1) == &Token::LParen {
                    self.advance(); // 関数名
                    self.advance(); // "("
                    let arg = if func == AggregateFunc::Count && self.peek() == &Token::Star {
                        self.advance();
                        AggregateArg::Star
                    } else {
                        AggregateArg::Column(self.expect_ident()?)
                    };
                    self.expect(Token::RParen)?;
                    if arg == AggregateArg::Star && func != AggregateFunc::Count {
                        return Err(ParseError::UnsupportedSyntax(
                            "'*' can only be used with COUNT(*); SUM/AVG/MIN/MAX require a column name".into()));
                    }
                    let alias = if self.peek() == &Token::As {
                        self.advance(); Some(self.expect_ident()?)
                    } else { None };
                    return Ok(SelectItem::Aggregate(AggregateItem { func, arg, alias }));
                }
            }
        }
        let column = self.expect_ident()?;
        let alias = if self.peek() == &Token::As {
            self.advance();
            Some(self.expect_ident()?)
        } else { None };
        Ok(SelectItem::Column(ColumnItem { column, alias }))
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

    /// `label.name`（識別子形式）と `'label.name'` / `'label.*'`（文字列リテラル形式）の
    /// 両方を受理する。識別子は英数字・`_` しか許容しないため、`:` などラベルによく使う
    /// 区切り文字（例: `country:Japan`）を含む名前は文字列リテラル形式でのみ指定できる。
    /// `label.name`（識別子形式）、`'label.name'` / `'label.*'`（文字列リテラル形式）、
    /// および `name`（`label.` を省略した糖衣構文。標準SQLのテーブル名感覚で書ける）の
    /// いずれも受理する。識別子は英数字・`_`しか許容しないため、`:` などラベルによく使う
    /// 区切り文字（例: `country:Japan`）を含む名前は文字列リテラル形式でのみ指定できる。
    fn parse_label_target(&mut self) -> Result<LabelTarget, ParseError> {
        if let Token::StringLit(_) = self.peek() {
            let s = match self.advance() { Token::StringLit(s) => s, _ => unreachable!() };
            let name = parse_quoted_label_name(&s)?;
            return if name == "*" { Ok(LabelTarget::All) }
                    else { Ok(LabelTarget::LabelName(validate_label_name(&name)?)) };
        }
        match self.advance() {
            // `label.name` / `label.*` 形式（"label" の直後が "." のときだけこちらとして扱う）
            Token::Ident(s) if s.to_lowercase() == "label" && self.peek() == &Token::Dot => {
                self.advance(); // "."
                match self.advance() {
                    Token::Star        => Ok(LabelTarget::All),
                    Token::Ident(name) => Ok(LabelTarget::LabelName(name)),
                    o => Err(ParseError::UnexpectedToken {
                        got: format!("{:?}", o), expected: "label name or *".into() }),
                }
            }
            // `label.` を省略した糖衣構文: FROM race は FROM label.race と同義
            Token::Ident(name) => Ok(LabelTarget::LabelName(name)),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "label name".into() }),
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
        self.parse_predicate()
    }

    /// `column op value` / `column IS [NOT] NULL` / `column [NOT] IN (...)` /
    /// `column [NOT] BETWEEN low AND high` のいずれか1つの述語をパースする。
    /// `NOT IN` / `NOT BETWEEN` / `IS NOT NULL` は `WhereExpr::Not` で包んで表現する
    /// （`WHERE NOT (...)` の先頭 NOT と同じ評価経路に乗せ、評価器側の重複を避けるため）。
    fn parse_predicate(&mut self) -> Result<WhereExpr, ParseError> {
        let column = self.expect_ident()?;

        if self.peek() == &Token::Is {
            self.advance();
            let negate = if self.peek() == &Token::Not { self.advance(); true } else { false };
            self.expect(Token::Null)?;
            let expr = WhereExpr::IsNull(IsNullExpr { column });
            return Ok(if negate { WhereExpr::Not(Box::new(expr)) } else { expr });
        }

        let negate = if self.peek() == &Token::Not { self.advance(); true } else { false };

        if self.peek() == &Token::In {
            self.advance();
            self.expect(Token::LParen)?;
            let values = self.parse_value_list()?;
            self.expect(Token::RParen)?;
            let expr = WhereExpr::In(InExpr { column, values });
            return Ok(if negate { WhereExpr::Not(Box::new(expr)) } else { expr });
        }
        if self.peek() == &Token::Between {
            self.advance();
            let low = self.parse_literal()?;
            self.expect(Token::And)?;
            let high = self.parse_literal()?;
            let expr = WhereExpr::Between(BetweenExpr { column, low, high });
            return Ok(if negate { WhereExpr::Not(Box::new(expr)) } else { expr });
        }
        if negate {
            return Err(ParseError::UnexpectedToken {
                got: format!("{:?}", self.peek()), expected: "IN or BETWEEN after NOT".into() });
        }

        let op = match self.advance() {
            Token::Eq   => CompareOp::Eq,  Token::Ne   => CompareOp::Ne,
            Token::Lt   => CompareOp::Lt,  Token::Le   => CompareOp::Le,
            Token::Gt   => CompareOp::Gt,  Token::Ge   => CompareOp::Ge,
            Token::Like => CompareOp::Like,
            o => return Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "comparison operator".into() }),
        };
        let value = self.parse_literal()?;
        Ok(WhereExpr::Comparison(Comparison { column, op, value }))
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
            let name = parse_quoted_label_name(&s)?;
            return validate_insert_label_name(&name);
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

    // -----------------------------------------------------------------
    // UPDATE
    // -----------------------------------------------------------------

    /// UPDATE label.name SET col=val, ... [WHERE ...]
    /// UPDATE LABEL label.old SET label.new [WHERE ...]
    ///
    /// 先頭が `label.` に続けてすぐ `.` が来ない `LABEL` トークン
    /// （= `UPDATE LABEL ...` の `LABEL` キーワード）かどうかで両構文を判別する。
    pub fn parse_update(&mut self) -> Result<UpdateStatement, ParseError> {
        self.expect(Token::Update)?;

        let is_label_rename = matches!(self.peek(), Token::Ident(s) if s.to_lowercase() == "label")
            && self.peek_at(1) != &Token::Dot;

        if is_label_rename {
            self.advance(); // "LABEL" キーワードを消費
            let old_label = self.parse_label_ref()?;
            self.expect(Token::Set)?;
            let new_label = self.parse_label_ref()?;
            let where_clause = if self.peek() == &Token::Where {
                self.advance(); Some(self.parse_where_expr()?)
            } else { None };
            return Ok(UpdateStatement::Label(UpdateLabelStatement { old_label, new_label, where_clause }));
        }

        let target = self.parse_label_target()?;
        self.expect(Token::Set)?;
        let assignments = self.parse_assignment_list()?;
        let where_clause = if self.peek() == &Token::Where {
            self.advance(); Some(self.parse_where_expr()?)
        } else { None };
        Ok(UpdateStatement::Data(UpdateDataStatement { target, assignments, where_clause }))
    }

    /// `label.<name>`（識別子形式）と `'label.<name>'`（文字列リテラル形式）の
    /// 単一ラベル参照をパースする（`label.*` / `'label.*'` は不可）。
    /// UPDATE LABEL / DELETE LABEL 文のラベル名指定に使う。
    fn parse_label_ref(&mut self) -> Result<String, ParseError> {
        if let Token::StringLit(_) = self.peek() {
            let s = match self.advance() { Token::StringLit(s) => s, _ => unreachable!() };
            let name = parse_quoted_label_name(&s)?;
            if name == "*" {
                return Err(ParseError::UnsupportedSyntax(
                    "UPDATE LABEL name cannot be '*'".into()));
            }
            return validate_label_name(&name);
        }
        match self.advance() {
            // `label.name` 形式（"label" の直後が "." のときだけこちらとして扱う）
            Token::Ident(s) if s.to_lowercase() == "label" && self.peek() == &Token::Dot => {
                self.advance(); // "."
                match self.advance() {
                    Token::Star => Err(ParseError::UnsupportedSyntax(
                        "UPDATE LABEL name cannot be '*'".into())),
                    Token::Ident(name) => Ok(name),
                    o => Err(ParseError::UnexpectedToken {
                        got: format!("{:?}", o), expected: "label name".into() }),
                }
            }
            // `label.` を省略した糖衣構文
            Token::Ident(name) => Ok(name),
            o => Err(ParseError::UnexpectedToken {
                got: format!("{:?}", o), expected: "label name".into() }),
        }
    }

    fn parse_assignment_list(&mut self) -> Result<Vec<Assignment>, ParseError> {
        let mut items = vec![self.parse_assignment()?];
        while self.peek() == &Token::Comma {
            self.advance();
            items.push(self.parse_assignment()?);
        }
        Ok(items)
    }

    fn parse_assignment(&mut self) -> Result<Assignment, ParseError> {
        let column = self.expect_ident()?;
        self.expect(Token::Eq)?;
        let value = self.parse_literal()?;
        Ok(Assignment { column, value })
    }

    // -----------------------------------------------------------------
    // DELETE
    // -----------------------------------------------------------------

    /// DELETE FROM label.name [WHERE ...]  |  DELETE FROM label.* [WHERE ...]
    /// DELETE LABEL FROM label.name [WHERE ...]
    ///
    /// UPDATE LABEL と同様、`DELETE` の直後が `.` を伴わない `LABEL` トークンかどうかで
    /// 両構文を判別する。
    pub fn parse_delete(&mut self) -> Result<DeleteStatement, ParseError> {
        self.expect(Token::Delete)?;

        let is_label_delete = matches!(self.peek(), Token::Ident(s) if s.to_lowercase() == "label")
            && self.peek_at(1) != &Token::Dot;

        if is_label_delete {
            self.advance(); // "LABEL" キーワードを消費
            self.expect(Token::From)?;
            let label = self.parse_label_ref()?;
            let where_clause = if self.peek() == &Token::Where {
                self.advance(); Some(self.parse_where_expr()?)
            } else { None };
            return Ok(DeleteStatement::Label(DeleteLabelStatement { label, where_clause }));
        }

        self.expect(Token::From)?;
        let target = self.parse_label_target()?;
        let where_clause = if self.peek() == &Token::Where {
            self.advance(); Some(self.parse_where_expr()?)
        } else { None };
        Ok(DeleteStatement::Data(DeleteDataStatement { target, where_clause }))
    }
}

/// `'label.<name>'` 形式の文字列リテラルから `<name>` 部分を取り出す（`'label.*'` は
/// `name == "*"` として返す。空・`*` かどうかの判定は呼び出し側の用途ごとに行う）。
/// 識別子名が集計関数名（大文字小文字無視）と一致すれば対応する `AggregateFunc` を返す。
/// `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` は予約語化していないため、呼び出し側は直後が `(` の
/// ときだけこの結果を集計関数呼び出しとして採用する（それ以外は通常の列名として扱う）。
fn aggregate_func_from_name(name: &str) -> Option<AggregateFunc> {
    match name.to_uppercase().as_str() {
        "COUNT" => Some(AggregateFunc::Count),
        "SUM"   => Some(AggregateFunc::Sum),
        "AVG"   => Some(AggregateFunc::Avg),
        "MIN"   => Some(AggregateFunc::Min),
        "MAX"   => Some(AggregateFunc::Max),
        _       => None,
    }
}

fn parse_quoted_label_name(s: &str) -> Result<String, ParseError> {
    s.strip_prefix("label.")
        .map(|name| name.to_string())
        .ok_or_else(|| ParseError::UnsupportedSyntax(
            format!("label must be in 'label.<name>' form, got '{}'", s)))
}

/// ラベル名として最低限の妥当性（空文字でない）を検証する。
fn validate_label_name(name: &str) -> Result<String, ParseError> {
    if name.is_empty() {
        return Err(ParseError::UnsupportedSyntax("label name cannot be empty".into()));
    }
    Ok(name.to_string())
}

fn validate_insert_label_name(name: &str) -> Result<String, ParseError> {
    if name == "*" {
        return Err(ParseError::UnsupportedSyntax(
            "INSERT label name cannot be empty or '*'".into()));
    }
    validate_label_name(name)
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

/// UPDATE文をパースする公開エントリーポイント
pub fn parse_update(sql: &str) -> Result<UpdateStatement, ParseError> {
    if sql.trim().is_empty() { return Err(ParseError::EmptyQuery); }
    let tokens = Lexer::new(sql).tokenize()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_update()?;
    parser.expect_eof()?;
    Ok(stmt)
}

/// DELETE文をパースする公開エントリーポイント
pub fn parse_delete(sql: &str) -> Result<DeleteStatement, ParseError> {
    if sql.trim().is_empty() { return Err(ParseError::EmptyQuery); }
    let tokens = Lexer::new(sql).tokenize()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_delete()?;
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
    fn test_select_rejects_colon_in_identifier_label() {
        // `:` は識別子形式の文字として使えないため、量化されていない `label.country:Japan`
        // はトークン化の時点でエラーになる（文字列リテラル形式を使う必要がある）。
        assert!(parse_select("SELECT * FROM label.country:Japan").is_err());
    }
    #[test]
    fn test_select_from_quoted_label_with_colon() {
        // `:` など識別子に使えない文字を含むラベルは 'label.<name>' 形式で指定できる
        // （INSERT で既に使える文字列リテラル形式を SELECT/UPDATE/DELETE にも拡張したもの）。
        let s = parse_select("SELECT * FROM 'label.country:Japan'").unwrap();
        assert_eq!(s.from, FromClause::Label(LabelTarget::LabelName("country:Japan".into())));
    }
    #[test]
    fn test_select_from_quoted_label_star() {
        let s = parse_select("SELECT * FROM 'label.*'").unwrap();
        assert_eq!(s.from, FromClause::Label(LabelTarget::All));
    }
    #[test]
    fn test_select_and_quoted_and_identifier_labels() {
        // 識別子形式と文字列リテラル形式を AND で混在させても解釈できる
        let s = parse_select("SELECT * FROM label.customer AND 'label.category:drink'").unwrap();
        assert_eq!(s.from, FromClause::And(vec![
            LabelTarget::LabelName("customer".into()),
            LabelTarget::LabelName("category:drink".into()),
        ]));
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
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Column(ColumnItem { column: "name".into(), alias: None }),
            SelectItem::Column(ColumnItem { column: "age".into(), alias: None }),
        ]));
    }
    #[test]
    fn test_select_column_alias() {
        let s = parse_select("SELECT age AS employee_age FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Column(ColumnItem { column: "age".into(), alias: Some("employee_age".into()) }),
        ]));
    }
    #[test]
    fn test_select_count_star() {
        let s = parse_select("SELECT COUNT(*) FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Count, arg: AggregateArg::Star, alias: None }),
        ]));
    }
    #[test]
    fn test_select_aggregate_with_alias_and_group_by() {
        let s = parse_select(
            "SELECT department, COUNT(*) AS n, AVG(age) AS avg_age FROM label.employee GROUP BY department"
        ).unwrap();
        assert_eq!(s.group_by, vec!["department".to_string()]);
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Column(ColumnItem { column: "department".into(), alias: None }),
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Count, arg: AggregateArg::Star, alias: Some("n".into()) }),
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Avg, arg: AggregateArg::Column("age".into()), alias: Some("avg_age".into()) }),
        ]));
    }
    #[test]
    fn test_select_group_by_multiple_columns() {
        let s = parse_select("SELECT a, b, COUNT(*) FROM label.x GROUP BY a, b").unwrap();
        assert_eq!(s.group_by, vec!["a".to_string(), "b".to_string()]);
    }
    #[test]
    fn test_select_having() {
        let s = parse_select(
            "SELECT department, COUNT(*) AS n FROM label.employee GROUP BY department HAVING n > 5"
        ).unwrap();
        assert_eq!(s.having, Some(WhereExpr::Comparison(Comparison {
            column: "n".into(), op: CompareOp::Gt, value: LiteralValue::Integer(5),
        })));
    }
    #[test]
    fn test_select_distinct() {
        let s = parse_select("SELECT DISTINCT department FROM label.employee").unwrap();
        assert!(s.distinct);
    }
    #[test]
    fn test_select_sum_avg_min_max() {
        let s = parse_select("SELECT SUM(age), AVG(age), MIN(age), MAX(age) FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Sum, arg: AggregateArg::Column("age".into()), alias: None }),
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Avg, arg: AggregateArg::Column("age".into()), alias: None }),
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Min, arg: AggregateArg::Column("age".into()), alias: None }),
            SelectItem::Aggregate(AggregateItem { func: AggregateFunc::Max, arg: AggregateArg::Column("age".into()), alias: None }),
        ]));
    }
    #[test]
    fn test_select_sum_star_is_error() {
        assert!(parse_select("SELECT SUM(*) FROM label.employee").is_err());
    }
    #[test]
    fn test_select_count_as_plain_column_name_still_works() {
        // COUNT/SUM/AVG/MIN/MAX は予約語化していないため、"(" が続かなければ通常の列名として扱える
        let s = parse_select("SELECT count FROM label.employee").unwrap();
        assert_eq!(s.columns, SelectColumns::Named(vec![
            SelectItem::Column(ColumnItem { column: "count".into(), alias: None }),
        ]));
    }
    #[test]
    fn test_select_star_rejects_group_by() {
        assert!(parse_select("SELECT * FROM label.employee GROUP BY department").is_err());
    }
    #[test]
    fn test_select_star_rejects_having() {
        assert!(parse_select("SELECT * FROM label.employee HAVING department = 'sales'").is_err());
    }
    #[test]
    fn test_select_order_by_limit() {
        let s = parse_select(
            "SELECT * FROM label.employee ORDER BY age DESC LIMIT 10"
        ).unwrap();
        assert_eq!(s.order_by.len(), 1);
        assert_eq!(s.order_by[0].direction, OrderDirection::Desc);
        assert_eq!(s.limit, Some(10));
        assert_eq!(s.offset, None);
    }
    #[test]
    fn test_select_limit_offset() {
        let s = parse_select("SELECT * FROM label.employee LIMIT 10 OFFSET 20").unwrap();
        assert_eq!(s.limit, Some(10));
        assert_eq!(s.offset, Some(20));
    }
    #[test]
    fn test_select_offset_without_limit() {
        let s = parse_select("SELECT * FROM label.employee OFFSET 5").unwrap();
        assert_eq!(s.limit, None);
        assert_eq!(s.offset, Some(5));
    }
    #[test]
    fn test_select_from_bare_label_name_sugar() {
        // `label.` を省略した糖衣構文: FROM race は FROM label.race と同義
        let s = parse_select("SELECT * FROM employee").unwrap();
        assert_eq!(s.from, FromClause::Label(LabelTarget::LabelName("employee".into())));
    }
    #[test]
    fn test_select_where_is_null() {
        let s = parse_select("SELECT * FROM label.employee WHERE department IS NULL").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::IsNull(IsNullExpr { column: "department".into() })));
    }
    #[test]
    fn test_select_where_is_not_null() {
        let s = parse_select("SELECT * FROM label.employee WHERE department IS NOT NULL").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Not(Box::new(
            WhereExpr::IsNull(IsNullExpr { column: "department".into() })
        ))));
    }
    #[test]
    fn test_select_where_in() {
        let s = parse_select("SELECT * FROM label.employee WHERE department IN ('sales', 'dev')").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::In(InExpr {
            column: "department".into(),
            values: vec![LiteralValue::Text("sales".into()), LiteralValue::Text("dev".into())],
        })));
    }
    #[test]
    fn test_select_where_not_in() {
        let s = parse_select("SELECT * FROM label.employee WHERE department NOT IN ('sales')").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Not(Box::new(
            WhereExpr::In(InExpr { column: "department".into(), values: vec![LiteralValue::Text("sales".into())] })
        ))));
    }
    #[test]
    fn test_select_where_between() {
        let s = parse_select("SELECT * FROM label.employee WHERE age BETWEEN 20 AND 30").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Between(BetweenExpr {
            column: "age".into(), low: LiteralValue::Integer(20), high: LiteralValue::Integer(30),
        })));
    }
    #[test]
    fn test_select_where_not_between() {
        let s = parse_select("SELECT * FROM label.employee WHERE age NOT BETWEEN 20 AND 30").unwrap();
        assert_eq!(s.where_clause, Some(WhereExpr::Not(Box::new(
            WhereExpr::Between(BetweenExpr { column: "age".into(), low: LiteralValue::Integer(20), high: LiteralValue::Integer(30) })
        ))));
    }
    #[test]
    fn test_select_where_not_in_without_in_is_error() {
        assert!(parse_select("SELECT * FROM label.employee WHERE age NOT 30").is_err());
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

    // -- UPDATE --

    #[test]
    fn test_update_data_with_where() {
        let s = parse_update(
            "UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'"
        ).unwrap();
        match s {
            UpdateStatement::Data(d) => {
                assert_eq!(d.target, LabelTarget::LabelName("employee".into()));
                assert_eq!(d.assignments, vec![Assignment {
                    column: "employee_name".into(), value: LiteralValue::Text("木村".into()),
                }]);
                assert_eq!(d.where_clause, Some(WhereExpr::Comparison(Comparison {
                    column: "employee_name".into(), op: CompareOp::Eq,
                    value: LiteralValue::Text("木邑".into()),
                })));
            }
            _ => panic!("expected UpdateStatement::Data"),
        }
    }

    #[test]
    fn test_update_data_multiple_assignments_without_where() {
        let s = parse_update("UPDATE label.employee SET age=30, department='sales'").unwrap();
        match s {
            UpdateStatement::Data(d) => {
                assert_eq!(d.assignments, vec![
                    Assignment { column: "age".into(), value: LiteralValue::Integer(30) },
                    Assignment { column: "department".into(), value: LiteralValue::Text("sales".into()) },
                ]);
                assert_eq!(d.where_clause, None);
            }
            _ => panic!("expected UpdateStatement::Data"),
        }
    }

    #[test]
    fn test_update_label_rename() {
        let s = parse_update("UPDATE LABEL label.employee SET label.staff").unwrap();
        match s {
            UpdateStatement::Label(l) => {
                assert_eq!(l.old_label, "employee");
                assert_eq!(l.new_label, "staff");
                assert_eq!(l.where_clause, None);
            }
            _ => panic!("expected UpdateStatement::Label"),
        }
    }

    #[test]
    fn test_update_label_rename_with_where() {
        let s = parse_update(
            "UPDATE LABEL label.employee SET label.staff WHERE department='sales'"
        ).unwrap();
        match s {
            UpdateStatement::Label(l) => {
                assert_eq!(l.old_label, "employee");
                assert_eq!(l.new_label, "staff");
                assert_eq!(l.where_clause, Some(WhereExpr::Comparison(Comparison {
                    column: "department".into(), op: CompareOp::Eq,
                    value: LiteralValue::Text("sales".into()),
                })));
            }
            _ => panic!("expected UpdateStatement::Label"),
        }
    }

    #[test]
    fn test_update_label_rejects_star() {
        assert!(parse_update("UPDATE LABEL label.* SET label.staff").is_err());
        assert!(parse_update("UPDATE LABEL label.employee SET label.*").is_err());
    }

    #[test]
    fn test_update_label_rename_quoted_form() {
        // `:` を含むラベルは UPDATE LABEL の旧名・新名どちらも文字列リテラル形式で指定できる
        let s = parse_update(
            "UPDATE LABEL 'label.country:Japan' SET 'label.country:JP'"
        ).unwrap();
        match s {
            UpdateStatement::Label(l) => {
                assert_eq!(l.old_label, "country:Japan");
                assert_eq!(l.new_label, "country:JP");
            }
            _ => panic!("expected UpdateStatement::Label"),
        }
    }

    #[test]
    fn test_update_label_quoted_form_rejects_star() {
        assert!(parse_update("UPDATE LABEL 'label.*' SET label.staff").is_err());
        assert!(parse_update("UPDATE LABEL label.employee SET 'label.*'").is_err());
    }

    #[test]
    fn test_update_data_target_quoted_form() {
        let s = parse_update("UPDATE 'label.country:Japan' SET active=true").unwrap();
        match s {
            UpdateStatement::Data(d) => assert_eq!(d.target, LabelTarget::LabelName("country:Japan".into())),
            _ => panic!("expected UpdateStatement::Data"),
        }
    }

    #[test]
    fn test_update_data_target_bare_name_sugar() {
        let s = parse_update("UPDATE employee SET active=true").unwrap();
        match s {
            UpdateStatement::Data(d) => assert_eq!(d.target, LabelTarget::LabelName("employee".into())),
            _ => panic!("expected UpdateStatement::Data"),
        }
    }

    #[test]
    fn test_update_rejects_trailing_tokens() {
        assert!(parse_update("UPDATE label.employee SET age=30 GARBAGE").is_err());
    }

    // -- DELETE --

    #[test]
    fn test_delete_data_with_where() {
        let s = parse_delete("DELETE FROM label.employee WHERE employee_name='田中'").unwrap();
        match s {
            DeleteStatement::Data(d) => {
                assert_eq!(d.target, LabelTarget::LabelName("employee".into()));
                assert_eq!(d.where_clause, Some(WhereExpr::Comparison(Comparison {
                    column: "employee_name".into(), op: CompareOp::Eq,
                    value: LiteralValue::Text("田中".into()),
                })));
            }
            _ => panic!("expected DeleteStatement::Data"),
        }
    }

    #[test]
    fn test_delete_data_without_where_targets_whole_label() {
        let s = parse_delete("DELETE FROM label.employee").unwrap();
        match s {
            DeleteStatement::Data(d) => {
                assert_eq!(d.target, LabelTarget::LabelName("employee".into()));
                assert_eq!(d.where_clause, None);
            }
            _ => panic!("expected DeleteStatement::Data"),
        }
    }

    #[test]
    fn test_delete_data_from_label_star_targets_all_records() {
        let s = parse_delete("DELETE FROM label.*").unwrap();
        match s {
            DeleteStatement::Data(d) => {
                assert_eq!(d.target, LabelTarget::All);
                assert_eq!(d.where_clause, None);
            }
            _ => panic!("expected DeleteStatement::Data"),
        }
    }

    #[test]
    fn test_delete_label_only_removes_label() {
        let s = parse_delete("DELETE LABEL FROM label.employee WHERE department='sales'").unwrap();
        match s {
            DeleteStatement::Label(l) => {
                assert_eq!(l.label, "employee");
                assert_eq!(l.where_clause, Some(WhereExpr::Comparison(Comparison {
                    column: "department".into(), op: CompareOp::Eq,
                    value: LiteralValue::Text("sales".into()),
                })));
            }
            _ => panic!("expected DeleteStatement::Label"),
        }
    }

    #[test]
    fn test_delete_label_without_where() {
        let s = parse_delete("DELETE LABEL FROM label.employee").unwrap();
        match s {
            DeleteStatement::Label(l) => {
                assert_eq!(l.label, "employee");
                assert_eq!(l.where_clause, None);
            }
            _ => panic!("expected DeleteStatement::Label"),
        }
    }

    #[test]
    fn test_delete_data_target_quoted_form() {
        let s = parse_delete("DELETE FROM 'label.country:Japan'").unwrap();
        match s {
            DeleteStatement::Data(d) => assert_eq!(d.target, LabelTarget::LabelName("country:Japan".into())),
            _ => panic!("expected DeleteStatement::Data"),
        }
    }

    #[test]
    fn test_delete_data_target_bare_name_sugar() {
        let s = parse_delete("DELETE FROM employee").unwrap();
        match s {
            DeleteStatement::Data(d) => assert_eq!(d.target, LabelTarget::LabelName("employee".into())),
            _ => panic!("expected DeleteStatement::Data"),
        }
    }

    #[test]
    fn test_delete_label_quoted_form() {
        let s = parse_delete("DELETE LABEL FROM 'label.country:Japan'").unwrap();
        match s {
            DeleteStatement::Label(l) => assert_eq!(l.label, "country:Japan"),
            _ => panic!("expected DeleteStatement::Label"),
        }
    }

    #[test]
    fn test_delete_label_quoted_form_rejects_star() {
        assert!(parse_delete("DELETE LABEL FROM 'label.*'").is_err());
    }

    #[test]
    fn test_delete_label_rejects_star() {
        assert!(parse_delete("DELETE LABEL FROM label.*").is_err());
    }

    #[test]
    fn test_delete_rejects_trailing_tokens() {
        assert!(parse_delete("DELETE FROM label.employee GARBAGE").is_err());
    }

    #[test]
    fn test_delete_case_insensitive() {
        let s = parse_delete("delete from label.employee where name = 'Alice'").unwrap();
        assert!(matches!(s, DeleteStatement::Data(_)));
    }
}


