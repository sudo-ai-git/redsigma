//! WAT (WebAssembly text) backend for REDΣSIGMA.
//!
//! The previous "codegen" emitted a 48-byte placeholder: magic, four EMPTY
//! sections, and a custom section holding the literal string
//! `"REDΣSIGMA Proof for <name>"`. `function_index` mapped names to function ids
//! that did not exist. This module replaces that with a real lowering of the
//! AST to WAT.
//!
//! ## Executable subset
//!
//! - Contract state fields become mutable `i64` globals.
//! - All integer locals and parameters lower to `i64`. `u8..u64` and `i8..i64`
//!   share one representation; `u128`/`i128` are **rejected** rather than
//!   silently truncated.
//! - Statements: `let`/`const`, expression statements, `return`, `if/else`,
//!   `while`, `for .. in a..b`, `match`, `break`, `continue`.
//! - Expressions: numbers, identifiers, booleans, unary and binary operators,
//!   calls, blocks.
//! - Anything else (strings, arrays, structs, indexing, method calls, field
//!   access) is a **compile error**, not a silent miscompile.
//!
//! The output is WAT text. Assembling and validating it is a separate step
//! (Python `wasmtime`), because `wat2wasm`/`Module` is the only trustworthy
//! validity gate — a text-level check cannot see structural defects.

use redsigma_parser::{BinOp, Contract, Expr, Stmt, UnOp};

use crate::CodegenError;

/// Lower a contract to WAT text.
pub fn emit_wat(contract: &Contract) -> Result<String, CodegenError> {
    let mut e = Emitter::new(contract);
    e.emit()
}

struct Emitter<'a> {
    contract: &'a Contract,
    /// locals declared in the function currently being emitted
    locals: Vec<String>,
    /// type of each local (all i64 in this backend)
    out: String,
    /// indentation width in spaces
    indent: usize,
}

