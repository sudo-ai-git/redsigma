//! Assemble the WAT emitted by [`crate::wat`] into WASM bytecode.
//!
//! `wat::emit_wat` produces machine-generated WAT in a bounded subset:
//!
//! ```text
//! (module
//!   (global $name (mut i64) (i64.const 0))
//!   (func $name (param $p i64)* (result i64)? (local $l i64)* <instr>*)
//!   (export "name" (func $f)))
//! ```
//!
//! Instructions appear both **unfolded** (`(local.get $x) (i64.add)`) and
//! **folded** (`(local.set $x (i64.add ...))`, `(br_if 1 <cond>)`,
//! `(if (result i64) <cond> (then ...) (else ...))`). This module parses exactly
//! that subset and encodes it with `wasm-encoder`; anything else is an error
//! rather than a guess.
//!
//! It is deliberately not a general WAT implementation. `compile()` no longer
//! emits a placeholder: it lowers to WAT and assembles it to a real module.

use crate::CodegenError;
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, Export, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, Instruction, Module as WasmModule, TypeSection, ValType,
};

// --------------------------------------------------------------------------
// tokenizer
// --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    /// `$name`, `i64.add`, `0`, `-1`, ...
    Word(String),
    /// contents of a `"..."` literal (quotes stripped)
    Str(String),
}

fn tokenize(input: &str) -> Result<Vec<Token>, CodegenError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                tokens.push(Token::LParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RParen);
                chars.next();
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => {
                            return Err(CodegenError::CodegenError(
                                "unterminated string literal in WAT".to_string(),
                            ))
                        }
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            Some(other) => {
                                s.push('\\');
                                s.push(other);
                            }
                            None => {
                                return Err(CodegenError::CodegenError(
                                    "unterminated escape in WAT string".to_string(),
                                ))
                            }
                        },
                        Some(other) => s.push(other),
                    }
                }
                tokens.push(Token::Str(s));
            }
            _ => {
                let mut w = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' || c == '"' {
                        break;
                    }
                    w.push(c);
                    chars.next();
                }
                tokens.push(Token::Word(w));
            }
        }
    }
    Ok(tokens)
}

// --------------------------------------------------------------------------
// s-expression parser
// --------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Word(String),
    Str(String),
    List(Vec<Node>),
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn parse_node(&mut self) -> Result<Node, CodegenError> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token::LParen) => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    match self.tokens.get(self.pos) {
                        Some(Token::RParen) => {
                            self.pos += 1;
                            break;
                        }
                        None => {
                            return Err(CodegenError::CodegenError(
                                "unterminated list in WAT".to_string(),
                            ))
                        }
                        Some(_) => items.push(self.parse_node()?),
                    }
                }
                Ok(Node::List(items))
            }
            Some(Token::Word(w)) => {
                self.pos += 1;
                Ok(Node::Word(w))
            }
            Some(Token::Str(s)) => {
                self.pos += 1;
                Ok(Node::Str(s))
            }
            Some(Token::RParen) => Err(CodegenError::CodegenError(
                "unexpected ')' in WAT".to_string(),
            )),
            None => Err(CodegenError::CodegenError(
                "unexpected end of WAT input".to_string(),
            )),
        }
    }
}

fn word(node: &Node) -> Option<&str> {
    match node {
        Node::Word(w) => Some(w),
        _ => None,
    }
}

fn head_word(list: &[Node]) -> Option<&str> {
    list.first().and_then(word)
}

fn as_i64(node: &Node) -> Result<i64, CodegenError> {
    match node {
        Node::Word(w) => {
            // The emitter writes the source literal verbatim, so a u64 like
            // 18446744073709551615 does not fit in i64; interpret the bits as
            // two's complement instead of rejecting the contract.
            if let Ok(v) = w.parse::<i64>() {
                return Ok(v);
            }
            if let Ok(v) = w.parse::<u64>() {
                return Ok(v as i64);
            }
            Err(CodegenError::CodegenError(format!(
                "invalid integer {w:?} in WAT"
            )))
        }
        _ => Err(CodegenError::CodegenError(
            "expected an integer in WAT".to_string(),
        )),
    }
}

