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
    /// The parsed invariant expression. It used to be stored as
    /// `format!("{:?}", expr)` — a Rust debug string that no verifier could
    /// evaluate and that changed with the compiler's formatting, so nothing
    /// downstream could honestly check an invariant.
    pub expr: Expr,
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
    /// `pre:` — must hold before the call. Parsed, not a string, so a verifier
    /// can actually reason about it.
    pub pre: Option<Expr>,
    /// `post:` — must hold after the call. `old(e)` is a normal call node; the
    /// verifier binds it to the pre-state value of `e`.
    pub post: Option<Expr>,
    /// `invariant:` — the contract invariant this call must preserve.
    pub invariant: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stmt {
    Let {
        mutable: bool,
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    Const {
        name: String,
        ty: Type,
        value: Expr,
    },
    Expr(Expr),
    Return(Option<Expr>),
    If {
        condition: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    For {
        var: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    Match {
        expr: Expr,
        arms: Vec<(String, Expr)>,
    },
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
    Binary {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Call {
        func: String,
        args: Vec<Expr>,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    FieldAccess {
        receiver: Box<Expr>,
        field: String,
    },
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },
    Array(Vec<Expr>),
    Struct {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    Block(Vec<Stmt>),
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    Match {
        expr: Box<Expr>,
        arms: Vec<(String, Expr)>,
    },
    Range {
        from: Box<Expr>,
        to: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Range,
    /// Single `=` — distinct from `==` (`Eq`). Previously parsed as `Add`,
    /// which made `x = v` indistinguishable from `x + v` and silently dropped
    /// every state mutation.
    Assign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    Not,
    Neg,
    Deref,
    Ref,
    RefMut,
}

/// Maximum nesting depth for expressions and statements. Without this a
/// recursive-descent parser aborts the process with a stack overflow on
/// hostile input (`((((...))))`), which no caller can catch.
///
/// Each nesting level costs two depth units (`parse_expr` + `parse_unary`), so
/// this allows roughly 50 nested parentheses. The limit is deliberately
/// conservative: the parser must not overflow on a default-sized thread stack
/// (~2 MiB), not just the 8 MiB main thread.
const MAX_DEPTH: usize = 100;

/// Maximum AST nesting depth. The source-level guard counts *recursive*
/// nesting, but the postfix index loop (`a[0][0]...`) and the binary-operator
/// loop (`1+1+1+...`) build arbitrarily deep trees iteratively, so a hostile
/// file can still produce a tree that overflows the stack in any recursive
/// walk (semantic projection, dual-stream hashing, codegen, reflection).
const MAX_AST_DEPTH: usize = 200;

enum AstNode<'a> {
    Expr(&'a Expr, usize),
    Stmt(&'a Stmt, usize),
}

/// Iterative AST depth check with an explicit stack (a recursive walk would
/// itself overflow on the input it is trying to reject).
fn ast_depth_ok(contract: &Contract) -> Result<(), String> {
    let mut stack: Vec<AstNode> = Vec::new();
    for inv in &contract.invariants {
        stack.push(AstNode::Expr(&inv.expr, 1));
    }
    for f in &contract.functions {
        for s in &f.body {
            stack.push(AstNode::Stmt(s, 1));
        }
    }
    while let Some(node) = stack.pop() {
        match node {
            AstNode::Expr(expr, depth) => {
                if depth > MAX_AST_DEPTH {
                    return Err(format!(
                        "AST too deeply nested (max depth {})",
                        MAX_AST_DEPTH
                    ));
                }
                match expr {
                    Expr::Binary { left, right, .. } => {
                        stack.push(AstNode::Expr(left, depth + 1));
                        stack.push(AstNode::Expr(right, depth + 1));
                    }
                    Expr::Unary { expr, .. } => stack.push(AstNode::Expr(expr, depth + 1)),
                    Expr::Call { args, .. } => {
                        for a in args {
                            stack.push(AstNode::Expr(a, depth + 1));
                        }
                    }
                    Expr::MethodCall { receiver, args, .. } => {
                        stack.push(AstNode::Expr(receiver, depth + 1));
                        for a in args {
                            stack.push(AstNode::Expr(a, depth + 1));
                        }
                    }
                    Expr::FieldAccess { receiver, .. } => {
                        stack.push(AstNode::Expr(receiver, depth + 1))
                    }
                    Expr::Index { array, index } => {
                        stack.push(AstNode::Expr(array, depth + 1));
                        stack.push(AstNode::Expr(index, depth + 1));
                    }
                    Expr::Array(items) => {
                        for i in items {
                            stack.push(AstNode::Expr(i, depth + 1));
                        }
                    }
                    Expr::Struct { fields, .. } => {
                        for (_, v) in fields {
                            stack.push(AstNode::Expr(v, depth + 1));
                        }
                    }
                    Expr::Block(stmts) => {
                        for s in stmts {
                            stack.push(AstNode::Stmt(s, depth + 1));
                        }
                    }
                    Expr::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        stack.push(AstNode::Expr(condition, depth + 1));
                        stack.push(AstNode::Expr(then_branch, depth + 1));
                        if let Some(e) = else_branch {
                            stack.push(AstNode::Expr(e, depth + 1));
                        }
                    }
                    Expr::Match { expr, arms } => {
                        stack.push(AstNode::Expr(expr, depth + 1));
                        for (_, a) in arms {
                            stack.push(AstNode::Expr(a, depth + 1));
                        }
                    }
                    Expr::Range { from, to } => {
                        stack.push(AstNode::Expr(from, depth + 1));
                        stack.push(AstNode::Expr(to, depth + 1));
                    }
                    _ => {}
                }
            }
            AstNode::Stmt(stmt, depth) => {
                if depth > MAX_AST_DEPTH {
                    return Err(format!(
                        "AST too deeply nested (max depth {})",
                        MAX_AST_DEPTH
                    ));
                }
                match stmt {
                    Stmt::Let { value, .. } => stack.push(AstNode::Expr(value, depth + 1)),
                    Stmt::Const { value, .. } => stack.push(AstNode::Expr(value, depth + 1)),
                    Stmt::Expr(e) => stack.push(AstNode::Expr(e, depth + 1)),
                    Stmt::Return(Some(e)) => stack.push(AstNode::Expr(e, depth + 1)),
                    Stmt::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        stack.push(AstNode::Expr(condition, depth + 1));
                        for s in then_branch {
                            stack.push(AstNode::Stmt(s, depth + 1));
                        }
                        if let Some(eb) = else_branch {
                            for s in eb {
                                stack.push(AstNode::Stmt(s, depth + 1));
                            }
                        }
                    }
                    Stmt::While { condition, body } => {
                        stack.push(AstNode::Expr(condition, depth + 1));
                        for s in body {
                            stack.push(AstNode::Stmt(s, depth + 1));
                        }
                    }
                    Stmt::For { iter, body, .. } => {
                        stack.push(AstNode::Expr(iter, depth + 1));
                        for s in body {
                            stack.push(AstNode::Stmt(s, depth + 1));
                        }
                    }
                    Stmt::Match { expr, arms } => {
                        stack.push(AstNode::Expr(expr, depth + 1));
                        for (_, a) in arms {
                            stack.push(AstNode::Expr(a, depth + 1));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

pub struct Parser {
    /// Source as a char vector: indexing a `String` by `chars().nth(pos)` is
    /// O(pos), which made the whole parser O(n²) on large files.
    input: Vec<char>,
    pos: usize,
    depth: usize,
    /// Set when a `/*` comment is never closed. `skip_whitespace` cannot return
    /// an error, so the top-level entry point checks this flag.
    unterminated_comment: bool,
}

impl Parser {
    fn new(input: &str) -> Self {
        Parser {
            input: input.chars().collect(),
            pos: 0,
            depth: 0,
            unterminated_comment: false,
        }
    }

    fn current(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }

    /// Enter a nested expression/statement. Returns an error past `MAX_DEPTH`
    /// instead of recursing until the stack is exhausted.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!(
                "Expression too deeply nested (max depth {})",
                MAX_DEPTH
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn skip_whitespace(&mut self) {
        loop {
            while let Some(c) = self.current() {
                if c.is_whitespace() {
                    self.advance();
                } else {
                    break;
                }
            }
            // `//` line comments and `/* */` block comments. Without these the
            // parser rejects every documented example file, which is full of
            // them, with "Unexpected token: Some('/')" on its own comment.
            if self.current() == Some('/') && self.peek(1) == Some('/') {
                while let Some(c) = self.current() {
                    if c == '\n' {
                        break;
                    }
                    self.advance();
                }
                continue;
            }
            if self.current() == Some('/') && self.peek(1) == Some('*') {
                self.advance();
                self.advance();
                let mut closed = false;
                loop {
                    match self.current() {
                        None => break,
                        Some('*') if self.peek(1) == Some('/') => {
                            self.advance();
                            self.advance();
                            closed = true;
                            break;
                        }
                        _ => self.advance(),
                    }
                }
                if !closed {
                    // An unterminated comment swallows the rest of the file and
                    // would otherwise be accepted silently.
                    self.unterminated_comment = true;
                    return;
                }
                continue;
            }
            break;
        }
    }

    fn consume(&mut self, expected: char) -> Result<(), String> {
        self.skip_whitespace();
        if self.current() == Some(expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "Expected '{}', found {:?}",
                expected,
                self.current()
            ))
        }
    }

    fn consume_str(&mut self, s: &str) -> Result<(), String> {
        self.skip_whitespace();
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
            let mut digits = 0;
            while let Some(c) = self.current() {
                if c.is_ascii_hexdigit() {
                    num.push(c);
                    digits += 1;
                    self.advance();
                } else {
                    break;
                }
            }
            if digits == 0 {
                return Err("Invalid hex literal: no digits after 0x".to_string());
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
            // `1_000` used to parse as the number `1` followed by the
            // identifier `_000`, silently becoming two statements. A digit run
            // directly followed by an identifier character is one bad literal.
            if let Some(c) = self.current() {
                if c == '_' || c.is_ascii_alphanumeric() {
                    return Err(format!(
                        "Invalid number literal: '{}' followed by '{}'",
                        num, c
                    ));
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
        self.consume('"')?;
        let mut s = String::new();
        let mut closed = false;

        while let Some(c) = self.current() {
            if c == '"' {
                self.advance();
                closed = true;
                break;
            } else if c == '\\' {
                self.advance();
                if let Some(escaped) = self.current() {
                    match escaped {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        _ => s.push(escaped),
                    }
                    self.advance();
                }
            } else {
                s.push(c);
                self.advance();
            }
        }

        // The closing quote is consumed above, so the terminator must be
        // tracked with a flag: checking `current() == '"'` after the loop
        // compares the character AFTER the quote and rejected every string.
        if !closed {
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
        self.enter()?;
        let result = self.parse_binary(0);
        self.leave();
        result
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;

        loop {
            self.skip_whitespace();
            let op_prec = self.get_binop_precedence();
            if op_prec == 0 || op_prec < min_prec {
                break;
            }

            let op = self.parse_binop()?;
            // Left-associative: the right operand must bind strictly tighter
            // than this operator, so recurse with `op_prec + 1`. Recursing with
            // `min_prec + 1` let a lower-precedence operator be absorbed into the
            // right operand — `a + b == c` parsed as `a + (b == c)`, which the
            // runtime invariant check then rejected as a type error.
            let right = self.parse_binary(op_prec + 1)?;

            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn get_binop_precedence(&self) -> u8 {
        let mut pos = self.pos;
        while let Some(c) = self.input.get(pos).copied() {
            if c.is_whitespace() {
                pos += 1;
            } else {
                break;
            }
        }
        let c1 = self.input.get(pos).copied();
        let c2 = self.input.get(pos + 1).copied();

        match (c1, c2) {
            (Some('|'), Some('|')) => 1,
            (Some('.'), Some('.')) => 1, // Range: ..
            (Some('&'), Some('&')) => 2,
            // Bitwise or/xor/and. These single-character forms were missing
            // entirely: `a & b` parsed as `a` followed by a stray unary `& b`.
            (Some('|'), _) => 3,
            (Some('^'), _) => 3,
            (Some('&'), _) => 3,
            (Some('='), Some('='))
            | (Some('!'), Some('='))
            | (Some('<'), Some('='))
            | (Some('>'), Some('='))
            | (Some('<'), Some('<'))
            | (Some('>'), Some('>')) => 4,
            (Some('+'), _) | (Some('-'), _) => 5,
            (Some('*'), _) | (Some('/'), _) | (Some('%'), _) => 6,
            (Some('='), _) => 1, // Assignment has lowest precedence
            (Some('<'), _) => 4, // single <
            (Some('>'), _) => 4, // single >
            _ => 0,
        }
    }

    fn parse_binop(&mut self) -> Result<BinOp, String> {
        self.skip_whitespace();
        let c1 = self.current().ok_or("Expected operator")?;
        let c2 = self.peek(1);

        match (c1, c2) {
            ('|', Some('|')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Or)
            }
            ('.', Some('.')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Range)
            }
            ('&', Some('&')) => {
                self.advance();
                self.advance();
                Ok(BinOp::And)
            }
            ('=', Some('=')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Eq)
            }
            ('!', Some('=')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Ne)
            }
            ('<', Some('=')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Le)
            }
            ('>', Some('=')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Ge)
            }
            ('<', Some('<')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Shl)
            }
            ('>', Some('>')) => {
                self.advance();
                self.advance();
                Ok(BinOp::Shr)
            }
            ('+', _) => {
                self.advance();
                Ok(BinOp::Add)
            }
            ('-', _) => {
                self.advance();
                Ok(BinOp::Sub)
            }
            ('*', _) => {
                self.advance();
                Ok(BinOp::Mul)
            }
            ('/', _) => {
                self.advance();
                Ok(BinOp::Div)
            }
            ('%', _) => {
                self.advance();
                Ok(BinOp::Mod)
            }
            ('=', _) => {
                self.advance();
                Ok(BinOp::Assign)
            }
            ('^', _) => {
                self.advance();
                Ok(BinOp::BitXor)
            }
            ('&', _) => {
                self.advance();
                Ok(BinOp::BitAnd)
            }
            ('|', _) => {
                self.advance();
                Ok(BinOp::BitOr)
            }
            ('<', _) => {
                self.advance();
                Ok(BinOp::Lt)
            }
            ('>', _) => {
                self.advance();
                Ok(BinOp::Gt)
            }
            _ => Err(format!("Unknown operator: {:?}{:?}", c1, c2)),
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        // A long chain of `-`/`!` recurses here without going through
        // `parse_expr`, so it needs its own depth guard.
        self.enter()?;
        let result = match self.current() {
            Some('!') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(expr),
                })
            }
            Some('-') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(expr),
                })
            }
            Some('&') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Ref,
                    expr: Box::new(expr),
                })
            }
            Some('*') => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary {
                    op: UnOp::Deref,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_primary(),
        };
        self.leave();
        result
    }

    fn parse_if_expr(&mut self) -> Result<Expr, String> {
        // "if" keyword already consumed by the caller
        self.skip_whitespace();
        let condition = self.parse_expr()?;
        self.skip_whitespace();
        self.consume('{')?;
        let then_branch = Box::new(self.parse_expr()?);
        self.skip_whitespace();
        self.consume('}')?;
        self.skip_whitespace();
        let else_branch = if self.check_keyword("else") {
            self.skip_whitespace();
            self.consume('{')?;
            let eb = Box::new(self.parse_expr()?);
            self.skip_whitespace();
            self.consume('}')?;
            Some(eb)
        } else {
            None
        };
        Ok(Expr::If {
            condition: Box::new(condition),
            then_branch,
            else_branch,
        })
    }

    fn parse_match_expr(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        let expr = self.parse_expr()?;
        self.skip_whitespace();
        self.consume('{')?;
        let mut arms = Vec::new();
        loop {
            self.skip_whitespace();
            if self.current() == Some('}') {
                break;
            }
            let pat = if self.current() == Some('_') {
                self.advance();
                "_".to_string()
            } else {
                self.parse_number()?
            };
            self.skip_whitespace();
            self.consume_str("=>")?;
            self.skip_whitespace();
            let arm_expr = self.parse_expr()?;
            arms.push((pat, arm_expr));
            self.skip_whitespace();
            if self.current() == Some(',') {
                self.advance();
            } else {
                break;
            }
        }
        self.skip_whitespace();
        self.consume('}')?;
        Ok(Expr::Match {
            expr: Box::new(expr),
            arms,
        })
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();

        // Keyword-led expressions
        if self.check_keyword("if") {
            return self.parse_if_expr();
        }
        if self.check_keyword("match") {
            return self.parse_match_expr();
        }
        if self.check_keyword("true") {
            return Ok(Expr::Bool(true));
        }
        if self.check_keyword("false") {
            return Ok(Expr::Bool(false));
        }

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
                loop {
                    self.skip_whitespace();
                    if self.current() == Some('}') {
                        break;
                    }
                    stmts.push(self.parse_stmt()?);
                }
                self.consume('}')?;
                Ok(Expr::Block(stmts))
            }
            Some('"') => {
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

                let mut expr = if self.current() == Some('(') {
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
                    Expr::Call {
                        func: ident.clone(),
                        args,
                    }
                } else if self.current() == Some('.') {
                    self.advance();
                    let field = self.parse_identifier()?;
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
                        Expr::MethodCall {
                            receiver: Box::new(Expr::Ident(ident.clone())),
                            method: field,
                            args,
                        }
                    } else {
                        Expr::FieldAccess {
                            receiver: Box::new(Expr::Ident(ident.clone())),
                            field,
                        }
                    }
                } else {
                    Expr::Ident(ident.clone())
                };

                // Postfix array indexing: arr[0]
                loop {
                    self.skip_whitespace();
                    if self.current() == Some('[') {
                        self.advance();
                        let index = self.parse_expr()?;
                        self.consume(']')?;
                        expr = Expr::Index {
                            array: Box::new(expr),
                            index: Box::new(index),
                        };
                    } else {
                        break;
                    }
                }

                self.skip_whitespace();
                if self.current() == Some(':') {
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
                    // Check for == (equality) vs = (assignment)
                    if self.peek(1) == Some('=') {
                        // It's ==, don't consume - let binary operator parsing handle it
                        Ok(expr)
                    } else {
                        // Assignment expression: ident = expr
                        self.advance();
                        let value = self.parse_expr()?;
                        Ok(Expr::Binary {
                            left: Box::new(expr),
                            op: BinOp::Assign,
                            right: Box::new(value),
                        })
                    }
                } else {
                    Ok(expr)
                }
            }
            _ => Err(format!(
                "Unexpected character: {:?} at pos {}",
                self.current(),
                self.pos
            )),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        // `if` / `while` / `for` / block statements recurse through here
        // without going through `parse_expr`, so they need the same guard.
        self.enter()?;
        let result = self.parse_stmt_inner();
        self.leave();
        result
    }

    fn parse_stmt_inner(&mut self) -> Result<Stmt, String> {
        self.skip_whitespace();

        if self.check_keyword("let") {
            let mut mutable = false;
            self.skip_whitespace();
            if self.check_keyword("mut") {
                mutable = true;
                self.skip_whitespace();
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
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Let {
                mutable,
                name,
                ty,
                value,
            })
        } else if self.check_keyword("const") {
            let name = self.parse_identifier()?;
            self.skip_whitespace();
            self.consume(':')?;
            let ty = self.parse_type()?;
            self.consume('=')?;
            let value = self.parse_expr()?;
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Const { name, ty, value })
        } else if self.check_keyword("return") {
            let value = if self.current() == Some(';') {
                None
            } else {
                Some(self.parse_expr()?)
            };
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Return(value))
        } else if self.check_keyword("if") {
            self.skip_whitespace();
            if self.current() == Some('(') {
                // Statement form: if (cond) { stmts } else { stmts }
                self.consume('(')?;
                let condition = self.parse_expr()?;
                self.consume(')')?;
                self.consume('{')?;
                let mut then_branch = Vec::new();
                loop {
                    self.skip_whitespace();
                    if self.current() == Some('}') {
                        break;
                    }
                    then_branch.push(self.parse_stmt()?);
                }
                self.consume('}')?;
                let else_branch = if self.skip_whitespace_or_keyword("else") {
                    self.skip_whitespace();
                    if self.current() == Some('{') {
                        self.advance();
                        let mut else_branch = Vec::new();
                        loop {
                            self.skip_whitespace();
                            if self.current() == Some('}') {
                                break;
                            }
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
                Ok(Stmt::If {
                    condition,
                    then_branch,
                    else_branch,
                })
            } else {
                // Expression form: if cond { expr } else { expr } used as a statement
                let expr = self.parse_if_expr()?;
                if self.current() == Some(';') {
                    self.advance();
                }
                Ok(Stmt::Expr(expr))
            }
        } else if self.check_keyword("while") {
            let condition = self.parse_expr()?;
            self.consume('{')?;
            let mut body = Vec::new();
            loop {
                self.skip_whitespace();
                if self.current() == Some('}') {
                    break;
                }
                body.push(self.parse_stmt()?);
            }
            self.consume('}')?;
            Ok(Stmt::While { condition, body })
        } else if self.check_keyword("for") {
            let var = self.parse_identifier()?;
            self.skip_whitespace();
            self.consume_str("in")?;
            let iter = self.parse_expr()?;
            self.consume('{')?;
            let mut body = Vec::new();
            loop {
                self.skip_whitespace();
                if self.current() == Some('}') {
                    break;
                }
                body.push(self.parse_stmt()?);
            }
            self.consume('}')?;
            Ok(Stmt::For { var, iter, body })
        } else if self.check_keyword("break") {
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Break)
        } else if self.check_keyword("continue") {
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Continue)
        } else if self.check_keyword("match") {
            let expr = self.parse_match_expr()?;
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Expr(expr))
        } else {
            let expr = self.parse_expr()?;
            if self.current() == Some(';') {
                self.advance();
            }
            Ok(Stmt::Expr(expr))
        }
    }

    fn skip_whitespace_or_keyword(&mut self, kw: &str) -> bool {
        self.skip_whitespace();
        self.check_keyword(kw)
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
        loop {
            self.skip_whitespace();
            if self.current().is_none() {
                break;
            }
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
                if self.current() == Some(';') {
                    self.advance();
                }
                invariants.push(Invariant { expr });
            } else if self.check_keyword("state") {
                state = self.parse_state_decl()?;
            } else if self.current() == Some('@') || self.check_keyword("fn") {
                functions.push(self.parse_function()?);
            } else {
                self.skip_whitespace();
                if self.current().is_some() {
                    return Err(format!("Unexpected token: {:?}", self.current()));
                }
            }
        }

        self.consume('}')?;
        Ok(Contract {
            name,
            invariants,
            state,
            functions,
        })
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
        if self.current() == Some('@') {
            self.advance();
            self.skip_whitespace();
            if self.check_keyword("verified") {
                self.skip_whitespace();
                let mut pre = None;
                let mut post = None;
                let mut invariant = None;
                if self.current() == Some('(') {
                    self.advance();
                    loop {
                        self.skip_whitespace();
                        if self.current() == Some(')') {
                            self.advance();
                            break;
                        }
                        if self.current().is_none() {
                            return Err("Unterminated @verified annotation".to_string());
                        }
                        let key = self.parse_identifier()?;
                        self.skip_whitespace();
                        self.consume(':')?;
                        let expr = self.parse_expr()?;
                        match key.as_str() {
                            "pre" => pre = Some(expr),
                            "post" => post = Some(expr),
                            "invariant" => invariant = Some(expr),
                            other => {
                                return Err(format!(
                                    "Unknown @verified key '{}' (expected pre, post or invariant)",
                                    other
                                ))
                            }
                        }
                        self.skip_whitespace();
                        if self.current() == Some(',') {
                            self.advance();
                        }
                    }
                }
                verified = Some(VerifiedAnnotation {
                    pre,
                    post,
                    invariant,
                });
            } else {
                // Some other attribute: skip its name and any balanced argument
                // list so the following `fn` still parses.
                self.parse_identifier()?;
                self.skip_whitespace();
                if self.current() == Some('(') {
                    let mut depth = 0u32;
                    loop {
                        match self.current() {
                            Some('(') => {
                                depth += 1;
                                self.advance();
                            }
                            Some(')') => {
                                depth -= 1;
                                self.advance();
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(_) => self.advance(),
                            None => break,
                        }
                    }
                }
            }
        }
        self.skip_whitespace();
        // The `fn` keyword is present when a verification annotation precedes it
        if self.check_keyword("fn") {
            self.skip_whitespace();
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
        let contracts = parser.parse_program()?;
        if parser.unterminated_comment {
            return Err("Unterminated block comment".to_string());
        }
        for contract in &contracts {
            ast_depth_ok(contract)?;
        }
        Ok(contracts)
    }
}