impl<'a> Emitter<'a> {
    fn new(contract: &'a Contract) -> Self {
        Self {
            contract,
            locals: Vec::new(),
            out: String::new(),
            indent: 1,
        }
    }

    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
    }

    fn line(&mut self, s: &str) {
        self.pad();
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn emit(mut self) -> Result<String, CodegenError> {
        self.line("(module");

        // ---- state globals ----
        for (name, ty) in &self.contract.state.fields {
            if !is_supported_int(&ty.name) {
                return Err(CodegenError::UnsupportedExpression(format!(
                    "state field '{}' has type '{}', which the WAT backend does not lower",
                    name, ty.name
                )));
            }
            self.line(&format!("(global ${} (mut i64) (i64.const 0))", name));
        }

        // ---- functions ----
        for func in &self.contract.functions {
            if !is_supported_int(&func.return_type.name) && !func.return_type.name.is_empty() {
                return Err(CodegenError::UnsupportedExpression(format!(
                    "function '{}' returns '{}', which the WAT backend does not lower",
                    func.name, func.return_type.name
                )));
            }
            for (p, ty) in &func.params {
                if !is_supported_int(&ty.name) {
                    return Err(CodegenError::UnsupportedExpression(format!(
                        "parameter '{}' of '{}' has type '{}', which the WAT backend does not lower",
                        p, func.name, ty.name
                    )));
                }
            }

            self.locals.clear();
            let params: Vec<String> = func
                .params
                .iter()
                .map(|(p, _)| format!("(param ${} i64)", p))
                .collect();
            let result = if func.return_type.name.is_empty() {
                String::new()
            } else {
                " (result i64)".to_string()
            };
            self.line(&format!(
                "(func ${} {}{}",
                func.name,
                params.join(" "),
                result
            ));
            self.indent += 1;

            // Parameters are already declared by `(param ...)` in the function
            // header; registering them here only teaches name resolution that
            // they are locals (and shadow state fields). Emitting a second
            // `(local $p i64)` is a duplicate-identifier error.
            for (p, _) in &func.params {
                if !self.locals.contains(p) {
                    self.locals.push(p.clone());
                }
            }
            for name in collect_locals(&func.body) {
                if self.contract.state.fields.iter().any(|(f, _)| f == &name) {
                    return Err(CodegenError::UnsupportedExpression(format!(
                        "local '{}' shadows a state field; shadowing is not supported by the WAT backend",
                        name
                    )));
                }
                self.locals.push(name.clone());
                self.line(&format!("(local ${} i64)", name));
            }
            self.locals.push("__match".to_string());
            self.line("(local $__match i64)");

            // A function's value is its last expression statement, so emit that
            // one without a `(drop)` and without a trailing `unreachable` —
            // otherwise a correct body falls through into a trap.
            for (i, stmt) in func.body.iter().enumerate() {
                let last = i + 1 == func.body.len();
                if last {
                    if let Stmt::Expr(e) = stmt {
                        self.expr(e)?;
                        continue;
                    }
                }
                self.stmt(stmt)?;
            }
            // A results function whose body does not end in an expression would
            // be invalid WAT; emit `unreachable` so the intent is explicit.
            if !func.return_type.name.is_empty()
                && !matches!(func.body.last(), Some(Stmt::Expr(_)))
            {
                self.line("unreachable");
            }
            self.indent -= 1;
            self.line(")");
            self.line(&format!("(export \"{}\" (func ${}))", func.name, func.name));
        }

        self.line(")");
        Ok(self.out)
    }

    // ---------- statements ----------

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Let { name, ty, value, .. } => {
                if let Some(t) = ty {
                    if !is_supported_int(&t.name) {
                        return Err(CodegenError::UnsupportedExpression(format!(
                            "local '{}' has type '{}', which the WAT backend does not lower",
                            name, t.name
                        )));
                    }
                }
                self.expr(value)?;
                self.line(&format!("(local.set ${})", name));
            }
            Stmt::Const { name, ty, value } => {
                if !is_supported_int(&ty.name) {
                    return Err(CodegenError::UnsupportedExpression(format!(
                        "const '{}' has type '{}', which the WAT backend does not lower",
                        name, ty.name
                    )));
                }
                self.expr(value)?;
                self.line(&format!("(local.set ${})", name));
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
                self.line("(drop)");
            }
            Stmt::Return(Some(e)) => {
                self.expr(e)?;
                self.line("(return)");
            }
            Stmt::Return(None) => {
                self.line("(return)");
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.emit_condition(condition)?;
                self.line("(if");
                self.indent += 1;
                self.line("(then");
                self.indent += 1;
                for s in then_branch {
                    self.stmt(s)?;
                }
                self.indent -= 1;
                self.line(")");
                if let Some(eb) = else_branch {
                    self.line("(else");
                    self.indent += 1;
                    for s in eb {
                        self.stmt(s)?;
                    }
                    self.indent -= 1;
                    self.line(")");
                }
                self.indent -= 1;
                self.line(")");
            }
            Stmt::While { condition, body } => {
                // (block (loop ...)) so the exit branch targets the block. A
                // bare loop's `br 1` would target the function body, which
                // expects the return value and finds an empty stack.
                self.line("(block");
                self.indent += 1;
                self.line("(loop");
                self.indent += 1;
                // exit when the condition is false: br_if branches on non-zero,
                // so the condition must be an i32
                self.emit_condition(condition)?;
                // `br_if` branches when non-zero, so invert the condition:
                // branch out of the loop when it is FALSE.
                self.line("(i32.eqz)");
                self.line("(br_if 1)");
                for s in body {
                    self.stmt(s)?;
                }
                self.line("(br 0)");
                self.indent -= 1;
                self.line(")");
                self.indent -= 1;
                self.line(")");
            }
            Stmt::For { var, iter, body } => {
                // Only `for i in a..b` is lowered; anything else is an error.
                // The parser builds a range as `Binary { op: Range }`, not as
                // `Expr::Range`; accept both.
                let (from, to) = match iter {
                    Expr::Range { from, to } => (from.as_ref().clone(), to.as_ref().clone()),
                    Expr::Binary {
                        left,
                        op: BinOp::Range,
                        right,
                    } => (left.as_ref().clone(), right.as_ref().clone()),
                    other => {
                        return Err(CodegenError::UnsupportedExpression(format!(
                            "for-loop iterator must be a range, got {}",
                            redsigma_parser::expr_to_source(other)
                        )))
                    }
                };
                let idx = format!("__for_{}", var);
                self.line("(block");
                self.indent += 1;
                self.expr(&from)?;
                self.line(&format!("(local.set ${})", idx));
                self.line("(loop");
                self.indent += 1;
                // exit when `idx < to` is false: the comparison is already an
                // i32, so it only needs inverting.
                self.line(&format!(
                    "(br_if 1 (i32.eqz (i64.lt_s (local.get ${})",
                    idx
                ));
                self.expr(&to)?;
                self.line(")))");
                self.line(&format!("(local.set ${} (local.get ${}))", var, idx));
                for s in body {
                    self.stmt(s)?;
                }
                self.line(&format!(
                    "(local.set ${} (i64.add (local.get ${}) (i64.const 1)))",
                    idx, idx
                ));
                self.line("(br 0)");
                self.indent -= 1;
                self.line(")");
                self.indent -= 1;
                self.line(")");
            }
            Stmt::Match { expr, arms } => {
                self.expr(expr)?;
                self.line("(local.set $__match)");
                self.emit_match_arm_chain(arms, false)?;
            }
            Stmt::Break => self.line("(br 0)"),
            Stmt::Continue => self.line("(br 0)"),
        }
        Ok(())
    }

    /// Emit an if/else chain over match arms.
    ///
    /// `value=true` makes the chain produce an i64 (`(if (result i64) ...)`),
    /// which is what expression position needs. `value=false` emits bare `if`
    /// statements and drops the value, for statement position.
    ///
    /// A value-producing match with no wildcard arm is a **compile error**:
    /// silently yielding 0 for an unhandled case is the silent-wrong-answer
    /// class, so it is refused instead.
    fn emit_match_arm_chain(
        &mut self,
        arms: &[(String, Expr)],
        value: bool,
    ) -> Result<(), CodegenError> {
        if value && !arms.iter().any(|(p, _)| p == "_") {
            return Err(CodegenError::UnsupportedExpression(
                "a match in value position must have a wildcard arm; refusing to \
                 guess a default"
                    .to_string(),
            ));
        }
        if arms.is_empty() {
            if value {
                self.line("(i64.const 0)");
            }
            return Ok(());
        }
        let (pat, body) = &arms[0];
        let rest = &arms[1..];
        if pat == "_" {
            self.expr(body)?;
            if !value {
                self.line("(drop)");
            }
            return Ok(());
        }
        if value {
            self.line("(if (result i64)");
        } else {
            self.line("(if");
        }
        self.indent += 1;
        self.line(&format!("(i64.eq (local.get $__match) (i64.const {}))", pat));
        self.line("(then");
        self.indent += 1;
        self.expr(body)?;
        if !value {
            self.line("(drop)");
        }
        self.indent -= 1;
        self.line(")");
        if !rest.is_empty() {
            self.line("(else");
            self.indent += 1;
            self.emit_match_arm_chain(rest, value)?;
            self.indent -= 1;
            self.line(")");
        }
        self.indent -= 1;
        self.line(")");
        Ok(())
    }

    // ---------- expressions ----------

    /// Emit a condition as an i32 suitable for `if` / `br_if`.
    ///
    /// Comparison operators already produce an i32 in WAT, so they are passed
    /// through. Anything else is an i64 and must be converted to "non-zero":
    /// `i64.eqz` yields i32 (1 when zero), and the outer `i32.eqz` inverts it.
    /// Using `i64.eqz` alone after a comparison is a type error — the operand is
    /// already i32 — which the harness caught.
    fn emit_condition(&mut self, expr: &Expr) -> Result<(), CodegenError> {
        self.expr(expr)?;
        if !is_comparison(expr) {
            self.line("(i64.eqz)");
            self.line("(i32.eqz)");
        }
        Ok(())
    }

    fn expr(&mut self, expr: &Expr) -> Result<(), CodegenError> {
        match expr {
            Expr::Number(n) => self.line(&format!("(i64.const {})", n)),
            Expr::Bool(b) => self.line(&format!("(i64.const {})", if *b { 1 } else { 0 })),
            Expr::Ident(name) => {
                // Locals (parameters and let-bindings) shadow state fields.
                if self.locals.contains(name) {
                    self.line(&format!("(local.get ${})", name));
                } else if self.contract.state.fields.iter().any(|(f, _)| f == name) {
                    self.line(&format!("(global.get ${})", name));
                } else {
                    return Err(CodegenError::UnsupportedExpression(format!(
                        "unknown name '{}'",
                        name
                    )));
                }
            }
            Expr::Binary { left, op, right } => {
                if *op == BinOp::Assign {
                    let name = match left.as_ref() {
                        Expr::Ident(n) => n.clone(),
                        other => {
                            return Err(CodegenError::UnsupportedExpression(format!(
                                "assignment target must be a name, got {}",
                                redsigma_parser::expr_to_source(other)
                            )))
                        }
                    };
                    self.expr(right)?;
                    // Assignment is a value-producing expression (it matches the
                    // interpreter, which returns the assigned value), so leave
                    // the value on the stack. `set` alone would leave nothing and
                    // make the enclosing `(drop)` underflow — a defect the WAT
                    // harness caught.
                    // Locals (parameters and let-bindings) shadow state fields.
                    if self.locals.contains(&name) {
                        self.line(&format!("(local.set ${})", name));
                        self.line(&format!("(local.get ${})", name));
                    } else if self.contract.state.fields.iter().any(|(f, _)| f == &name) {
                        self.line(&format!("(global.set ${})", name));
                        self.line(&format!("(global.get ${})", name));
                    } else {
                        return Err(CodegenError::UnsupportedExpression(format!(
                            "assignment to unknown name '{}'",
                            name
                        )));
                    }
                    return Ok(());
                }
                self.expr(left)?;
                self.expr(right)?;
                let inst = match op {
                    BinOp::Add => "i64.add",
                    BinOp::Sub => "i64.sub",
                    BinOp::Mul => "i64.mul",
                    BinOp::Div => "i64.div_s",
                    BinOp::Mod => "i64.rem_s",
                    BinOp::Eq => "i64.eq",
                    BinOp::Ne => "i64.ne",
                    BinOp::Lt => "i64.lt_s",
                    BinOp::Gt => "i64.gt_s",
                    BinOp::Le => "i64.le_s",
                    BinOp::Ge => "i64.ge_s",
                    BinOp::BitAnd => "i64.and",
                    BinOp::BitOr => "i64.or",
                    BinOp::BitXor => "i64.xor",
                    BinOp::Shl => "i64.shl",
                    BinOp::Shr => "i64.shr_s",
                    BinOp::And => "i64.and",
                    BinOp::Or => "i64.or",
                    BinOp::Range => {
                        return Err(CodegenError::UnsupportedExpression(
                            "range is only valid as a for-loop iterator".to_string(),
                        ))
                    }
                    BinOp::Assign => unreachable!(),
                };
                self.line(&format!("({})", inst));
            }
            Expr::Unary { op, expr } => match op {
                UnOp::Neg => {
                    self.line("(i64.const 0)");
                    self.expr(expr)?;
                    self.line("(i64.sub)");
                }
                UnOp::Not => {
                    self.expr(expr)?;
                    self.line("(i64.eqz)");
                }
                other => {
                    return Err(CodegenError::UnsupportedExpression(format!(
                        "unary operator {:?} is not lowered",
                        other
                    )))
                }
            },
            Expr::Call { func, args } => {
                for a in args {
                    self.expr(a)?;
                }
                self.line(&format!("(call ${})", func));
            }
            Expr::Block(stmts) => {
                // A block's value is its last expression statement's value; the
                // caller drops it when the block is used as a statement.
                for (i, s) in stmts.iter().enumerate() {
                    let last = i + 1 == stmts.len();
                    if last {
                        if let Stmt::Expr(e) = s {
                            self.expr(e)?;
                            continue;
                        }
                    }
                    self.stmt(s)?;
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if else_branch.is_none() {
                    return Err(CodegenError::UnsupportedExpression(
                        "an if in value position needs an else branch; write \
                         `if (cond) { ... }` for the statement form"
                            .to_string(),
                    ));
                }
                self.emit_condition(condition)?;
                self.line("(if (result i64)");
                self.indent += 1;
                self.line("(then");
                self.indent += 1;
                self.expr(then_branch)?;
                self.indent -= 1;
                self.line(")");
                if let Some(eb) = else_branch {
                    self.line("(else");
                    self.indent += 1;
                    self.expr(eb)?;
                    self.indent -= 1;
                    self.line(")");
                }
                self.indent -= 1;
                self.line(")");
            }
            Expr::Match { expr, arms } => {
                self.expr(expr)?;
                self.line("(local.set $__match)");
                self.emit_match_arm_chain(arms, true)?;
            }
            other => {
                return Err(CodegenError::UnsupportedExpression(format!(
                    "expression is not in the WAT backend's executable subset: {}",
                    redsigma_parser::expr_to_source(other)
                )))
            }
        }
        Ok(())
    }
}