/// `$name` → `name`
fn dollar_name(node: &Node) -> Result<&str, CodegenError> {
    match node {
        Node::Word(w) => w.strip_prefix('$').ok_or_else(|| {
            CodegenError::CodegenError(format!("expected $name in WAT, found {w:?}"))
        }),
        _ => Err(CodegenError::CodegenError(
            "expected $name in WAT".to_string(),
        )),
    }
}

// --------------------------------------------------------------------------
// encoder
// --------------------------------------------------------------------------

struct Encoder {
    types: TypeSection,
    funcs: FunctionSection,
    globals: GlobalSection,
    exports: ExportSection,
    code: CodeSection,
    /// source name -> global index
    global_idx: HashMap<String, u32>,
    /// source name -> function index
    func_idx: HashMap<String, u32>,
    /// (params, results) encoded as valtype bytes -> type index
    type_idx: HashMap<(Vec<u8>, Vec<u8>), u32>,
}

/// Assemble WAT produced by `wat::emit_wat` into a WASM module.
pub fn assemble(wat: &str) -> Result<Vec<u8>, CodegenError> {
    let tokens = tokenize(wat)?;
    let mut parser = Parser { tokens, pos: 0 };
    let module = parser.parse_node()?;
    if !parser.at_end() {
        return Err(CodegenError::CodegenError(
            "trailing tokens after the WAT module".to_string(),
        ));
    }
    encode_module(&module)
}