/// Symbol for a binary operator, as the parser accepts it.
pub fn binop_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Range => "..",
        BinOp::Assign => "=",
    }
}

pub fn unop_symbol(op: UnOp) -> &'static str {
    match op {
        UnOp::Not => "!",
        UnOp::Neg => "-",
        UnOp::Deref => "*",
        UnOp::Ref => "&",
        UnOp::RefMut => "&mut ",
    }
}

pub fn type_to_source(ty: &Type) -> String {
    if ty.generics.is_empty() {
        ty.name.clone()
    } else {
        let params: Vec<String> = ty.generics.iter().map(type_to_source).collect();
        format!("{}<{}>", ty.name, params.join(", "))
    }
}

/// Render an expression back to REDΣSIGMA source.
///
/// Binary and unary forms are always fully parenthesized, so the output is
/// unambiguously re-parseable without having to reproduce the parser's
/// precedence table. The previous implementation emitted Rust `{:?}` debug
/// text, which the parser could not read back at all.
pub fn expr_to_source(expr: &Expr) -> String {
    match expr {
        Expr::Number(n) => n.clone(),
        Expr::String(s) => format!("{:?}", s),
        Expr::Ident(name) => name.clone(),
        Expr::Bool(b) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        Expr::Unit => "()".into(),
        Expr::Binary { left, op, right } => {
            format!(
                "({} {} {})",
                expr_to_source(left),
                binop_symbol(*op),
                expr_to_source(right)
            )
        }
        Expr::Unary { op, expr } => {
            format!("({}{})", unop_symbol(*op), expr_to_source(expr))
        }
        Expr::Call { func, args } => {
            let a: Vec<String> = args.iter().map(expr_to_source).collect();
            format!("{}({})", func, a.join(", "))
        }
        Expr::MethodCall {
            receiver,
            method,
            args,
        } => {
            let a: Vec<String> = args.iter().map(expr_to_source).collect();
            format!("{}.{}({})", expr_to_source(receiver), method, a.join(", "))
        }
        Expr::FieldAccess { receiver, field } => {
            format!("{}.{}", expr_to_source(receiver), field)
        }
        Expr::Index { array, index } => {
            format!("{}[{}]", expr_to_source(array), expr_to_source(index))
        }
        Expr::Array(items) => {
            let a: Vec<String> = items.iter().map(expr_to_source).collect();
            format!("[{}]", a.join(", "))
        }
        Expr::Struct { name, fields } => {
            let f: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, expr_to_source(v)))
                .collect();
            format!("{} {{ {} }}", name, f.join(", "))
        }
        Expr::Block(stmts) => {
            let s: Vec<String> = stmts.iter().map(stmt_to_source).collect();
            format!("{{ {} }}", s.join(" "))
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let mut out = format!(
                "if {} {{ {} }}",
                expr_to_source(condition),
                expr_to_source(then_branch)
            );
            if let Some(eb) = else_branch {
                out.push_str(&format!(" else {{ {} }}", expr_to_source(eb)));
            }
            out
        }
        Expr::Match { expr, arms } => {
            let a: Vec<String> = arms
                .iter()
                .map(|(p, e)| format!("{} => {}", p, expr_to_source(e)))
                .collect();
            format!("match {} {{ {} }}", expr_to_source(expr), a.join(", "))
        }
        Expr::Range { from, to } => {
            format!("{}..{}", expr_to_source(from), expr_to_source(to))
        }
    }
}

