//! REDΣSIGMA NEAR Runtime
//!
//! NEAR Protocol runtime verification library for deploying and executing
//! REDΣSIGMA contracts with full mathematical verification.
//!
//! Execution is a tree-walking interpreter over the parsed contract AST. The
//! previous implementation dispatched on hardcoded method names
//! (`if method_name == "increment"`), which silently ignored every function it
//! did not recognise — `set`, `release` and `get_locked` returned an empty vec,
//! so state mutations never happened and `deposit` only ever touched `balance`.

use redsigma_parser::{BinOp, Contract, Expr, Stmt, Type, UnOp};
use redsigma_semantic::{SemanticAnalyzer, SemanticReport};
use redsigma_dualstream::{DualStreamAnalyzer, DualStreamReport};
use redsigma_reflection::{ReflectionEngine, ReflectionReport};
use redsigma_causal::CausalMemory;
use redsigma_codegen::{CodegenPipeline, WasmCompilation, ProofObject, gate231_checksum};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use std::collections::HashMap;

#[derive(Error, Debug)]
pub enum RuntimeError {
    #[error("Contract not found: {0}")]
    ContractNotFound(String),
    #[error("Verification failed: {0}")]
    VerificationFailed(String),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("State error: {0}")]
    StateError(String),
    #[error("NEAR host error: {0}")]
    HostError(String),
    #[error("Runtime error: {0}")]
    RuntimeError(String),
    #[error("Invariant violated: {0}")]
    InvariantViolation(String),
}

/// NEAR account ID
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Contract state key
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateKey(pub Vec<u8>);

/// Contract state value
pub type StateValue = Vec<u8>;

/// Execution context for a contract call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Contract account ID
    pub contract_id: AccountId,
    /// Caller account ID
    pub caller_id: AccountId,
    /// Predecessor account ID (immediate caller)
    pub predecessor_id: AccountId,
    /// Attached deposit (in yoctoNEAR)
    pub attached_deposit: u128,
    /// Prepaid gas
    pub prepaid_gas: u64,
    /// Block height
    pub block_height: u64,
    /// Block timestamp (nanoseconds since epoch)
    pub block_timestamp: u64,
    /// Random seed
    pub random_seed: Vec<u8>,
}

/// Contract deployment info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployedContract {
    /// Contract account ID
    pub account_id: AccountId,
    /// WASM bytecode produced by the codegen pipeline.
    ///
    /// **Carried and stored, never executed.** There is no WASM instantiation
    /// anywhere in this crate — `execute()` runs the tree-walking interpreter
    /// over `contract` (the AST), so this field is inert payload. It is
    /// non-empty (asserted in tests) and costs real storage when a
    /// `DeployedContract` is serialised, so it must not be described as the
    /// execution artifact.
    pub wasm_bytecode: Vec<u8>,
    /// Proof object
    pub proof: ProofObject,
    /// Contract source code
    pub source: String,
    /// Deployment block height
    pub deployed_at_height: u64,
    /// Contract state
    pub state: HashMap<StateKey, StateValue>,
    /// Parsed contract AST. The interpreter executes this directly, so the
    /// runtime is not a table of recognised method names.
    pub contract: Contract,
}

/// Gas costs for operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasCosts {
    pub base: u64,
    pub storage_read: u64,
    pub storage_write: u64,
    pub storage_remove: u64,
    pub math_op: u64,
    pub verification: u64,
}

impl Default for GasCosts {
    fn default() -> Self {
        Self {
            base: 1_000_000,
            storage_read: 100_000,
            storage_write: 500_000,
            storage_remove: 200_000,
            math_op: 10_000,
            verification: 10_000_000,
        }
    }
}

/// Interpreted value, carrying its declared type information so the runtime's
/// integer semantics match the verifier's per-width/signedness BitVec model.
///
/// `value` is stored sign-extended to i128 (two's complement). Unsigned values
/// are zero-extended (which for i128 storage is the same bit pattern).
/// `width` is the byte width of the declared type; `signed` is its signedness.
/// `u128`/`i128` are not representable here — the verifier declines them and so
/// does the runtime (they are rejected at `type_info` resolution).
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(i128, usize, bool), // (value, width bytes, signed)
    Bool(bool),
    Str(String),
    Unit,
}

/// Control flow out of a statement or block.
#[derive(Debug, Clone, PartialEq)]
enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

/// Upper bound on interpreted operations, so a `while true` cannot hang the runtime.
const STEP_LIMIT: u64 = 1_000_000;

/// Wrap an i128 value to the declared width, respecting signedness.
fn wrap_to_width(value: i128, width: usize, signed: bool) -> i128 {
    let bits = width * 8;
    if bits >= 128 {
        return value;
    }
    let mask = (1u128 << bits) - 1;
    let bits_u = (value as u128) & mask;
    if signed {
        // Sign-extend from `bits` bits.
        let sign_bit = 1u128 << (bits - 1);
        if bits_u & sign_bit != 0 {
            (bits_u | !mask) as i128
        } else {
            bits_u as i128
        }
    } else {
        bits_u as i128
    }
}

/// Extract the wrapped value, width, and signedness from an Int value.
fn int_parts(v: &Value) -> Option<(i128, usize, bool)> {
    match v {
        Value::Int(val, width, signed) => Some((*val, *width, *signed)),
        _ => None,
    }
}

/// Binary operation result type: the result width/signedness is the "wider" of
/// the two operands (matching the verifier, which declines mixed-width but we
/// need a concrete rule for the runtime's own arithmetic on typed values).
fn result_type(w1: usize, s1: bool, w2: usize, s2: bool) -> (usize, bool) {
    if w1 >= w2 {
        (w1, s1)
    } else {
        (w2, s2)
    }
}

/// Tree-walking interpreter for the REDΣSIGMA AST.
#[derive(Clone)]
struct Interpreter<'a> {
    contract: &'a Contract,
    /// Contract state fields, keyed by name. Values carry type info.
    state: HashMap<String, Value>,
    /// Stack of local scopes (function body + nested blocks).
    scopes: Vec<HashMap<String, Value>>,
    steps: u64,
}

impl<'a> Interpreter<'a> {
    fn new(contract: &'a Contract, state: HashMap<String, Value>) -> Self {
        Self {
            contract,
            state,
            scopes: Vec::new(),
            steps: 0,
        }
    }