fn encode_module(module: &Node) -> Result<Vec<u8>, CodegenError> {
    let items = match module {
        Node::List(l) => l,
        _ => {
            return Err(CodegenError::CodegenError(
                "WAT input is not a list".to_string(),
            ))
        }
    };
    if head_word(items) != Some("module") {
        return Err(CodegenError::CodegenError(
            "WAT input does not start with (module ...)".to_string(),
        ));
    }

    let mut enc = Encoder {
        types: TypeSection::new(),
        funcs: FunctionSection::new(),
        globals: GlobalSection::new(),
        exports: ExportSection::new(),
        code: CodeSection::new(),
        global_idx: HashMap::new(),
        func_idx: HashMap::new(),
        type_idx: HashMap::new(),
    };

    // Index globals and functions first: instructions reference them by index.
    let mut global_names: Vec<String> = Vec::new();
    let mut func_names: Vec<String> = Vec::new();
    for item in &items[1..] {
        if let Node::List(l) = item {
            match head_word(l) {
                Some("global") => {
                    let name = global_name(l)?.to_string();
                    enc.global_idx
                        .insert(name.clone(), global_names.len() as u32);
                    global_names.push(name);
                }
                Some("func") => {
                    let name = func_name(l)?.to_string();
                    enc.func_idx.insert(name.clone(), func_names.len() as u32);
                    func_names.push(name);
                }
                _ => {}
            }
        }
    }

    // Types, function-section entries, globals, exports.
    for item in &items[1..] {
        let l = match item {
            Node::List(l) => l,
            _ => continue,
        };
        match head_word(l) {
            Some("global") => {
                // (global $name (mut i64) (i64.const 0))
                let mutable = match l.get(2) {
                    Some(Node::List(t)) => head_word(t) == Some("mut"),
                    _ => false,
                };
                let val_type = match l.get(2) {
                    Some(Node::List(t)) => val_type_at(t, 1)?,
                    _ => ValType::I64,
                };
                let init = match l.get(3) {
                    Some(Node::List(init)) if head_word(init) == Some("i64.const") => {
                        Instruction::I64Const(as_i64(init.get(1).ok_or_else(|| {
                            CodegenError::CodegenError("i64.const without a value".to_string())
                        })?)?)
                    }
                    _ => Instruction::I64Const(0),
                };
                enc.globals.global(GlobalType { val_type, mutable }, init);
            }
            Some("func") => {
                let (params, results) = func_signature(l)?;
                let key: (Vec<u8>, Vec<u8>) = (
                    params.iter().map(|t| u8::from(*t)).collect(),
                    results.iter().map(|t| u8::from(*t)).collect(),
                );
                let ty = match enc.type_idx.get(&key) {
                    Some(t) => *t,
                    None => {
                        enc.types
                            .function(params.iter().copied(), results.iter().copied());
                        let idx = enc.type_idx.len() as u32;
                        enc.type_idx.insert(key, idx);
                        idx
                    }
                };
                enc.funcs.function(ty);
            }
            Some("export") => {
                // (export "name" (func $f))
                let export_name = match l.get(1) {
                    Some(Node::Str(s)) => s.clone(),
                    _ => {
                        return Err(CodegenError::CodegenError(
                            "export without a name".to_string(),
                        ))
                    }
                };
                let target = match l.get(2) {
                    Some(Node::List(t)) => t,
                    _ => {
                        return Err(CodegenError::CodegenError(
                            "export without a target".to_string(),
                        ))
                    }
                };
                match head_word(target) {
                    Some("func") => {
                        let name = dollar_name(target.get(1).ok_or_else(|| {
                            CodegenError::CodegenError("func export without a name".to_string())
                        })?)?;
                        let idx = *enc.func_idx.get(name).ok_or_else(|| {
                            CodegenError::CodegenError(format!("export of unknown func {name}"))
                        })?;
                        enc.exports.export(&export_name, Export::Function(idx));
                    }
                    other => {
                        return Err(CodegenError::CodegenError(format!(
                            "unsupported export target {other:?} (only (func $f) is emitted)"
                        )))
                    }
                }
            }
            _ => {}
        }
    }

    // Code section: bodies reference locals/globals/funcs by index.
    for item in &items[1..] {
        let l = match item {
            Node::List(l) => l,
            _ => continue,
        };
        if head_word(l) != Some("func") {
            continue;
        }
        let (params, results) = func_signature(l)?;
        // Locals declared inside the function, after params/result.
        let mut local_names: Vec<String> = Vec::new();
        let mut body: &[Node] = &[];
        for (i, node) in l.iter().enumerate().skip(1) {
            if let Node::List(item) = node {
                match head_word(item) {
                    Some("local") => {
                        let n = dollar_name(item.get(1).ok_or_else(|| {
                            CodegenError::CodegenError("local without a name".to_string())
                        })?)?;
                        local_names.push(n.to_string());
                    }
                    _ => {
                        // everything that is not a signature/local annotation is
                        // the body; the emitter writes it last.
                        if i > 1 && !is_annotation(item) {
                            body = &l[i..];
                            break;
                        }
                    }
                }
            }
        }

        let mut local_idx: HashMap<String, u32> = HashMap::new();
        for (i, p) in params.iter().enumerate() {
            // params are unnamed in the emitter; index by position.
            local_idx.insert(format!("__param{i}"), i as u32);
        }
        // Recover parameter names from the (param $p i64) annotations.
        let mut param_pos = 0usize;
        for node in &l[1..] {
            if let Node::List(item) = node {
                if head_word(item) == Some("param") {
                    let n = dollar_name(item.get(1).ok_or_else(|| {
                        CodegenError::CodegenError("param without a name".to_string())
                    })?)?;
                    local_idx.insert(n.to_string(), param_pos as u32);
                    param_pos += 1;
                }
            }
        }
        for (i, n) in local_names.iter().enumerate() {
            local_idx.insert(n.clone(), (params.len() + i) as u32);
        }

        // One declaration per distinct type keeps the section small; the emitter
        // only uses i64 locals.
        let mut local_decls: Vec<(u32, ValType)> = Vec::new();
        local_decls.push((local_names.len() as u32, ValType::I64));
        let mut f = Function::new(local_decls);
        for instr in body {
            encode_instr(instr, &mut f, &local_idx, &enc.global_idx, &enc.func_idx)?;
        }
        // Every function body must terminate with the `end` opcode.
        f.instruction(Instruction::End);
        let _ = &results;
        enc.code.function(&f);
    }

    let mut module = WasmModule::new();
    module
        .section(&enc.types)
        .section(&enc.funcs)
        .section(&enc.globals)
        .section(&enc.exports)
        .section(&enc.code);
    Ok(module.finish())
}

fn is_annotation(item: &[Node]) -> bool {
    matches!(
        head_word(item),
        Some("param") | Some("result") | Some("local")
    )
}