pub fn stmt_to_source(stmt: &Stmt) -> String {
    match stmt {
        Stmt::Let {
            mutable,
            name,
            ty,
            value,
        } => {
            let kw = if *mutable { "let mut" } else { "let" };
            match ty {
                Some(t) => format!(
                    "{} {}: {} = {};",
                    kw,
                    name,
                    type_to_source(t),
                    expr_to_source(value)
                ),
                None => format!("{} {} = {};", kw, name, expr_to_source(value)),
            }
        }
        Stmt::Const { name, ty, value } => {
            format!(
                "const {}: {} = {};",
                name,
                type_to_source(ty),
                expr_to_source(value)
            )
        }
        Stmt::Expr(e) => format!("{};", expr_to_source(e)),
        Stmt::Return(Some(e)) => format!("return {};", expr_to_source(e)),
        Stmt::Return(None) => "return;".into(),
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let body: Vec<String> = then_branch.iter().map(stmt_to_source).collect();
            let mut out = format!(
                "if ({}) {{ {} }}",
                expr_to_source(condition),
                body.join(" ")
            );
            if let Some(eb) = else_branch {
                let e: Vec<String> = eb.iter().map(stmt_to_source).collect();
                out.push_str(&format!(" else {{ {} }}", e.join(" ")));
            }
            out
        }
        Stmt::While { condition, body } => {
            let b: Vec<String> = body.iter().map(stmt_to_source).collect();
            format!("while {} {{ {} }}", expr_to_source(condition), b.join(" "))
        }
        Stmt::For { var, iter, body } => {
            let b: Vec<String> = body.iter().map(stmt_to_source).collect();
            format!(
                "for {} in {} {{ {} }}",
                var,
                expr_to_source(iter),
                b.join(" ")
            )
        }
        Stmt::Match { expr, arms } => {
            let a: Vec<String> = arms
                .iter()
                .map(|(p, e)| format!("{} => {}", p, expr_to_source(e)))
                .collect();
            format!("match {} {{ {} }}", expr_to_source(expr), a.join(", "))
        }
        Stmt::Break => "break;".into(),
        Stmt::Continue => "continue;".into(),
    }
}