    fn tick(&mut self) -> Result<(), RuntimeError> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            return Err(RuntimeError::ExecutionError(
                "Execution step limit exceeded".to_string(),
            ));
        }
        Ok(())
    }

    // ----- scope handling -----

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: &str, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), value);
        }
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        self.state.get(name).cloned()
    }

    /// Assign to an existing local, else to a state field.
    fn assign(&mut self, name: &str, value: Value) -> Result<(), RuntimeError> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return Ok(());
            }
        }
        if self.state.contains_key(name) {
            self.state.insert(name.to_string(), value);
            return Ok(());
        }
        // A bare assignment to a name that is neither a local nor a declared
        // state field is a contract bug; surface it instead of silently
        // creating state.
        Err(RuntimeError::ExecutionError(format!(
            "Assignment to unknown name '{}'",
            name
        )))
    }

    // ----- statements -----

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<(Flow, Value), RuntimeError> {
        self.push_scope();
        let mut last = Value::Unit;
        let mut flow = Flow::Normal;
        for stmt in stmts {
            let (f, v) = self.exec_stmt(stmt)?;
            last = v;
            if f != Flow::Normal {
                flow = f;
                break;
            }
        }
        self.pop_scope();
        Ok((flow, last))
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<(Flow, Value), RuntimeError> {
        self.tick()?;
        match stmt {
            Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
                let v = self.eval(value)?;
                self.define(name, v);
                Ok((Flow::Normal, Value::Unit))
            }
            Stmt::Expr(expr) => Ok((Flow::Normal, self.eval(expr)?)),
            Stmt::Return(expr) => {
                let v = match expr {
                    Some(e) => self.eval(e)?,
                    None => Value::Unit,
                };
                Ok((Flow::Return(v.clone()), v))
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.eval(condition)?;
                if cond == Value::Bool(true) {
                    self.exec_block(then_branch)
                } else if let Some(eb) = else_branch {
                    self.exec_block(eb)
                } else {
                    Ok((Flow::Normal, Value::Unit))
                }
            }
            Stmt::While { condition, body } => {
                loop {
                    self.tick()?;
                    if self.eval(condition)? != Value::Bool(true) {
                        break;
                    }
                    let (f, _) = self.exec_block(body)?;
                    match f {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok((Flow::Return(v.clone()), v)),
                        Flow::Normal | Flow::Continue => {}
                    }
                }
                Ok((Flow::Normal, Value::Unit))
            }
            Stmt::For { var, iter, body } => {
                let range = self.eval(iter)?;
                let (from, to) = match range {
                    Value::Int(n, _, _) => (0i128, n), // a bare integer iterates 0..n
                    _ => (0i128, 0),
                };
                let mut i = from;
                while i < to {
                    self.tick()?;
                    self.push_scope();
                    self.define(var, Value::Int(i, 8, false));
                    let (f, _) = self.exec_block(body)?;
                    self.pop_scope();
                    match f {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok((Flow::Return(v.clone()), v)),
                        Flow::Normal | Flow::Continue => {}
                    }
                    i += 1;
                }
                Ok((Flow::Normal, Value::Unit))
            }
            Stmt::Match { expr, arms } => {
                let scrut = self.eval(expr)?;
                for (pat, arm_expr) in arms {
                    if pat == "_" {
                        return Ok((Flow::Normal, self.eval(arm_expr)?));
                    }
                    if let (Value::Int(a, _, _), Ok(b)) = (&scrut, pat.parse::<i128>()) {
                        if a == &b {
                            return Ok((Flow::Normal, self.eval(arm_expr)?));
                        }
                    }
                }
                Ok((Flow::Normal, Value::Unit))
            }
            Stmt::Break => Ok((Flow::Break, Value::Unit)),
            Stmt::Continue => Ok((Flow::Continue, Value::Unit)),
        }
    }

    // ----- expressions -----

    fn eval(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        self.tick()?;
        match expr {
            Expr::Number(n) => n
                .parse::<i128>()
                .map(|v| Value::Int(v, 16, false))
                .map_err(|_| RuntimeError::ExecutionError(format!("Invalid number: {}", n))),
            Expr::String(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            Expr::Ident(name) => self.lookup(name).ok_or_else(|| {
                RuntimeError::ExecutionError(format!("Unknown identifier: {}", name))
            }),
            Expr::Block(stmts) => Ok(self.exec_block(stmts)?.1),
            Expr::Binary { left, op, right } => self.eval_binary(left, *op, right),
            Expr::Unary { op, expr } => {
                let v = self.eval(expr)?;
                match op {
                    UnOp::Not => Ok(Value::Bool(v != Value::Bool(true))),
                    UnOp::Neg => match v {
                        Value::Int(n, width, signed) => {
                            let r = wrap_to_width((!n).wrapping_add(1), width, signed);
                            Ok(Value::Int(r, width, signed))
                        }
                        _ => Err(RuntimeError::ExecutionError(
                            "Cannot negate a non-integer".to_string(),
                        )),
                    },
                    _ => Ok(v), // Deref/Ref/RefMut are identity in this subset
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.eval(condition)? == Value::Bool(true) {
                    self.eval(then_branch)
                } else if let Some(eb) = else_branch {
                    self.eval(eb)
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Match { expr, arms } => {
                let scrut = self.eval(expr)?;
                for (pat, arm_expr) in arms {
                    if pat == "_" {
                        return self.eval(arm_expr);
                    }
                    if let (Value::Int(a, _, _), Ok(b)) = (&scrut, pat.parse::<i128>()) {
                        if a == &b {
                            return self.eval(arm_expr);
                        }
                    }
                }
                Ok(Value::Unit)
            }
            Expr::Range { from, to } => {
                // Represented as a half-open range packed into one integer is not
                // meaningful; callers use `for i in 0..n`, handled in Stmt::For.
                let _ = (from, to);
                Ok(Value::Unit)
            }
            Expr::Call { func, args } => self.call_function(func, args),
            // The remaining forms are not part of the executable subset yet.
            Expr::MethodCall { .. } => Err(RuntimeError::ExecutionError(
                "Method calls are not supported by the interpreter yet".to_string(),
            )),
            Expr::FieldAccess { .. } => Err(RuntimeError::ExecutionError(
                "Field access is not supported by the interpreter yet".to_string(),
            )),
            Expr::Index { .. } => Err(RuntimeError::ExecutionError(
                "Indexing is not supported by the interpreter yet".to_string(),
            )),
            Expr::Array(_) => Err(RuntimeError::ExecutionError(
                "Array literals are not supported by the interpreter yet".to_string(),
            )),
            Expr::Struct { .. } => Err(RuntimeError::ExecutionError(
                "Struct literals are not supported by the interpreter yet".to_string(),
            )),
        }
    }

    /// Evaluate an expression against the post-state interpreter, with `old(e)`
    /// resolved against the optional pre-state interpreter.
    fn eval_with_pre(
        &mut self,
        expr: &Expr,
        pre_state: Option<Interpreter<'_>>,
    ) -> Result<Value, RuntimeError> {
        self.tick()?;
        match expr {
            Expr::Number(n) => n
                .parse::<i128>()
                .map(|v| Value::Int(v, 16, false))
                .map_err(|_| RuntimeError::ExecutionError(format!("Invalid number: {n}"))),
            Expr::String(s) => Ok(Value::Str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Unit => Ok(Value::Unit),
            Expr::Ident(name) => self.lookup(name).ok_or_else(|| {
                RuntimeError::ExecutionError(format!("Unknown identifier: {name}"))
            }),
            Expr::Block(stmts) => Ok(self.exec_block(stmts)?.1),
            Expr::Binary { left, op, right } => {
                let l = self.eval_with_pre(left, pre_state.clone())?;
                let r = self.eval_with_pre(right, pre_state.clone())?;
                self.eval_binary_values(l, *op, r)
            }
            Expr::Unary { op, expr } => {
                let v = self.eval_with_pre(expr, pre_state.clone())?;
                match op {
                    UnOp::Not => Ok(Value::Bool(v != Value::Bool(true))),
                    UnOp::Neg => match v {
                        Value::Int(n, width, signed) => {
                            let r = wrap_to_width((!n).wrapping_add(1), width, signed);
                            Ok(Value::Int(r, width, signed))
                        }
                        _ => Err(RuntimeError::ExecutionError(
                            "Cannot negate a non-integer".to_string(),
                        )),
                    },
                    _ => Ok(v),
                }
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.eval_with_pre(condition, pre_state.clone())? == Value::Bool(true) {
                    self.eval_with_pre(then_branch, pre_state.clone())
                } else if let Some(eb) = else_branch {
                    self.eval_with_pre(eb, pre_state.clone())
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Match { expr, arms } => {
                let scrut = self.eval_with_pre(expr, pre_state.clone())?;
                for (pat, arm_expr) in arms {
                    if pat == "_" {
                        return self.eval_with_pre(arm_expr, pre_state.clone());
                    }
                    if let (Value::Int(a, _, _), Ok(b)) = (&scrut, pat.parse::<i128>()) {
                        if a == &b {
                            return self.eval_with_pre(arm_expr, pre_state.clone());
                        }
                    }
                }
                Ok(Value::Unit)
            }
            Expr::Range { from, to } => {
                let _ = (from, to);
                Ok(Value::Unit)
            }
            Expr::Call { func, args } if func == "old" => {
                let arg = args
                    .first()
                    .ok_or_else(|| {
                        RuntimeError::ExecutionError(
                            "old() requires exactly one argument".to_string(),
                        )
                    })?;
                let mut pre = pre_state.ok_or_else(|| {
                    RuntimeError::ExecutionError(
                        "old() outside of postcondition context".to_string(),
                    )
                })?;
                pre.eval(arg)
            }
            Expr::Call { func, args } => self.call_function(func, args),
            Expr::MethodCall { .. } => Err(RuntimeError::ExecutionError(
                "Method calls are not supported by the interpreter yet".to_string(),
            )),
            Expr::FieldAccess { .. } => Err(RuntimeError::ExecutionError(
                "Field access is not supported by the interpreter yet".to_string(),
            )),
            Expr::Index { .. } => Err(RuntimeError::ExecutionError(
                "Indexing is not supported by the interpreter yet".to_string(),
            )),
            Expr::Array(_) => Err(RuntimeError::ExecutionError(
                "Array literals are not supported by the interpreter yet".to_string(),
            )),
            Expr::Struct { .. } => Err(RuntimeError::ExecutionError(
                "Struct literals are not supported by the interpreter yet".to_string(),
            )),
        }
    }

    /// Evaluate a binary operation on two already-evaluated values.
    ///
    /// Each `Value::Int` carries `(value, width, signed)`. Arithmetic wraps at
    /// the declared width; comparisons respect signedness.
    fn eval_binary_values(
        &mut self,
        left: Value,
        op: BinOp,
        right: Value,
    ) -> Result<Value, RuntimeError> {
        let (a, aw, asig) = match &left {
            Value::Int(v, w, s) => (*v, *w, *s),
            _ => {
                return Err(RuntimeError::ExecutionError(format!(
                    "Arithmetic on non-integers: {:?} {:?}",
                    left, right
                )))
            }
        };
        let (b, bw, bsig) = match &right {
            Value::Int(v, w, s) => (*v, *w, *s),
            _ => {
                return Err(RuntimeError::ExecutionError(format!(
                    "Arithmetic on non-integers: {:?} {:?}",
                    left, right
                )))
            }
        };

        let (rw, rsig) = result_type(aw, asig, bw, bsig);

        // Wrap both operands to the result width before operating.
        let a = wrap_to_width(a, aw, asig);
        let b = wrap_to_width(b, bw, bsig);

        match op {
            BinOp::Add => {
                let r = wrap_to_width(a.wrapping_add(b), rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Sub => {
                let r = wrap_to_width(a.wrapping_sub(b), rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Mul => {
                let r = wrap_to_width(a.wrapping_mul(b), rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Div => {
                if b == 0 {
                    Err(RuntimeError::ExecutionError("Division by zero".to_string()))
                } else {
                    // WASM integer division truncates toward zero.
                    let q = a / b;
                    let r = wrap_to_width(q, rw, rsig);
                    Ok(Value::Int(r, rw, rsig))
                }
            }
            BinOp::Mod => {
                if b == 0 {
                    Err(RuntimeError::ExecutionError("Division by zero".to_string()))
                } else {
                    let r = a % b;
                    let r = wrap_to_width(r, rw, rsig);
                    Ok(Value::Int(r, rw, rsig))
                }
            }
            BinOp::Eq => Ok(Value::Bool(a == b)),
            BinOp::Ne => Ok(Value::Bool(a != b)),
            BinOp::Lt => {
                let result = if rsig {
                    (a as i128) < (b as i128)
                } else {
                    (a as u128) < (b as u128)
                };
                Ok(Value::Bool(result))
            }
            BinOp::Gt => {
                let result = if rsig {
                    (a as i128) > (b as i128)
                } else {
                    (a as u128) > (b as u128)
                };
                Ok(Value::Bool(result))
            }
            BinOp::Le => {
                let result = if rsig {
                    (a as i128) <= (b as i128)
                } else {
                    (a as u128) <= (b as u128)
                };
                Ok(Value::Bool(result))
            }
            BinOp::Ge => {
                let result = if rsig {
                    (a as i128) >= (b as i128)
                } else {
                    (a as u128) >= (b as u128)
                };
                Ok(Value::Bool(result))
            }
            BinOp::And => Ok(Value::Bool(a != 0 && b != 0)),
            BinOp::Or => Ok(Value::Bool(a != 0 || b != 0)),
            BinOp::BitAnd => {
                let r = a & b;
                let r = wrap_to_width(r, rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::BitOr => {
                let r = a | b;
                let r = wrap_to_width(r, rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::BitXor => {
                let r = a ^ b;
                let r = wrap_to_width(r, rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Shl => {
                let count = (b as u32) % (rw as u32 * 8);
                let r = a.wrapping_shl(count);
                let r = wrap_to_width(r, rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Shr => {
                let count = (b as u32) % (rw as u32 * 8);
                let r = if rsig {
                    (a as i128).wrapping_shr(count)
                } else {
                    (a as u128).wrapping_shr(count) as i128
                };
                let r = wrap_to_width(r, rw, rsig);
                Ok(Value::Int(r, rw, rsig))
            }
            BinOp::Range => Ok(Value::Unit),
            BinOp::Assign => unreachable!("handled above"),
        }
    }

    fn eval_binary(
        &mut self,
        left: &Expr,
        op: BinOp,
        right: &Expr,
    ) -> Result<Value, RuntimeError> {
        if op == BinOp::Assign {
            let value = self.eval(right)?;
            match left {
                Expr::Ident(name) => {
                    self.assign(name, value.clone())?;
                    return Ok(value);
                }
                _ => {
                    return Err(RuntimeError::ExecutionError(
                        "Assignment target must be a name".to_string(),
                    ))
                }
            }
        }

        let l = self.eval(left)?;
        let r = self.eval(right)?;
        self.eval_binary_values(l, op, r)
    }

    fn call_function(&mut self, name: &str, args: &[Expr]) -> Result<Value, RuntimeError> {
        let func = self
            .contract
            .functions
            .iter()
            .find(|f| f.name == name)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::ExecutionError(format!("Unknown function: {}", name))
            })?;

        let mut evaluated = Vec::with_capacity(args.len());
        for arg in args {
            evaluated.push(self.eval(arg)?);
        }

        self.push_scope();
        for ((param, _), value) in func.params.iter().zip(evaluated.into_iter()) {
            self.define(param, value);
        }
        let (flow, value) = self.exec_block(&func.body)?;
        self.pop_scope();

        match flow {
            Flow::Return(v) => Ok(v),
            _ => Ok(value),
        }
    }
}

/// Byte width of an integer type, used to encode state and return values.
///
/// Mirrors the verifier's model: per-width and per-signedness. `u128`/`i128`
/// are rejected here just as the z3 verifier declines them (no 128-bit model).
fn type_info(name: &str) -> Option<(usize, bool)> {
    // (width in bytes, is_signed)
    let bare = name.strip_prefix("Verified").unwrap_or(name);
    match bare {
        "u8" | "bool" => Some((1, false)),
        "i8" => Some((1, true)),
        "u16" => Some((2, false)),
        "i16" => Some((2, true)),
        "u32" => Some((4, false)),
        "i32" => Some((4, true)),
        "u64" => Some((8, false)),
        "i64" => Some((8, true)),
        // u128/i128: the verifier declines them; so does the runtime.
        "u128" | "i128" => None,
        _ => None,
    }
}

/// Width in bytes for an integer type (kept for callers that only need width).
fn type_width(name: &str) -> Option<usize> {
    type_info(name).map(|(w, _)| w)
}

/// Whether a type is signed.
fn type_signed(name: &str) -> Option<bool> {
    type_info(name).map(|(_, s)| s)
}

fn encode_int(n: i128, width: usize) -> Vec<u8> {
    let bytes = n.to_le_bytes();
    bytes[..width.min(16)].to_vec()
}

fn decode_int(bytes: &[u8], width: usize, signed: bool) -> i128 {
    let mut buf = [0u8; 16];
    let n = bytes.len().min(width).min(16);
    buf[..n].copy_from_slice(&bytes[..n]);
    let unsigned = u128::from_le_bytes(buf);
    if signed {
        // Sign-extend from `width` bits to i128.
        let shift = (128 - width * 8) as u32;
        (unsigned as i128).checked_shl(shift).unwrap_or(0) >> shift
    } else {
        unsigned as i128
    }
}

fn encode_value(value: &Value, ty: &Type) -> Vec<u8> {
    match value {
        Value::Int(n, width, signed) => match type_info(&ty.name) {
            Some((w, s)) => {
                let wrapped = wrap_to_width(*n, *width, *signed);
                encode_int(wrapped, w)
            }
            None => encode_int(*n, 16),
        },
        Value::Bool(b) => vec![*b as u8],
        Value::Str(s) => s.clone().into_bytes(),
        Value::Unit => Vec::new(),
    }
}

/// Decode the stored byte state into named integer values.
fn decode_state(contract: &Contract, state: &HashMap<StateKey, StateValue>) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    for (name, ty) in &contract.state.fields {
        let value = match type_info(&ty.name) {
            Some((w, signed)) => {
                let raw = state
                    .get(&StateKey(name.as_bytes().to_vec()))
                    .map(|v| decode_int(v, w, signed))
                    .unwrap_or(0);
                Value::Int(raw, w, signed)
            }
            None => Value::Unit,
        };
        out.insert(name.clone(), value);
    }
    out
}

/// Evaluate every contract invariant over the decoded state.
///
/// This is the layer that can actually reject a real state transition: the
/// checksum and reflection layers describe the AST, but only this evaluates the
/// contract's own declared invariants against the values it is about to commit.
/// An invariant that does not evaluate to `true` — or cannot be evaluated at
/// all — is a hard failure, never a warning.
fn check_invariants(
    contract: &Contract,
    state_bytes: &HashMap<StateKey, StateValue>,
) -> Result<(), RuntimeError> {
    let decoded = decode_state(contract, state_bytes);
    for inv in &contract.invariants {
        let mut interpreter = Interpreter::new(contract, decoded.clone());
        let source = redsigma_parser::expr_to_source(&inv.expr);
        match interpreter.eval(&inv.expr) {
            Ok(Value::Bool(true)) => {}
            Ok(Value::Bool(false)) => {
                return Err(RuntimeError::InvariantViolation(format!(
                    "invariant does not hold: {}",
                    source
                )));
            }
            Ok(other) => {
                return Err(RuntimeError::ExecutionError(format!(
                    "invariant evaluated to a non-boolean value: {} -> {:?}",
                    source, other
                )));
            }
            Err(e) => {
                return Err(RuntimeError::ExecutionError(format!(
                    "invariant could not be evaluated: {} ({})",
                    source, e
                )));
            }
        }
    }
    Ok(())
}

/// Run one contract function over the AST. `read_only` discards state writes.
fn run_function(
    contract: &Contract,
    state: &mut HashMap<StateKey, StateValue>,
    method_name: &str,
    args: &[u8],
    read_only: bool,
) -> Result<Vec<u8>, RuntimeError> {
    let func = contract
        .functions
        .iter()
        .find(|f| f.name == method_name)
        .ok_or_else(|| RuntimeError::ExecutionError(format!("Method not found: {}", method_name)))?;

    // Seed the interpreter from the stored byte state.
    let mut interpreter = Interpreter::new(contract, decode_state(contract, state));

    // Build a pre-state snapshot interpreter for `old()` resolution in
    // postconditions and for precondition checking.
    let mut pre_state = Interpreter::new(contract, decode_state(contract, state));

    // Function parameters are defined in both interpreters so that
    // preconditions can reference them.
    interpreter.push_scope();
    pre_state.push_scope();
    let mut param_values = Vec::with_capacity(func.params.len());
    let mut offset = 0usize;
    for (param, ty) in &func.params {
        let (width, signed) = type_info(&ty.name).unwrap_or((16, false));
        let value = if args.len() >= offset + width {
            let raw = decode_int(&args[offset..offset + width], width, signed);
            Value::Int(raw, width, signed)
        } else {
            Value::Int(0, width, signed)
        };
        param_values.push((param.clone(), value));
        offset += width;
    }
    for (name, value) in &param_values {
        pre_state.define(name, value.clone());
        interpreter.define(name, value.clone());
    }

    // --- Precondition check ---
    if let Some(pre_expr) = &func.verified.as_ref().and_then(|v| v.pre.as_ref()) {
        let mut pre = pre_state.clone();
        match pre.eval(pre_expr) {
            Ok(Value::Bool(true)) => {}
            Ok(Value::Bool(false)) => {
                return Err(RuntimeError::ExecutionError(format!(
                    "Precondition failed: {}",
                    redsigma_parser::expr_to_source(pre_expr)
                )));
            }
            Ok(other) => {
                return Err(RuntimeError::ExecutionError(format!(
                    "Precondition evaluated to non-boolean: {:?}",
                    other
                )));
            }
            Err(e) => {
                return Err(RuntimeError::ExecutionError(format!(
                    "Precondition could not be evaluated: {}",
                    e
                )));
            }
        }
    }

    let (flow, value) = interpreter.exec_block(&func.body)?;
    interpreter.pop_scope();
    pre_state.pop_scope();

    let result = match flow {
        Flow::Return(v) => v,
        _ => value,
    };

    // Define `result` in the post-state interpreter for postcondition checking.
    interpreter.push_scope();
    interpreter.define("result", result.clone());

    // --- Postcondition check ---
    if !read_only {
        if let Some(post_expr) = &func.verified.as_ref().and_then(|v| v.post.as_ref()) {
            // Evaluate postcondition against the post-state interpreter, with
            // `old(e)` resolved against the pre-state snapshot.
            match interpreter.eval_with_pre(post_expr, Some(pre_state.clone())) {
                Ok(Value::Bool(true)) => {}
                Ok(Value::Bool(false)) => {
                    return Err(RuntimeError::ExecutionError(format!(
                        "Postcondition failed: {}",
                        redsigma_parser::expr_to_source(post_expr)
                    )));
                }
                Ok(other) => {
                    return Err(RuntimeError::ExecutionError(format!(
                        "Postcondition evaluated to non-boolean: {:?}",
                        other
                    )));
                }
                Err(e) => {
                    return Err(RuntimeError::ExecutionError(format!(
                        "Postcondition could not be evaluated: {}",
                        e
                    )));
                }
            }
        }

        // Overwrite the declared fields, preserving any undeclared keys.
        let mut stored = state.clone();
        for (name, ty) in &contract.state.fields {
            let v = interpreter.state.get(name).cloned().unwrap_or(Value::Unit);
            stored.insert(StateKey(name.as_bytes().to_vec()), encode_value(&v, ty));
        }
        *state = stored;
    }

    Ok(encode_value(&result, &func.return_type))
}

/// NEAR Runtime for REDΣSIGMA contracts
pub struct NearRuntime {
    /// Deployed contracts
    contracts: HashMap<AccountId, DeployedContract>,
    /// Codegen pipeline for verification
    codegen_pipeline: CodegenPipeline,
    /// Causal memory for transaction history
    causal_memory: CausalMemory,
    /// Gas costs (simplified)
    gas_costs: GasCosts,
}

impl NearRuntime {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            codegen_pipeline: CodegenPipeline::new(),
            causal_memory: CausalMemory::new(),
            gas_costs: GasCosts::default(),
        }
    }

    /// Deploy a new contract
    pub fn deploy_contract(
        &mut self,
        account_id: AccountId,
        source: &str,
        deployer: AccountId,
        block_height: u64,
    ) -> Result<(), RuntimeError> {
        // Parse and verify contract
        let contracts = redsigma_parser::Parser::parse(source)
            .map_err(|e| RuntimeError::VerificationFailed(format!("Parse error: {}", e)))?;

        if contracts.len() != 1 {
            return Err(RuntimeError::VerificationFailed(
                format!("Expected exactly 1 contract, found {}", contracts.len())
            ));
        }

        let contract = contracts[0].clone();
        let contract_name = contract.name.clone();

        // Compile with full verification
        let mut pipeline = CodegenPipeline::new();
        let compilation = pipeline.compile(&contract)
            .map_err(|e| RuntimeError::VerificationFailed(format!("Compilation error: {}", e)))?;

        // Store deployed contract
        let deployed = DeployedContract {
            account_id: account_id.clone(),
            wasm_bytecode: compilation.bytecode,
            proof: compilation.proof,
            source: compilation.source,
            deployed_at_height: block_height,
            state: HashMap::new(),
            contract,
        };

        let gate231 = gate231_checksum(&deployed.contract);
        self.contracts.insert(account_id.clone(), deployed);

        // Check that the initial (zero) state satisfies all contract
        // invariants. Invariants are preservation-only on transitions, but the
        // initial state must satisfy them too — otherwise a contract could be
        // deployed in a state that already violates its own invariants.
        let deployed = self.contracts.get(&account_id).unwrap();
        check_invariants(&deployed.contract, &deployed.state)?;

        // Record the deployment as the root of this contract's causal chain.
        let _ = self.causal_memory.record_transition(
            &contract_name,
            "deploy",
            0,
            gate231,
            gate231,
            0,
            None,
        );

        Ok(())
    }

    /// Execute a contract function
    pub fn execute(
        &mut self,
        contract_id: &AccountId,
        method_name: &str,
        args: Vec<u8>,
        context: ExecutionContext,
    ) -> Result<Vec<u8>, RuntimeError> {
        // Verify proof object before touching state.
        {
            let contract = self.contracts.get(contract_id)
                .ok_or_else(|| RuntimeError::ContractNotFound(contract_id.0.clone()))?;
            self.verify_proof(&contract.proof, &contract.contract)?;
        }

        // Check gas
        if context.prepaid_gas < self.gas_costs.verification {
            return Err(RuntimeError::ExecutionError("Insufficient gas for verification".to_string()));
        }

        // Interpret the function against the contract AST under invariant
        // enforcement: check the pre-state, run, then check the post-state and
        // roll the state back if it does not hold. A rejected transition must
        // leave storage byte-identical.
        let outcome = {
            let contract = self.contracts.get_mut(contract_id)
                .ok_or_else(|| RuntimeError::ContractNotFound(contract_id.0.clone()))?;

            check_invariants(&contract.contract, &contract.state)?;
            let pre = state_signature(&contract.state);
            let snapshot = contract.state.clone();

            match run_function(&contract.contract, &mut contract.state, method_name, &args, false) {
                Ok(r) => match check_invariants(&contract.contract, &contract.state) {
                    Ok(()) => {
                        let post = state_signature(&contract.state);
                        let gate = gate231_checksum(&contract.contract);
                        Ok((r, pre, post, gate))
                    }
                    Err(e) => {
                        contract.state = snapshot;
                        Err(e)
                    }
                },
                Err(e) => {
                    contract.state = snapshot;
                    Err(e)
                }
            }
        };
        let (result, pre_state_sig, post_state_sig, gate231) = outcome?;

        // Record transaction in causal memory
        let tx_hash = self.compute_tx_hash(&context, method_name, &args);
        let _ = self.causal_memory.record_transition(
            &contract_id.0,
            method_name,
            pre_state_sig,
            post_state_sig,
            gate231,
            tx_hash,
            None,
        );

        Ok(result)
    }

    /// View call (read-only, no state changes)
    pub fn view(
        &self,
        contract_id: &AccountId,
        method_name: &str,
        args: Vec<u8>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let contract = self.contracts.get(contract_id)
            .ok_or_else(|| RuntimeError::ContractNotFound(contract_id.0.clone()))?;

        // Verify proof
        self.verify_proof(&contract.proof, &contract.contract)?;

        // A view call must still respect the invariants of the state it reads.
        check_invariants(&contract.contract, &contract.state)?;

        // Simulate view call on a scratch copy so no state can change.
        let mut scratch = contract.state.clone();
        run_function(&contract.contract, &mut scratch, method_name, &args, true)
    }

    /// Get contract state
    pub fn get_state(&self, contract_id: &AccountId, key: &StateKey) -> Option<StateValue> {
        self.contracts.get(contract_id)
            .and_then(|c| c.state.get(key).cloned())
    }

    /// Set contract state
    pub fn set_state(&mut self, contract_id: &AccountId, key: StateKey, value: StateValue) -> Result<(), RuntimeError> {
        let contract = self.contracts.get_mut(contract_id)
            .ok_or_else(|| RuntimeError::ContractNotFound(contract_id.0.clone()))?;
        contract.state.insert(key, value);
        Ok(())
    }

    /// Remove contract state
    pub fn remove_state(&mut self, contract_id: &AccountId, key: &StateKey) -> Result<(), RuntimeError> {
        let contract = self.contracts.get_mut(contract_id)
            .ok_or_else(|| RuntimeError::ContractNotFound(contract_id.0.clone()))?;
        contract.state.remove(key);
        Ok(())
    }

    /// Verify a proof object against the contract it claims to describe.
    ///
    /// This used to read four flags off a `ProofObject` produced by the same
    /// `compile()` call in the same process, which proves nothing: the issuer and
    /// the checker were the same code. Every field is now **re-derived** from the
    /// AST and compared, so a proof that does not match the contract it names is
    /// rejected regardless of who issued it.
    pub fn verify_proof(&self, proof: &ProofObject, contract: &Contract) -> Result<(), RuntimeError> {
        if proof.contract_name != contract.name {
            return Err(RuntimeError::VerificationFailed(format!(
                "proof is for contract '{}', not '{}'",
                proof.contract_name, contract.name
            )));
        }

        // 231-gate checksum: re-derive from the AST.
        let gate = gate231_checksum(contract);
        if proof.gate231_checksum != gate {
            return Err(RuntimeError::VerificationFailed(format!(
                "231-gate checksum mismatch: proof says {}, re-derived {}",
                proof.gate231_checksum, gate
            )));
        }

        // Re-run the verification pipeline independently and compare the
        // signatures rather than the recorded pass flags.
        let mut pipeline = CodegenPipeline::new();
        let rederived = pipeline
            .compile(contract)
            .map_err(|e| RuntimeError::VerificationFailed(format!("re-derivation failed: {}", e)))?;

        if rederived.proof.semantic_report.semantic_signature
            != proof.semantic_report.semantic_signature
        {
            return Err(RuntimeError::VerificationFailed(
                "semantic signature mismatch after re-derivation".to_string(),
            ));
        }
        if rederived.proof.dual_stream_report.joint_signature
            != proof.dual_stream_report.joint_signature
        {
            return Err(RuntimeError::VerificationFailed(
                "dual-stream joint signature mismatch after re-derivation".to_string(),
            ));
        }
        if rederived.proof.reflection_report.overall_passed != proof.reflection_report.overall_passed {
            return Err(RuntimeError::VerificationFailed(
                "reflection verdict changed on re-derivation".to_string(),
            ));
        }

        // The re-derived pipeline must itself pass; a proof asserting a failing
        // verdict is not a verification.
        if !rederived.proof.reflection_report.overall_passed {
            return Err(RuntimeError::VerificationFailed(
                "Reflection verification failed".to_string(),
            ));
        }
        if rederived.proof.dual_stream_report.collisions > 0 {
            return Err(RuntimeError::VerificationFailed(
                "Dual-stream collision detected".to_string(),
            ));
        }

        Ok(())
    }

    /// Compute transaction hash
    fn compute_tx_hash(&self, context: &ExecutionContext, method: &str, args: &[u8]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        context.caller_id.0.hash(&mut hasher);
        context.predecessor_id.0.hash(&mut hasher);
        method.hash(&mut hasher);
        args.hash(&mut hasher);
        context.block_height.hash(&mut hasher);
        context.block_timestamp.hash(&mut hasher);
        hasher.finish()
    }

    /// Get deployed contract info
    pub fn get_contract(&self, contract_id: &AccountId) -> Option<&DeployedContract> {
        self.contracts.get(contract_id)
    }

    /// Insert a pre-built deployed contract (used by the NEAR test contract).
    pub fn insert_contract(&mut self, account_id: AccountId, deployed: DeployedContract) {
        self.contracts.insert(account_id, deployed);
    }

    /// List all deployed contracts
    pub fn list_contracts(&self) -> Vec<AccountId> {
        self.contracts.keys().cloned().collect()
    }

    /// Get causal memory for inspection
    pub fn causal_memory(&self) -> &CausalMemory {
        &self.causal_memory
    }
}

impl Default for NearRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Cheap deterministic signature over the byte state, for causal chain links.
fn state_signature(state: &HashMap<StateKey, StateValue>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    let mut keys: Vec<&StateKey> = state.keys().collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    for key in keys {
        key.0.hash(&mut hasher);
        state[key].hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deploy_contract() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        let result = runtime.deploy_contract(account_id.clone(), input, deployer, 1000);
        assert!(result.is_ok());

        let contract = runtime.get_contract(&account_id).unwrap();
        assert_eq!(contract.account_id.0, "counter.testnet");
        assert!(!contract.wasm_bytecode.is_empty());
        assert_eq!(contract.proof.contract_name, "Counter");
        assert_eq!(contract.contract.name, "Counter");
    }

    #[test]
    fn test_execute_increment() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        // First increment
        let result = runtime.execute(&account_id, "increment", vec![], context.clone()).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 1);

        // Second increment
        let result = runtime.execute(&account_id, "increment", vec![], context.clone()).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 2);
    }

    #[test]
    fn test_view_call() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        // Execute increment first
        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime.execute(&account_id, "increment", vec![], context).unwrap();

        // View call
        let result = runtime.view(&account_id, "get", vec![]).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 1);
    }

    #[test]
    fn test_escrow_deposit() {
        // u64, not u128: deploy_contract lowers through the WAT backend, whose
        // compiled subset is u8..u64 (u128 is rejected rather than truncated).
        let input = r#"
            contract Escrow {
                invariant balance >= 0;
                invariant locked + released == total_deposited;
                state {
                    balance: u64,
                    locked: u64,
                    released: u64,
                    total_deposited: u64,
                }
                fn deposit(amount: u64) -> u64 {
                    let new_balance = balance + amount;
                    let new_locked = locked + amount;
                    let new_total = total_deposited + amount;
                    balance = new_balance;
                    locked = new_locked;
                    total_deposited = new_total;
                    new_locked;
                }
                fn get_balance() -> u64 { balance; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("escrow.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 100,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        // Deposit 100
        let amount = 100u64.to_le_bytes().to_vec();
        let result = runtime.execute(&account_id, "deposit", amount, context.clone()).unwrap();
        let locked = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(locked, 100);

        // Deposit another 50
        let amount = 50u64.to_le_bytes().to_vec();
        let result = runtime.execute(&account_id, "deposit", amount, context).unwrap();
        let locked = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(locked, 150);
    }

    #[test]
    fn test_state_management() {
        let input = r#"
            contract Test {
                invariant x >= 0;
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("test.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        // Set state
        let key = StateKey(b"custom_key".to_vec());
        let value = b"custom_value".to_vec();
        runtime.set_state(&account_id, key.clone(), value.clone()).unwrap();

        // Get state
        let retrieved = runtime.get_state(&account_id, &key).unwrap();
        assert_eq!(retrieved, b"custom_value");

        // Remove state
        runtime.remove_state(&account_id, &key).unwrap();
        assert!(runtime.get_state(&account_id, &key).is_none());
    }

    #[test]
    fn test_proof_verification() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let contract = runtime.get_contract(&account_id).unwrap();
        let result = runtime.verify_proof(&contract.proof, &contract.contract);
        assert!(result.is_ok());
    }

    fn bounded_context(account_id: &AccountId) -> ExecutionContext {
        ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        }
    }

    #[test]
    fn test_invariant_violation_is_rejected_and_rolled_back() {
        // `count <= 100` is the contract's own declared invariant. A transition
        // that would commit 200 must be rejected, and the state must be left
        // byte-identical — a rejected call cannot silently persist half of itself.
        let input = r#"
            contract Bounded {
                invariant count <= 100;
                state { count: u64 }
                fn set_count(v: u64) -> u64 { count = v; v; }
                fn get_count() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("bounded.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = bounded_context(&account_id);
        let err = runtime
            .execute(&account_id, "set_count", 200u64.to_le_bytes().to_vec(), context.clone())
            .expect_err("a transition that violates the declared invariant must be rejected");
        assert!(
            err.to_string().contains("invariant does not hold"),
            "unexpected error: {}",
            err
        );

        // The rejected transition must not have been persisted.
        let result = runtime.view(&account_id, "get_count", vec![]).unwrap();
        let stored = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(stored, 0, "state must be unchanged after a rejected transition");
    }

    #[test]
    fn test_invariant_satisfied_allows_transition() {
        let input = r#"
            contract Bounded {
                invariant count <= 100;
                state { count: u64 }
                fn set_count(v: u64) -> u64 { count = v; v; }
                fn get_count() -> u64 { count; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("bounded.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = bounded_context(&account_id);
        runtime
            .execute(&account_id, "set_count", 50u64.to_le_bytes().to_vec(), context)
            .unwrap();
        let result = runtime.view(&account_id, "get_count", vec![]).unwrap();
        let stored = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(stored, 50);
    }

    #[test]
    fn test_initial_state_invariant_must_hold() {
        // `count > 0` is violated by the zero-initial state (count == 0),
        // so deployment must be rejected.
        let input = r#"
            contract Positive {
                invariant count >= 1;
                state { count: u64 }
                fn set_count(v: u64) -> u64 { count = v; v; }
                fn get_count() -> u64 { count; }
            }
"#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("positive.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        let err = runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .expect_err("deployment with an unsatisfied initial invariant must fail");
        assert!(
            err.to_string().contains("invariant does not hold"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_initial_state_invariant_satisfied_allows_deploy() {
        // `count >= 0` holds for the zero state, so deployment succeeds.
        let input = r#"
            contract NonNeg {
                invariant count >= 0;
                state { count: u64 }
                fn set_count(v: u64) -> u64 { count = v; v; }
                fn get_count() -> u64 { count; }
            }
"#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("nonneg.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();
    }

    #[test]
    fn test_proof_rejects_tampered_contract() {
        // A proof must describe the contract it names. Removing an invariant
        // after the proof was issued changes the AST, so re-derivation must
        // disagree with the recorded proof and reject it.
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state { count: u64 }
                fn increment() -> u64 { let n = count + 1; count = n; n; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let proof = runtime.get_contract(&account_id).unwrap().proof.clone();
        assert!(runtime.verify_proof(&proof, &runtime.get_contract(&account_id).unwrap().contract).is_ok());

        let mut tampered = runtime.get_contract(&account_id).unwrap().clone();
        tampered.contract.invariants.clear();
        let err = runtime
            .verify_proof(&proof, &tampered.contract)
            .expect_err("a proof must not verify against a different AST");
        assert!(
            err.to_string().to_lowercase().contains("mismatch"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_causal_memory_tracking() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state { count: u64 }
                fn increment() -> u64 { let new = count + 1; count = new; new; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());

        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime.execute(&account_id, "increment", vec![], context.clone()).unwrap();
        runtime.execute(&account_id, "increment", vec![], context.clone()).unwrap();

        let causal_memory = runtime.causal_memory();
        assert!(!causal_memory.is_empty());
        // deploy + 2 increments
        assert!(causal_memory.len() >= 3);
    }

    #[test]
    fn test_assignment_is_not_addition() {
        // Regression: `x = v` used to parse as `x + v`, so nothing was ever
        // written and the function returned the sum instead of the value.
        let input = r#"
            contract Assign {
                invariant x >= 0;
                state { x: u64 }
                fn set(v: u64) -> u64 { x = v; v; }
                fn get() -> u64 { x; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("assign.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime.execute(&account_id, "set", 7u64.to_le_bytes().to_vec(), context).unwrap();
        let result = runtime.view(&account_id, "get", vec![]).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 7);
    }

    #[test]
    fn test_precondition_rejects_invalid_call() {
        // `pre: v > 0` — calling set(0) should fail the precondition.
        let input = r#"
            contract Pre {
                invariant x >= 0;
                state { x: u64 }
                @verified(pre: v > 0)
                fn set(v: u64) -> u64 { x = v; v; }
                fn get() -> u64 { x; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("pre.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        // v=0 violates pre: v > 0
        let err = runtime
            .execute(
                &account_id,
                "set",
                0u64.to_le_bytes().to_vec(),
                context.clone(),
            )
            .expect_err("precondition v > 0 should reject v=0");
        assert!(
            err.to_string().contains("Precondition failed"),
            "unexpected error: {}",
            err
        );

        // v=5 satisfies the precondition
        runtime
            .execute(
                &account_id,
                "set",
                5u64.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        let result = runtime.view(&account_id, "get", vec![]).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 5);
    }

    #[test]
    fn test_postcondition_rejects_invalid_result() {
        // `post: result == old(x) + 1` — calling increment when x=5 should
        // produce 6, and anything else is rejected.
        let input = r#"
            contract Post {
                invariant x >= 0;
                state { x: u64 }
                @verified(post: result == old(x) + 1)
                fn increment() -> u64 { x = x + 1; x; }
                fn get() -> u64 { x; }
            }
        "#;

        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("post.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime.deploy_contract(account_id.clone(), input, deployer, 1000).unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        // Valid call: x=0 -> increment -> x=1, result=1 == old(0)+1
        let result = runtime
            .execute(&account_id, "increment", vec![], context.clone())
            .unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 1);

        // Now tamper with the contract body so the postcondition fails:
        // replace the body with `x = x + 2; x;` which violates
        // `result == old(x) + 1`.
        let mut tampered = runtime.get_contract(&account_id).unwrap().clone();
        // Recompile with a modified body that breaks the postcondition.
        let broken = r#"
            contract Post {
                invariant x >= 0;
                state { x: u64 }
                @verified(post: result == old(x) + 1)
                fn increment() -> u64 { x = x + 2; x; }
                fn get() -> u64 { x; }
            }
        "#;
        let contracts = redsigma_parser::Parser::parse(broken).unwrap();
        let mut pipeline = redsigma_codegen::CodegenPipeline::new();
        let compilation = pipeline.compile(&contracts[0]).unwrap();
        let mut new_deployed = runtime.get_contract(&account_id).unwrap().clone();
        new_deployed.wasm_bytecode = compilation.bytecode;
        new_deployed.proof = compilation.proof;
        new_deployed.contract = contracts[0].clone();
        runtime.contracts.insert(account_id.clone(), new_deployed);

        // x is currently 1. Calling increment should set x=3 but the
        // postcondition expects result == old(1) + 1 == 2, so it fails.
        let err = runtime
            .execute(&account_id, "increment", vec![], context.clone())
            .expect_err("postcondition result == old(x) + 1 should reject x+2");
        assert!(
            err.to_string().contains("Postcondition failed"),
            "unexpected error: {}",
            err
        );
    }
}