fn global_name(l: &[Node]) -> Result<&str, CodegenError> {
    let n = l
        .get(1)
        .ok_or_else(|| CodegenError::CodegenError("global without a name".to_string()))?;
    dollar_name(n)
}

fn func_name(l: &[Node]) -> Result<&str, CodegenError> {
    let n = l
        .get(1)
        .ok_or_else(|| CodegenError::CodegenError("func without a name".to_string()))?;
    dollar_name(n)
}

fn val_type_at(l: &[Node], idx: usize) -> Result<ValType, CodegenError> {
    match l.get(idx).and_then(word) {
        Some("i64") | Some("i32") | Some("i16") | Some("i8") => Ok(ValType::I64),
        _ => Err(CodegenError::CodegenError(format!(
            "unsupported value type in WAT: {:?}",
            l.get(idx)
        ))),
    }
}

/// `(func $name (param $p i64)* (result i64)? ...) -> (params, results)`
fn func_signature(l: &[Node]) -> Result<(Vec<ValType>, Vec<ValType>), CodegenError> {
    let mut params = Vec::new();
    let mut results = Vec::new();
    for item in &l[1..] {
        if let Node::List(item) = item {
            match head_word(item) {
                Some("param") => params.push(val_type_at(item, 2)?),
                Some("result") => results.push(val_type_at(item, 1)?),
                _ => {}
            }
        }
    }
    Ok((params, results))
}

fn block_type(l: &[Node]) -> Result<BlockType, CodegenError> {
    // `(if (result i64) ...)` / `(block)` / `(loop)`
    for item in l {
        if let Node::List(item) = item {
            if head_word(item) == Some("result") {
                return Ok(BlockType::Result(val_type_at(item, 1)?));
            }
        }
    }
    Ok(BlockType::Empty)
}