/// Whether an expression already evaluates to an i32 condition in WAT.
fn is_comparison(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Binary {
            op: BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge,
            ..
        }
    )
}

/// Whether a declared integer type can be lowered by this backend.
///
/// Everything up to 64 bits shares one `i64` representation. 128-bit integers
/// are rejected rather than truncated.
pub fn is_supported_int(name: &str) -> bool {
    let bare = name.strip_prefix("Verified").unwrap_or(name);
    matches!(
        bare,
        "u8" | "u16" | "u32" | "u64" | "i8" | "i16" | "i32" | "i64" | "bool"
    )
}

/// Names of locals a function body declares, in first-seen order.
pub fn collect_locals(body: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk_stmt(stmt: &Stmt, out: &mut Vec<String>) {
        match stmt {
            Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
                walk_expr(value, out);
            }
            Stmt::Expr(e) => walk_expr(e, out),
            Stmt::Return(Some(e)) => walk_expr(e, out),
            Stmt::Return(None) => {}
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                walk_expr(condition, out);
                for s in then_branch {
                    walk_stmt(s, out);
                }
                if let Some(eb) = else_branch {
                    for s in eb {
                        walk_stmt(s, out);
                    }
                }
            }
            Stmt::While { condition, body } => {
                walk_expr(condition, out);
                for s in body {
                    walk_stmt(s, out);
                }
            }
            Stmt::For { var, iter, body, .. } => {
                if !out.contains(var) {
                    out.push(var.clone());
                }
                if !out.contains(&format!("__for_{}", var)) {
                    out.push(format!("__for_{}", var));
                }
                walk_expr(iter, out);
                for s in body {
                    walk_stmt(s, out);
                }
            }
            Stmt::Match { expr, arms } => {
                walk_expr(expr, out);
                for (_, e) in arms {
                    walk_expr(e, out);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }
    fn walk_expr(expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Binary { left, right, .. } => {
                walk_expr(left, out);
                walk_expr(right, out);
            }
            Expr::Unary { expr, .. } => walk_expr(expr, out),
            Expr::Call { args, .. } => {
                for a in args {
                    walk_expr(a, out);
                }
            }
            Expr::Block(stmts) => {
                for s in stmts {
                    walk_stmt(s, out);
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                walk_expr(condition, out);
                walk_expr(then_branch, out);
                if let Some(e) = else_branch {
                    walk_expr(e, out);
                }
            }
            Expr::Match { expr, arms } => {
                walk_expr(expr, out);
                for (_, e) in arms {
                    walk_expr(e, out);
                }
            }
            _ => {}
        }
    }
    for s in body {
        walk_stmt(s, &mut out);
    }
    out
}