/// Collect every identifier named anywhere in an expression.
pub fn collect_idents(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Ident(n) => out.push(n.clone()),
        Expr::Binary { left, right, .. } => {
            collect_idents(left, out);
            collect_idents(right, out);
        }
        Expr::Unary { expr, .. } => collect_idents(expr, out),
        Expr::Call { args, .. } => {
            for a in args {
                collect_idents(a, out);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_idents(receiver, out);
            for a in args {
                collect_idents(a, out);
            }
        }
        Expr::FieldAccess { receiver, .. } => collect_idents(receiver, out),
        Expr::Index { array, index } => {
            collect_idents(array, out);
            collect_idents(index, out);
        }
        Expr::Array(items) => {
            for i in items {
                collect_idents(i, out);
            }
        }
        Expr::Struct { fields, .. } => {
            for (_, v) in fields {
                collect_idents(v, out);
            }
        }
        Expr::Block(stmts) => {
            for s in stmts {
                collect_stmt_idents(s, out);
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_idents(condition, out);
            collect_idents(then_branch, out);
            if let Some(e) = else_branch {
                collect_idents(e, out);
            }
        }
        Expr::Match { expr, arms } => {
            collect_idents(expr, out);
            for (_, e) in arms {
                collect_idents(e, out);
            }
        }
        Expr::Range { from, to } => {
            collect_idents(from, out);
            collect_idents(to, out);
        }
        _ => {}
    }
}

pub fn collect_stmt_idents(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } | Stmt::Expr(value) => {
            collect_idents(value, out)
        }
        Stmt::Return(Some(e)) => collect_idents(e, out),
        Stmt::Return(None) => {}
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_idents(condition, out);
            for s in then_branch {
                collect_stmt_idents(s, out);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_stmt_idents(s, out);
                }
            }
        }
        Stmt::While { condition, body } => {
            collect_idents(condition, out);
            for s in body {
                collect_stmt_idents(s, out);
            }
        }
        Stmt::For { iter, body, .. } => {
            collect_idents(iter, out);
            for s in body {
                collect_stmt_idents(s, out);
            }
        }
        Stmt::Match { expr, arms } => {
            collect_idents(expr, out);
            for (_, e) in arms {
                collect_idents(e, out);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

/// Render a contract back to REDΣSIGMA source that the parser can read again.
pub fn contract_to_source(contract: &Contract) -> String {
    let mut out = format!("contract {} {{\n", contract.name);
    for inv in &contract.invariants {
        out.push_str(&format!("    invariant {};\n", expr_to_source(&inv.expr)));
    }
    out.push_str("    state {\n");
    for (name, ty) in &contract.state.fields {
        out.push_str(&format!("        {}: {},\n", name, type_to_source(ty)));
    }
    out.push_str("    }\n");
    for func in &contract.functions {
        let params: Vec<String> = func
            .params
            .iter()
            .map(|(n, t)| format!("{}: {}", n, type_to_source(t)))
            .collect();
        out.push_str(&format!(
            "\n    fn {}({}) -> {} {{\n",
            func.name,
            params.join(", "),
            type_to_source(&func.return_type)
        ));
        for stmt in &func.body {
            out.push_str(&format!("        {}\n", stmt_to_source(stmt)));
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n");
    out
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

    #[test]
    fn test_comments_are_skipped() {
        // Regression: the parser rejected its own comment tokens, so every
        // documented example file failed with "Unexpected token: Some('/')".
        let input = r#"
            // line comment
            contract Test {
                /* block
                   comment */
                state { x: u64 } // trailing comment
                fn foo() -> u64 { x; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].name, "Test");
        assert_eq!(contracts[0].state.fields[0].0, "x");
    }

    #[test]
    fn test_division_is_not_a_comment() {
        // `x / 2` must stay a division; only `//` and `/*` open a comment.
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { x / 2; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::Binary { op: BinOp::Div, .. }) => {}
            other => panic!("expected Div, got {:?}", other),
        }
    }

    #[test]
    fn test_string_literal_roundtrips() {
        // Regression: the closing quote is consumed inside the loop, so the
        // old `current() == '"'` check compared the character AFTER the quote
        // and rejected every well-formed string literal.
        let input = r#"contract Test { state { x: u64 } fn foo() -> u64 { "http://x"; } }"#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::String(s)) => assert_eq!(s, "http://x"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn test_string_escape_sequences() {
        let input = r#"contract Test { state { x: u64 } fn foo() -> u64 { "a\nb\t\"c"; } }"#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::String(s)) => assert_eq!(s, "a\nb\t\"c"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn test_unterminated_string_is_error() {
        let input = r#"contract Test { state { x: u64 } fn foo() -> u64 { "abc; } }"#;
        let err = Parser::parse(input).unwrap_err();
        assert_eq!(err, "Unterminated string");
    }

    #[test]
    fn test_unterminated_block_comment_is_error() {
        // Regression: an unterminated `/*` swallowed the rest of the file and
        // was accepted silently.
        let input = "contract Test { state { x: u64 } } /*";
        let err = Parser::parse(input).unwrap_err();
        assert_eq!(err, "Unterminated block comment");
    }

    #[test]
    fn test_underscore_literal_is_error() {
        // Regression: `1_000` parsed as the number `1` followed by the
        // identifier `_000`, silently becoming two statements.
        let input = "contract Test { state { x: u64 } fn foo() -> u64 { 1_000; } }";
        let err = Parser::parse(input).unwrap_err();
        assert!(err.contains("Invalid number literal"), "got: {}", err);
    }

    #[test]
    fn test_invalid_hex_literal_is_error() {
        let input = "contract Test { state { x: u64 } fn foo() -> u64 { 0x; } }";
        let err = Parser::parse(input).unwrap_err();
        assert!(err.contains("Invalid hex literal"), "got: {}", err);
    }

    #[test]
    fn test_hex_literal_parses() {
        let input = "contract Test { state { x: u64 } fn foo() -> u64 { 0xFF; } }";
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::Number(n)) => assert_eq!(n, "0xFF"),
            other => panic!("expected Number, got {:?}", other),
        }
    }

    #[test]
    fn test_deep_nesting_is_clean_error() {
        // Regression: 2000 nested parens overflowed the stack (SIGABRT).
        let input = format!(
            "contract Test {{ state {{ x: u64 }} fn foo() -> u64 {{ {}1{}; }} }}",
            "(".repeat(2000),
            ")".repeat(2000)
        );
        let err = Parser::parse(&input).unwrap_err();
        assert!(err.contains("too deeply nested"), "got: {}", err);
    }

    #[test]
    fn test_deep_unary_is_clean_error() {
        let input = format!(
            "contract Test {{ state {{ x: u64 }} fn foo() -> u64 {{ {}1; }} }}",
            "-".repeat(5000)
        );
        let err = Parser::parse(&input).unwrap_err();
        assert!(err.contains("too deeply nested"), "got: {}", err);
    }

    #[test]
    fn test_nesting_at_limit_is_accepted() {
        // A depth just under the guard must still parse (40 nested parens
        // costs ~82 depth units; the guard is 100).
        let input = format!(
            "contract Test {{ state {{ x: u64 }} fn foo() -> u64 {{ {}1{}; }} }}",
            "(".repeat(40),
            ")".repeat(40)
        );
        assert!(Parser::parse(&input).is_ok());
    }

    #[test]
    fn test_assignment_is_not_addition() {
        // Regression: `x = v` used to parse as `x + v` (BinOp::Add placeholder),
        // which made every state mutation disappear from the AST.
        let input = r#"
            contract Test {
                state { x: u64 }
                fn set(v: u64) -> u64 { x = v; v; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::Binary {
                op: BinOp::Assign, ..
            }) => {}
            other => panic!("expected BinOp::Assign, got {:?}", other),
        }
    }

    #[test]
    fn test_equality_is_not_assignment() {
        // `==` must stay Eq; only a single `=` is Assign.
        let input = r#"
            contract Test {
                state { x: u64 }
                fn same(a: u64, b: u64) -> u64 { a == b; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].functions[0].body[0] {
            Stmt::Expr(Expr::Binary { op: BinOp::Eq, .. }) => {}
            other => panic!("expected BinOp::Eq, got {:?}", other),
        }
    }

    #[test]
    fn test_verified_annotation_is_captured_as_expressions() {
        // Regression: `@verified(pre:..., post:..., invariant:...)` was discarded
        // at the only construction site, so no verifier could ever see it.
        let input = r#"
            contract Test {
                invariant total >= 0;
                state { total: u64 }
                @verified(
                    pre: amount > 0,
                    post: total == old(total) + amount,
                    invariant: total >= 0,
                )
                fn deposit(amount: u64) -> u64 { total = total + amount; amount; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let ann = contracts[0].functions[0]
            .verified
            .as_ref()
            .expect("@verified must be captured");

        let pre = ann.pre.as_ref().expect("pre must be parsed");
        assert_eq!(expr_to_source(pre), "(amount > 0)");

        let post = ann.post.as_ref().expect("post must be parsed");
        let post_src = expr_to_source(post);
        assert!(post_src.contains("=="), "post should compare: {}", post_src);
        assert!(
            post_src.contains("old"),
            "post should mention old(): {}",
            post_src
        );
        // `old(total)` parses as an ordinary call node.
        assert!(matches!(post, Expr::Binary { .. }));

        let inv = ann.invariant.as_ref().expect("invariant must be parsed");
        assert_eq!(expr_to_source(inv), "(total >= 0)");
    }

    #[test]
    fn test_verified_annotation_without_parens_has_no_clauses() {
        let input = r#"
            contract Test {
                invariant x >= 0;
                state { x: u64 }
                @verified
                fn get() -> u64 { x; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let ann = contracts[0].functions[0].verified.as_ref().unwrap();
        assert!(ann.pre.is_none());
        assert!(ann.post.is_none());
        assert!(ann.invariant.is_none());
    }

    #[test]
    fn test_unknown_verified_key_is_rejected() {
        let input = r#"
            contract Test {
                invariant x >= 0;
                state { x: u64 }
                @verified(unknown: x > 0)
                fn get() -> u64 { x; }
            }
        "#;
        let err = Parser::parse(input).expect_err("unknown @verified key must be an error");
        assert!(err.contains("Unknown @verified key"), "{}", err);
    }

    #[test]
    fn test_precedence_arithmetic_binds_tighter_than_comparison() {
        // Regression: `a + b == c` parsed as `a + (b == c)` because parse_binary
        // recursed with `min_prec + 1` instead of the operator's own precedence.
        let input = r#"
            contract Test {
                invariant a + b == c;
                state { a: u64, b: u64, c: u64 }
                fn f() -> u64 { 1; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        match &contracts[0].invariants[0].expr {
            Expr::Binary {
                op: BinOp::Eq,
                left,
                ..
            } => match left.as_ref() {
                Expr::Binary { op: BinOp::Add, .. } => {}
                other => panic!("expected (a + b) on the left of ==, got {:?}", other),
            },
            other => panic!("expected a top-level ==, got {:?}", other),
        }
    }
}