fn encode_instr(
    node: &Node,
    f: &mut Function,
    locals: &HashMap<String, u32>,
    globals: &HashMap<String, u32>,
    funcs: &HashMap<String, u32>,
) -> Result<(), CodegenError> {
    let list = match node {
        Node::List(l) => l,
        _ => {
            return Err(CodegenError::CodegenError(
                "WAT instruction is not a list".to_string(),
            ))
        }
    };
    let head = match head_word(list) {
        Some(h) => h,
        None => {
            return Err(CodegenError::CodegenError(
                "empty WAT instruction".to_string(),
            ))
        }
    };
    let args = &list[1..];

    let local = |name: &Node| -> Result<u32, CodegenError> {
        let n = dollar_name(name)?;
        locals
            .get(n)
            .copied()
            .ok_or_else(|| CodegenError::CodegenError(format!("unknown local ${n} in WAT")))
    };
    let global = |name: &Node| -> Result<u32, CodegenError> {
        let n = dollar_name(name)?;
        globals
            .get(n)
            .copied()
            .ok_or_else(|| CodegenError::CodegenError(format!("unknown global ${n} in WAT")))
    };
    let func = |name: &Node| -> Result<u32, CodegenError> {
        let n = dollar_name(name)?;
        funcs
            .get(n)
            .copied()
            .ok_or_else(|| CodegenError::CodegenError(format!("unknown func ${n} in WAT")))
    };

    // Instructions whose operand may be folded into the node.
    match head {
        "local.set" | "local.tee" => {
            // `(local.set $x)` (value already on the stack) or
            // `(local.set $x <folded value>)`.
            let idx = local(&args[0])?;
            for a in args.iter().skip(1) {
                if matches!(a, Node::List(_)) {
                    encode_instr(a, f, locals, globals, funcs)?;
                }
            }
            f.instruction(if head == "local.set" {
                Instruction::LocalSet(idx)
            } else {
                Instruction::LocalTee(idx)
            });
            return Ok(());
        }
        "global.set" => {
            let idx = global(&args[0])?;
            for a in args.iter().skip(1) {
                if matches!(a, Node::List(_)) {
                    encode_instr(a, f, locals, globals, funcs)?;
                }
            }
            f.instruction(Instruction::GlobalSet(idx));
            return Ok(());
        }
        "br_if" => {
            // `(br_if N)` or `(br_if N <cond>)`
            let n = as_i64(&args[0])? as u32;
            if let Some(Node::List(_)) = args.get(1) {
                encode_instr(&args[1], f, locals, globals, funcs)?;
            }
            f.instruction(Instruction::BrIf(n));
            return Ok(());
        }
        "if" => {
            // `(if (then ...) (else ...))` — condition already on the stack, or
            // `(if (result i64) <cond> (then ...) (else ...))` — folded.
            let bt = block_type(args)?;
            let mut start = 0usize;
            if let Some(Node::List(first)) = args.first() {
                if head_word(first) == Some("result") {
                    start = 1;
                }
            }
            // A folded condition (`(if (result i64) <cond> (then ...) ...)`) is
            // an operand, not a body clause; consume it and move the body start.
            let mut body_start = start;
            if let Some(Node::List(cond)) = args.get(start) {
                let h = head_word(cond);
                if h != Some("then") && h != Some("else") {
                    encode_instr(&args[start], f, locals, globals, funcs)?;
                    body_start = start + 1;
                }
            }
            f.instruction(Instruction::If(bt));
            for a in &args[body_start..] {
                if let Node::List(body) = a {
                    match head_word(body) {
                        Some("then") => {
                            for instr in &body[1..] {
                                encode_instr(instr, f, locals, globals, funcs)?;
                            }
                        }
                        Some("else") => {
                            f.instruction(Instruction::Else);
                            for instr in &body[1..] {
                                encode_instr(instr, f, locals, globals, funcs)?;
                            }
                        }
                        _ => {
                            return Err(CodegenError::CodegenError(
                                "if without a (then ...) body".to_string(),
                            ))
                        }
                    }
                }
            }
            f.instruction(Instruction::End);
            return Ok(());
        }
        "block" | "loop" => {
            f.instruction(if head == "block" {
                Instruction::Block(BlockType::Empty)
            } else {
                Instruction::Loop(BlockType::Empty)
            });
            for instr in args {
                encode_instr(instr, f, locals, globals, funcs)?;
            }
            f.instruction(Instruction::End);
            return Ok(());
        }
        _ => {}
    }

    // Everything else is a flat instruction with immediate arguments.
    //
    // Its operands may still be folded (`(i64.lt_s (local.get $i) (i64.const 3))`),
    // and in that case they are emitted BEFORE the operator.
    for a in args {
        if matches!(a, Node::List(_)) {
            encode_instr(a, f, locals, globals, funcs)?;
        }
    }

    let instr = match head {
        "i64.const" => Instruction::I64Const(as_i64(&args[0])?),
        "local.get" => Instruction::LocalGet(local(&args[0])?),
        "global.get" => Instruction::GlobalGet(global(&args[0])?),
        "call" => Instruction::Call(func(&args[0])?),
        "br" => Instruction::Br(as_i64(&args[0])? as u32),
        "i64.add" => Instruction::I64Add,
        "i64.sub" => Instruction::I64Sub,
        "i64.mul" => Instruction::I64Mul,
        "i64.div_s" => Instruction::I64DivS,
        "i64.rem_s" => Instruction::I64RemS,
        "i64.and" => Instruction::I64And,
        "i64.or" => Instruction::I64Or,
        "i64.xor" => Instruction::I64Xor,
        "i64.shl" => Instruction::I64Shl,
        "i64.shr_s" => Instruction::I64ShrS,
        "i64.eq" => Instruction::I64Eq,
        "i64.ne" => Instruction::I64Neq,
        "i64.lt_s" => Instruction::I64LtS,
        "i64.gt_s" => Instruction::I64GtS,
        "i64.le_s" => Instruction::I64LeS,
        "i64.ge_s" => Instruction::I64GeS,
        "i64.eqz" => Instruction::I64Eqz,
        "i32.eqz" => Instruction::I32Eqz,
        "drop" => Instruction::Drop,
        "return" => Instruction::Return,
        "end" => Instruction::End,
        other => {
            return Err(CodegenError::CodegenError(format!(
                "unsupported WAT instruction {other:?} (the emitter should not produce it)"
            )))
        }
    };
    f.instruction(instr);
    Ok(())
}
