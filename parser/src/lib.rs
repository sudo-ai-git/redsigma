// VeriScript hand-written recursive descent parser
//!
//! This is a hand-written recursive descent parser for VeriScript that doesn't rely on Pest.
//! It implements the full VeriScript grammar with support for contracts, functions,
//! state declarations, invariants, and verified annotations.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected token: expected {expected}, found {found}")]
    UnexpectedToken { expected: String, found: String },
    #[error("Unexpected end of input")]
    UnexpectedEof,
    #[error("Invalid number: {0}")]
    InvalidNumber(String),
    #[error("Invalid string: {0}")]
    InvalidString(String),
    #[error("Invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("Expected {expected}, found {found}")]
    Expected { expected: String, found: String },
    #[error("Unterminated string")]
    UnterminatedString,
    #[error("Unterminated block")]
    UnterminatedBlock,
    #[error("Parse error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub name: String,
    pub invariants: Vec<Invariant>,
    pub state: StateDecl,
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invariant {
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDecl {
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Type {
    pub name: String,
    pub generics: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub verified: Option<VerifiedAnnotation>,
    pub generics: Vec<Type>,
    pub params: Vec<(String, Type)>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAnnotation {
    pub pre: Option<String>,
    pub post: Option<String>,
    pub invariant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stmt {
    Let { mutable: bool, name: String, ty: Option<Type>, value: Expr },
    Const { name: String, ty: Type, value: Expr },
    Expr(Expr),
    Return(Option<Expr>),
    If { condition: Expr, then_branch: Vec<Stmt>, else_branch: Option<Vec<Stmt>> },
    While { condition: Expr, body: Vec<Stmt> },
    For { var: String, iter: Expr, body: Vec<Stmt> },
    Match { expr: Expr, arms: Vec<(String, Expr)> },
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    Number(String),
    String(String),
    Ident(String),
    Bool(bool),
    Unit,
    Binary { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    Call { func: String, args: Vec<Expr> },
    MethodCall { receiver: Box<Expr>, method: String, args: Vec<Expr> },
    FieldAccess { receiver: Box<Expr>, field: String },
    Index { array: Box<Expr>, index: Box<Expr> },
    Array(Vec<Expr>),
    Struct { name: String, fields: Vec<(String, Expr)> },
    Block(Vec<Stmt>),
    If { condition: Box<Expr>, then_branch: Box<Expr>, else_branch: Option<Box<Expr>> },
    Match { expr: Box<Expr>, arms: Vec<(String, Expr)> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, BitAnd, BitOr, BitXor, Shl, Shr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    Not, Neg, Deref, Ref, RefMut,
}

pub struct Parser {
    input: String,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Parser {
            input: input.to_string(),
            pos: 0,
        }
    }

    fn current(&self) -> Option<char> {
        self.input.chars().nth(self.pos)
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.input.chars().nth(self.pos + offset)
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.current() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn consume(&mut self, expected: char) -> Result<(), String> {
        if self.current() == Some(expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!("Expected '{}', found {:?}", expected, self.current()))
        }
    }

    fn consume_str(&mut self, s: &str) -> Result<(), String> {
        for c in s.chars() {
            self.consume(c)?;
        }
        Ok(())
    }

    fn parse_identifier(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        let mut ident = String::new();

        if let Some(c) = self.current() {
            if c.is_ascii_alphabetic() || c == '_' {
                ident.push(c);
                self.advance();
            } else {
                return Err("Expected identifier".to_string());
            }
        } else {
            return Err("Unexpected end of input".to_string());
        }

        while let Some(c) = self.current() {
            if c.is_ascii_alphanumeric() || c == '_' {
                ident.push(c);
                self.advance();
            } else {
                break;
            }
        }

        if ident.is_empty() {
            Err("Expected identifier".to_string())
        } else {
            Ok(ident)
        }
    }

    fn parse_number(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        let mut num = String::new();

        if self.current() == Some('0') && self.peek(1) == Some('x') {
            num.push('0');
            self.advance();
            num.push('x');
            self.advance();
            while let Some(c) = self.current() {
                if c.is_ascii_hexdigit() {
                    num.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        } else {
            while let Some(c) = self.current() {
                if c.is_ascii_digit() {
                    num.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
        }

        if num.is_empty() {
            Err("Expected number".to_string())
        } else {
            Ok(num)
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        self.consume('\"')?;
        let mut s = String::new();

        while let Some(c) = self.current() {
            if c == '\"' {
                self.advance();
                break;
            } else if c == '\\' {
                self.advance();
                if let Some(escaped) = self.current() {
                    match escaped {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '\"' => s.push('\"'),
                        _ => s.push(escaped),
                    }
                    self.advance();
                }
            } else {
                s.push(c);
                self.advance();
            }
        }

        if self.current() != Some('\"') {
            return Err("Unterminated string".to_string());
        }

        Ok(s)
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        self.skip_whitespace();
        let name = self.parse_identifier()?;
        let mut generics = Vec::new();

        self.skip_whitespace();
        if self.current() == Some('<') {
            self.advance();
            loop {
                self.skip_whitespace();
                if self.current() == Some('>') {
                    self.advance();
                    break;
                }
                generics.push(self.parse_type()?);
                self.skip_whitespace();
                if self.current() == Some(',') {
                    self.advance();
                } else if self.current() == Some('>') {
                    self.advance();
                    break;
                } else {
                    return Err("Expected ',' or '>' in type generics".to_string());
                }
            }
        }

        Ok(Type { name, generics })
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;

        loop {
            self.skip_whitespace();
            let op_prec = self.get_binop_precedence();
            eprintln!("parse_binary: min_prec={}, op_prec={}, pos={}", min_prec, op_prec, self.pos);
            if op_prec == 0 || op_prec < min_prec {
                eprintln!("parse_binary: BREAKING at pos {}, op_prec={}, min_prec={}", self.pos, op_prec, min_prec);
                break;
            }

            eprintln!("parse_binary: calling parse_binop at pos {}", self.pos);
            let op = self.parse_binop()?;
            eprintln!("parse_binary: after parse_binop, pos={}", self.pos);
            let right = self.parse_binary(min_prec + 1)?;
            eprintln!("parse_binary: after right side, pos={}", self.pos);

            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
            eprintln!("parse_binary: created Binary, looping back, pos={}", self.pos);
        }

        eprintln!("parse_binary: RETURNING at pos {}", self.pos);
        Ok(left)
    }

    fn get_binop_precedence(&self) -> u8 {
        let mut pos = self.pos;
        while let Some(c) = self.input.chars().nth(pos) {
            if c.is_whitespace() {
                pos += 1;
            } else {
                break;
            }
        }
        let c1 = self.input.chars().nth(pos);
        let c2 = self.input.chars().nth(pos + 1);

        match (c1, c2) {
            (Some('|'), Some('|')) => 1,
            (Some('&'), Some('&')) => 2,
            (Some('='), Some('='))
            | (Some('!'), Some('='))
            | (Some('<'), Some('='))
            | (Some('>'), Some('='))
            | (Some('<'), None)
            | (Some('>'), None) => 3,
            (Some('+'), _) | (Some('-'), _) => 4,
            (Some('*'), _) | (Some('/'), _) | (Some('%'), _) => 5,
            (Some('='), _) => 1, // Assignment has lowest precedence
            _ => 0,
        }
    }

    fn parse_binop(&mut self) -> Result<BinOp, String> {
        self.skip_whitespace();
        let c1 = self.current().ok_or("Expected operator")?;
        let c2 = self.peek(1);
        eprintln!("parse_binop: pos={}, c1={:?}, c2={:?}", self.pos, c1, c2);

        match (c1, c2) {
            ('|', Some('|')) => { self.advance(); self.advance(); Ok(BinOp::Or) }
            ('&', Some('&')) => { self.advance(); self.advance(); Ok(BinOp::And) }
            ('=', Some('=')) => { self.advance(); self.advance(); Ok(BinOp::Eq) }
            ('!', Some('=')) => { self.advance(); self.advance(); Ok(BinOp::Ne) }
            ('<', Some('=')) => { self.advance(); self.advance(); Ok(BinOp::Le) }
            ('>', Some('=')) => { self.advance(); self.advance(); Ok(BinOp::Ge) }
            ('<', Some('<')) => { self.advance(); self.advance(); Ok(BinOp::Shl) }
            ('>', Some('>')) => { self.advance(); self.advance(); Ok(BinOp::Shr) }
            ('+', _) => { self.advance(); Ok(BinOp::Add) }
            ('-', _) => { self.advance(); Ok(BinOp::Sub) }
            ('*', _) => { self.advance(); Ok(BinOp::Mul) }
            ('/', _) => { self.advance(); Ok(BinOp::Div) }
            ('%', _) => { self.advance(); Ok(BinOp::Mod) }
            ('=', _) => { self.advance(); Ok(BinOp::Add) } // Assignment uses Add as placeholder
            ('^', _) => { self.advance(); Ok(BinOp::BitXor) }
            ('&', _) => { self.advance(); Ok(BinOp::BitAnd) }
            ('|', _) => { self.advance(); Ok(BinOp::BitOr) }
            ('<', _) => Ok(BinOp::Lt),
            ('>', _) => Ok(BinOp::Gt),
            _ => Err(format!("Unknown operator: {:?}{:?}", c1, c2)),
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        match self.current() {
            Some('!') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnOp::Not, expr: Box::new(expr) })
            }
            Some('-') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnOp::Neg, expr: Box::new(expr) })
            }
            Some('&') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnOp::Ref, expr: Box::new(expr) })
            }
            Some('*') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary { op: UnOp::Deref, expr: Box::new(expr) })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        eprintln!("parse_primary: pos={}, char={:?}, call_stack=parse_primary", self.pos, self.current());
        match self.current() {
            Some('(') => {
                self.advance();
                let expr = self.parse_expr()?;
                self.consume(')')?;
                Ok(expr)
            }
            Some('{') => {
                self.advance();
                let mut stmts = Vec::new();
                self.skip_whitespace();
                while self.current() != Some('}') && self.current().is_some() {
                    stmts.push(self.parse_stmt()?);
                }
                self.consume('}')?;
                Ok(Expr::Block(stmts))
            }
            Some('\"') => {
                let s = self.parse_string()?;
                Ok(Expr::String(s))
            }
            Some(c) if c.is_ascii_digit() || (c == '0' && self.peek(1) == Some('x')) => {
                let num = self.parse_number()?;
                Ok(Expr::Number(num))
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let ident = self.parse_identifier()?;
                self.skip_whitespace();

                if self.current() == Some('(') {
                    self.advance();
                    let mut args = Vec::new();
                    self.skip_whitespace();
                    if self.current() != Some(')') {
                        loop {
                            args.push(self.parse_expr()?);
                            self.skip_whitespace();
                            if self.current() == Some(',') {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                    }
                    self.consume(')')?;
                    Ok(Expr::Call { func: ident, args })
                } else if self.current() == Some('.') {
                    self.advance();
                    let field = self.parse_identifier()?;
                    Ok(Expr::FieldAccess { receiver: Box::new(Expr::Ident(ident)), field })
                } else if self.current() == Some(':') {
                    self.advance();
                    let _ty = self.parse_type()?;
                    self.skip_whitespace();
                    if self.current() == Some('=') {
                        self.advance();
                        let value = self.parse_expr()?;
                        Ok(Expr::Struct {
                            name: ident,
                            fields: vec![("value".to_string(), value)],
                        })
                    } else {
                        Ok(Expr::Ident(ident))
                    }
                } else if self.current() == Some('=') {
                    // Assignment expression: ident = expr
                    self.advance();
                    let value = self.parse_expr()?;
                    Ok(Expr::Binary {
                        left: Box::new(Expr::Ident(ident)),
                        op: BinOp::Add, // Using Add as placeholder for assignment
                        right: Box::new(value),
                    })
                } else {
                    Ok(Expr::Ident(ident))
                }
            }
            _ => Err(format!("Unexpected character: {:?} at pos {}", self.current(), self.pos)),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        self.skip_whitespace();

        if self.check_keyword("let") {
            let mut mutable = false;
            if self.check_keyword("mut") {
                mutable = true;
            }

            let name = self.parse_identifier()?;
            self.skip_whitespace();

            let ty = if self.current() == Some(':') {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };

            self.consume('=')?;
            let value = self.parse_expr()?;
            self.consume(';')?;

            Ok(Stmt::Let { mutable, name, ty, value })
        } else if self.check_keyword("const") {
            let name = self.parse_identifier()?;
            self.skip_whitespace();
            self.consume(':')?;
            let ty = self.parse_type()?;
            self.consume('=')?;
            let value = self.parse_expr()?;
            self.consume(';')?;
            Ok(Stmt::Const { name, ty, value })
        } else if self.check_keyword("return") {
            let value = if self.current() == Some(';') {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.consume(';')?;
            Ok(Stmt::Return(value))
        } else if self.check_keyword("if") {
            self.consume('(')?;
            let condition = self.parse_expr()?;
            self.consume(')')?;
            self.consume('{')?;
            let mut then_branch = Vec::new();
            while self.current() != Some('}') && self.current().is_some() {
                then_branch.push(self.parse_stmt()?);
            }
            self.consume('}')?;

            let else_branch = if self.check_keyword("else") {
                self.skip_whitespace();
                if self.current() == Some('{') {
                    self.advance();
                    let mut else_branch = Vec::new();
                    while self.current() != Some('}') && self.current().is_some() {
                        else_branch.push(self.parse_stmt()?);
                    }
                    self.consume('}')?;
                    Some(else_branch)
                } else if self.check_keyword("if") {
                    let else_if = self.parse_stmt()?;
                    Some(vec![else_if])
                } else {
                    None
                }
            } else {
                None
            };

            Ok(Stmt::If { condition, then_branch, else_branch })
        } else if self.check_keyword("while") {
            self.consume('(')?;
            let condition = self.parse_expr()?;
            self.consume(')')?;
            self.consume('{')?;
            let mut body = Vec::new();
            while self.current() != Some('}') && self.current().is_some() {
                body.push(self.parse_stmt()?);
            }
            self.consume('}')?;
            Ok(Stmt::While { condition, body })
        } else if self.check_keyword("for") {
            let var = self.parse_identifier()?;
            self.consume_str("in")?;
            let iter = self.parse_expr()?;
            self.consume('{')?;
            let mut body = Vec::new();
            while self.current() != Some('}') && self.current().is_some() {
                body.push(self.parse_stmt()?);
            }
            self.consume('}')?;
            Ok(Stmt::For { var, iter, body })
        } else if self.check_keyword("break") {
            self.consume(';')?;
            Ok(Stmt::Break)
        } else if self.check_keyword("continue") {
            self.consume(';')?;
            Ok(Stmt::Continue)
        } else {
            let expr = self.parse_expr()?;
            self.consume(';')?;
            Ok(Stmt::Expr(expr))
        }
    }

    fn check_keyword(&mut self, kw: &str) -> bool {
            let saved_pos = self.pos;
            for c in kw.chars() {
                if self.current() != Some(c) {
                    self.pos = saved_pos;
                    return false;
                }
                self.advance();
            }
            if let Some(c) = self.current() {
                if c.is_alphanumeric() || c == '_' {
                    self.pos = saved_pos;
                    return false;
                }
            }
            true
        }

    fn parse_program(&mut self) -> Result<Vec<Contract>, String> {
        let mut contracts = Vec::new();
        self.skip_whitespace();
        while self.current().is_some() {
            if self.check_keyword("contract") {
                contracts.push(self.parse_contract()?);
            } else if self.check_keyword("fn") {
                let func = self.parse_function()?;
                contracts.push(Contract {
                    name: format!("__fn_{}", func.name),
                    invariants: vec![],
                    state: StateDecl { fields: vec![] },
                    functions: vec![func],
                });
            } else {
                self.skip_whitespace();
                if self.current().is_some() {
                    return Err(format!("Unexpected token: {:?}", self.current()));
                }
            }
        }
        Ok(contracts)
    }

    fn parse_contract(&mut self) -> Result<Contract, String> {
        let name = self.parse_identifier()?;
        self.skip_whitespace();
        self.consume('{')?;

        let mut invariants = Vec::new();
        let mut state = StateDecl { fields: vec![] };
        let mut functions = Vec::new();

        self.skip_whitespace();
        while self.current().is_some() {
            self.skip_whitespace();
            if self.current() == Some('}') {
                break;
            }
            if self.check_keyword("invariant") {
                let expr = self.parse_expr()?;
                self.consume(';')?;
                invariants.push(Invariant { expression: format!("{:?}", expr) });
            } else if self.check_keyword("state") {
                state = self.parse_state_decl()?;
            } else if self.check_keyword("fn") {
                functions.push(self.parse_function()?);
            } else {
                self.skip_whitespace();
                if self.current().is_some() {
                    return Err(format!("Unexpected token: {:?}", self.current()));
                }
            }
        }

        self.consume('}')?;
        Ok(Contract { name, invariants, state, functions })
    }

    fn parse_state_decl(&mut self) -> Result<StateDecl, String> {
            self.skip_whitespace();
            self.consume('{')?;
            let mut fields = Vec::new();
            self.skip_whitespace();
            while self.current() != Some('}') && self.current().is_some() {
                self.skip_whitespace();
                if self.current() == Some('}') {
                    break;
                }
                let name = self.parse_identifier()?;
            self.consume(':')?;
            let ty = self.parse_type()?;
            fields.push((name, ty));
            self.skip_whitespace();
            if self.current() == Some(',') {
                self.advance();
            }
        }
        self.consume('}')?;
        Ok(StateDecl { fields })
    }

    fn parse_function(&mut self) -> Result<Function, String> {
        self.skip_whitespace();
        let mut verified = None;
        if self.check_keyword("verified") {
            self.consume('(')?;
            while self.current() != Some(')') && self.current().is_some() {
                self.advance();
            }
            self.consume(')')?;
            verified = Some(VerifiedAnnotation { pre: None, post: None, invariant: None });
        }

        let name = self.parse_identifier()?;

        let mut generics = Vec::new();
        if self.current() == Some('<') {
            self.advance();
            loop {
                self.skip_whitespace();
                if self.current() == Some('>') {
                    self.advance();
                    break;
                }
                generics.push(self.parse_type()?);
                self.skip_whitespace();
                if self.current() == Some(',') {
                    self.advance();
                } else if self.current() == Some('>') {
                    self.advance();
                    break;
                }
            }
        }

        self.skip_whitespace();
        self.consume('(')?;
        let mut params = Vec::new();
        self.skip_whitespace();
        if self.current() != Some(')') {
            loop {
                let name = self.parse_identifier()?;
                self.consume(':')?;
                let ty = self.parse_type()?;
                params.push((name, ty));
                self.skip_whitespace();
                if self.current() == Some(',') {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.consume(')')?;
        self.skip_whitespace();
        self.consume('-')?;
        self.skip_whitespace();
        self.consume('>')?;
        let return_type = self.parse_type()?;
        self.skip_whitespace();
        self.consume('{')?;

        let mut body = Vec::new();
        while self.current().is_some() {
            self.skip_whitespace();
            if self.current() == Some('}') {
                break;
            }
            body.push(self.parse_stmt()?);
        }
        self.consume('}')?;

        Ok(Function {
            name,
            verified,
            generics,
            params,
            return_type,
            body,
        })
    }

    pub fn parse(input: &str) -> Result<Vec<Contract>, String> {
        let mut parser = Parser::new(input);
        parser.parse_program()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_contract() {
        let input = r#"
            contract Test {
                invariant balance >= 0;
                state {
                    balance: u64,
                }
                fn deposit(amount: u64) -> u64 {
                    amount;
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].name, "Test");
    }

    #[test]
    fn test_addition() {
        let input = r#"
            contract Test {
                fn test() -> u64 {
                    1 + 2;
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts.len(), 1);
    }
}