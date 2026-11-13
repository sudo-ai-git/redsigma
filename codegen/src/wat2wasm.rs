//! Assemble the WAT emitted by [`crate::wat`] and [`crate::near_abi`] into WASM bytecode.
//!
//! `wat::emit_wat` produces machine-generated WAT in a bounded subset:
//! ```text
//! (module
//!   (global $name (mut i64) (i64.const 0))
//!   (func $name (param $p i64)* (result i64)? (local $l i64)* <instr>*)
//!   (export "name" (func $f)))
//! ```
//!
//! `near_abi::emit_near_module` extends this with NEAR ABI constructs:
//! - `(import "env" "name" (func ...))` — NEAR host function imports
//! - `(memory (export "memory") 1)` — exported linear memory
//! - `(global $name (mut i32) ...)` — i32 globals (heap pointer, key pointers)
//! - i32 params and locals
//! - i32/i64 load/store instructions
//!
//! Instructions appear both **unfolded** (`(local.get $x) (i64.add)`) and
//! **folded** (`(local.set $x (i64.add ...))`, `(br_if 1 <cond>)`,
//! `(if (result i64) <cond> (then ...) (else ...))`). This module parses exactly
//! that subset and encodes it with `wasm-encoder`; anything else is an error
//! rather than a guess.
//!
//! Imported functions occupy indices `0..n_imports-1` in the WASM function
//! index space; defined functions start at `n_imports`. A `call` to an
//! imported function uses its import index; a `call` to a defined function
//! uses `n_imports + defined_index`.

use crate::CodegenError;
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, Export, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, ImportSection, Instruction, MemorySection, MemoryType, Module as WasmModule,
    TypeSection, ValType,
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
            ';' => {
                // `;;` comment — skip to end of line
                while let Some(&c) = chars.peek() {
                    if c == '\n' {
                        break;
                    }
                    chars.next();
                }
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
                                    "unterminated escape in WAT".to_string(),
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

fn as_i32(node: &Node) -> Result<i32, CodegenError> {
    match node {
        Node::Word(w) => {
            if let Ok(v) = w.parse::<i32>() {
                return Ok(v);
            }
            if let Ok(v) = w.parse::<u32>() {
                return Ok(v as i32);
            }
            Err(CodegenError::CodegenError(format!(
                "invalid i32 integer {w:?} in WAT"
            )))
        }
        _ => Err(CodegenError::CodegenError(
            "expected an i32 integer in WAT".to_string(),
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
    imports: ImportSection,
    types: TypeSection,
    funcs: FunctionSection,
    globals: GlobalSection,
    memories: MemorySection,
    exports: ExportSection,
    code: CodeSection,
    /// source name -> global index
    global_idx: HashMap<String, u32>,
    /// source name -> function index (in the *defined-function* space;
    /// imported functions occupy indices 0..num_imported_funcs-1)
    func_idx: HashMap<String, u32>,
    /// number of imported functions
    num_imported_funcs: u32,
    /// (params, results) encoded as valtype bytes -> type index
    type_idx: HashMap<(Vec<u8>, Vec<u8>), u32>,
    /// memory index counter
    memory_count: u32,
}

/// Assemble WAT produced by `wat::emit_wat` or `near_abi::emit_near_module`
/// into a WASM module.
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
        imports: ImportSection::new(),
        types: TypeSection::new(),
        funcs: FunctionSection::new(),
        globals: GlobalSection::new(),
        memories: MemorySection::new(),
        exports: ExportSection::new(),
        code: CodeSection::new(),
        global_idx: HashMap::new(),
        func_idx: HashMap::new(),
        num_imported_funcs: 0,
        type_idx: HashMap::new(),
        memory_count: 0,
    };

    // ---- Pass 1: index imports, globals, and functions ----
    // Imported functions occupy indices 0..num_imported_funcs-1 in the
    // WASM function index space. Defined functions start after them.
    for item in &items[1..] {
        if let Node::List(l) = item {
            match head_word(l) {
                Some("import") => {
                    // (import "env" "name" (func $name (param ...) (result ...)))
                    if let Some(Node::List(ft)) = l.get(3) {
                        if head_word(ft) == Some("func") {
                            let name = dollar_name(ft.get(1).ok_or_else(|| {
                                CodegenError::CodegenError("import without a name".to_string())
                            })?)?;
                            enc.func_idx.insert(name.to_string(), enc.num_imported_funcs);
                            enc.num_imported_funcs += 1;
                        }
                    }
                }
                Some("global") => {
                    let name = global_name(l)?.to_string();
                    enc.global_idx
                        .insert(name.clone(), enc.global_idx.len() as u32);
                }
                Some("func") => {
                    let name = func_name(l)?.to_string();
                    // Defined function index = num_imported_funcs + position in defined list
                    let defined_idx = enc.func_idx.len() as u32 - enc.num_imported_funcs;
                    enc.func_idx.insert(name, enc.num_imported_funcs + defined_idx);
                }
                _ => {}
            }
        }
    }

    // ---- Pass 2: emit import, type, function, global, memory, export sections ----
    for item in &items[1..] {
        let l = match item {
            Node::List(l) => l,
            _ => continue,
        };
        match head_word(l) {
            Some("import") => {
                // (import "env" "name" (func $name (param ...) (result ...)))
                let import_module = match l.get(1) {
                    Some(Node::Str(s)) => s.clone(),
                    _ => {
                        return Err(CodegenError::CodegenError(
                            "import without a module name".to_string(),
                        ))
                    }
                };
                let import_name = match l.get(2) {
                    Some(Node::Str(s)) => s.clone(),
                    _ => {
                        return Err(CodegenError::CodegenError(
                            "import without a field name".to_string(),
                        ))
                    }
                };
                let target = match l.get(3) {
                    Some(Node::List(t)) => t,
                    _ => {
                        return Err(CodegenError::CodegenError(
                            "import without a target".to_string(),
                        ))
                    }
                };
                match head_word(target) {
                    Some("func") => {
                        let _name = dollar_name(target.get(1).ok_or_else(|| {
                            CodegenError::CodegenError("func import without a name".to_string())
                        })?)?;
                        let (params, results) = import_signature(target)?;
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
                        enc.imports.import(
                            &import_module,
                            &import_name,
                            wasm_encoder::ImportType::Function(ty),
                        );
                    }
                    other => {
                        return Err(CodegenError::CodegenError(format!(
                            "unsupported import target {other:?} (only (func $f) is imported)"
                        )));
                    }
                }
            }
            Some("memory") => {
                // (memory (export "memory") 1)
                let limits = wasm_encoder::Limits { min: 1, max: None };
                let mem_type = MemoryType { limits };
                enc.memories.memory(mem_type);
                enc.memory_count += 1;
                // The export annotation is a CHILD of the memory definition
                // (`(memory (export "memory") 1)`), not a sibling of it, so a
                // top-level-only scan never sees it. Emit it here.
                for node in &l[1..] {
                    if let Node::List(inner) = node {
                        if head_word(inner) == Some("export") {
                            emit_export(
                                &mut enc,
                                &augment_export_target(inner, "memory".to_string(), None),
                            )?;
                        }
                    }
                }
            }
            Some("global") => {
                // (global $name (mut i64) (i64.const 0)) or (global $name (mut i32) (i32.const 0))
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
                    Some(Node::List(init)) if head_word(init) == Some("i32.const") => {
                        Instruction::I32Const(as_i32(init.get(1).ok_or_else(|| {
                            CodegenError::CodegenError("i32.const without a value".to_string())
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
                // `(func $f (export "f") ...)` — the export annotation is a
                // child of the func definition, so emit it from here.
                let fname = func_name(l)?.to_string();
                for node in &l[1..] {
                    if let Node::List(inner) = node {
                        if head_word(inner) == Some("export") {
                            emit_export(
                                &mut enc,
                                &augment_export_target(inner, "func".to_string(), Some(&fname)),
                            )?;
                        }
                    }
                }
            }
            Some("export") => emit_export(&mut enc, l)?,
            _ => {}
        }
    }

    // ---- Pass 3: code section (function bodies) ----
    for item in &items[1..] {
        let l = match item {
            Node::List(l) => l,
            _ => continue,
        };
        if head_word(l) != Some("func") {
            continue;
        }
        // Skip imported functions (they have no body)
        if l.get(1).and_then(word) == Some("import") {
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

        // Local declarations are grouped by type in the binary format. WASM
        // assigns indices to ALL locals (params first, then declared locals)
        // in *declaration order*, which is NOT the same as the grouped order
        // used by `(local ...)` groups. Assign indices from the grouped
        // declaration order so `local.get $x` resolves to the right slot,
        // otherwise an i32 local reads an i64 slot (or vice versa) and the
        // validator reports a type mismatch.
        let mut i32_locals: Vec<String> = Vec::new();
        let mut i64_locals: Vec<String> = Vec::new();
        for n in &local_names {
            // The NEAR ABI layer declares its scratch locals as i32; the
            // WAT backend declares body locals as i64.
            let ty = local_type_at(l, n).unwrap_or(ValType::I64);
            match ty {
                ValType::I32 => i32_locals.push(n.clone()),
                _ => i64_locals.push(n.clone()),
            }
        }
        let mut local_decls: Vec<(u32, ValType)> = Vec::new();
        let mut next_idx = params.len() as u32;
        if !i32_locals.is_empty() {
            local_decls.push((i32_locals.len() as u32, ValType::I32));
            for n in &i32_locals {
                local_idx.insert(n.clone(), next_idx);
                next_idx += 1;
            }
        }
        if !i64_locals.is_empty() {
            local_decls.push((i64_locals.len() as u32, ValType::I64));
            for n in &i64_locals {
                local_idx.insert(n.clone(), next_idx);
                next_idx += 1;
            }
        }
        let mut f = Function::new(local_decls);
        for instr in body {
            encode_instr(instr, &mut f, &local_idx, &enc.global_idx, &enc.func_idx, enc.num_imported_funcs)?;
        }
        // Every function body must terminate with the `end` opcode.
        f.instruction(Instruction::End);
        let _ = &results;
        enc.code.function(&f);
    }

    // Section order is fixed by the WASM spec (Section 6.2):
    //   type(1) -> import(2) -> func(3) -> table(4) -> memory(5) -> global(6)
    //   -> export(7) -> start(8) -> elem(9) -> code(10) -> data(11)
    // The import section references type indices, so it must follow the type
    // section; a module with sections out of this order is rejected by every
    // validator (wasm-encoder's `Module` does not reorder them for us).
    let mut module = WasmModule::new();
    module
        .section(&enc.types)
        .section(&enc.imports)
        .section(&enc.funcs)
        .section(&enc.memories)
        .section(&enc.globals)
        .section(&enc.exports)
        .section(&enc.code);
    Ok(module.finish())
}

/// Determine the ValType of a local declaration `(local $name i32)` or `(local $name i64)`.
fn local_type_at(l: &[Node], name: &str) -> Result<ValType, CodegenError> {
    for node in l {
        if let Node::List(item) = node {
            if head_word(item) == Some("local") {
                let n = dollar_name(item.get(1).ok_or_else(|| {
                    CodegenError::CodegenError("local without a name".to_string())
                })?)?;
                if n == name {
                    return val_type_at(item, 2);
                }
            }
        }
    }
    Ok(ValType::I64)
}

/// `(import ... (func $name (param $p i32)* (result i32)?))` -> (params, results).
/// Handles both named `(param $name i32)` and anonymous `(param i32)` forms.
fn import_signature(l: &[Node]) -> Result<(Vec<ValType>, Vec<ValType>), CodegenError> {
    let mut params = Vec::new();
    let mut results = Vec::new();
    for item in l.iter().skip(1) {
        if let Node::List(item) = item {
            match head_word(item) {
                Some("param") => {
                    // Anonymous param: (param i32) — type at index 1.
                    // Named param: (param $name i32) — type at index 2.
                    if item.len() >= 3 {
                        params.push(val_type_at(item, 2)?);
                    } else {
                        params.push(val_type_at(item, 1)?);
                    }
                }
                Some("result") => results.push(val_type_at(item, 1)?),
                _ => {}
            }
        }
    }
    Ok((params, results))
}

/// `(func $name (param $p i64)* (result i64)? ...)` -> (params, results).
/// Handles both named `(param $name i64)` and anonymous `(param i64)` forms.
fn func_signature(l: &[Node]) -> Result<(Vec<ValType>, Vec<ValType>), CodegenError> {
    let mut params = Vec::new();
    let mut results = Vec::new();
    for item in &l[1..] {
        if let Node::List(item) = item {
            match head_word(item) {
                Some("param") => {
                    if item.len() >= 3 {
                        params.push(val_type_at(item, 2)?);
                    } else {
                        params.push(val_type_at(item, 1)?);
                    }
                }
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

fn is_annotation(item: &[Node]) -> bool {
    matches!(
        head_word(item),
        Some("param") | Some("result") | Some("local") | Some("export")
    )
}

fn global_name(l: &[Node]) -> Result<&str, CodegenError> {
    let n = l
        .get(1)
        .ok_or_else(|| CodegenError::CodegenError("global without a name".to_string()))?;
    dollar_name(n)
}

/// Rebuild an inline export node with its implied target made explicit.
///
/// `(func $increment (export "increment"))` in WAT text is shorthand for
/// `(export "increment" (func $increment))`; the nested node parses as just
/// `(export "increment")` with the target missing. `kind`/`name` supply it.
fn augment_export_target(
    inner: &[Node],
    kind: String,
    name: Option<&str>,
) -> Vec<Node> {
    let mut out: Vec<Node> = vec![Node::Word("export".to_string())];
    // preserve the export name (first element after the head word)
    if let Some(Node::Str(s)) = inner.get(1) {
        out.push(Node::Str(s.clone()));
    }
    let mut target: Vec<Node> = vec![Node::Word(kind)];
    match name {
        Some(n) => target.push(Node::Word(format!("${n}"))),
        None => target.push(Node::Word("0".to_string())),
    }
    out.push(Node::List(target));
    out
}

/// Emit one `(export "name" (func $f))` / `(export "name" (memory 0))` node.
///
/// Exports appear in two shapes in the emitted WAT: as a top-level module
/// entry, and inline inside a `(func ...)` or `(memory ...)` definition. Both
/// forms are the same node shape, so both call this helper.
fn emit_export(enc: &mut Encoder, l: &[Node]) -> Result<(), CodegenError> {
    let export_name = match l.get(1) {
        Some(Node::Str(s)) => s.clone(),
        _ => {
            return Err(CodegenError::CodegenError(
                "export without a name".to_string(),
            ))
        }
    };
    // `(export "f")` nested inside a func/memory omits the explicit target —
    // the enclosing definition is implied. Callers that hit this form pass
    // the enclosing name in `l[2]` as a word so this branch is unreachable;
    // see the two call sites below.
    let target: Node = match l.get(2) {
        Some(Node::List(t)) => Node::List(t.clone()),
        Some(Node::Word(w)) => Node::List(vec![
            Node::Word("func".to_string()),
            Node::Word(format!("${w}")),
        ]),
        _ => {
            return Err(CodegenError::CodegenError(
                "export without a target".to_string(),
            ))
        }
    };
    let target: Vec<Node> = match target {
        Node::List(v) => v,
        other => vec![other],
    };
    match head_word(&target) {
        Some("func") => {
            let name = dollar_name(target.get(1).ok_or_else(|| {
                CodegenError::CodegenError("func export without a name".to_string())
            })?)?;
            let idx = *enc.func_idx.get(name).ok_or_else(|| {
                CodegenError::CodegenError(format!("export of unknown func {name}"))
            })?;
            enc.exports.export(&export_name, Export::Function(idx));
        }
        Some("memory") => {
            enc.exports.export(&export_name, Export::Memory(0));
        }
        other => {
            return Err(CodegenError::CodegenError(format!(
                "unsupported export target {other:?} (only (func $f) and (memory) are exported)"
            )));
        }
    }
    Ok(())
}

fn func_name(l: &[Node]) -> Result<&str, CodegenError> {
    let n = l
        .get(1)
        .ok_or_else(|| CodegenError::CodegenError("func without a name".to_string()))?;
    dollar_name(n)
}

fn val_type_at(l: &[Node], idx: usize) -> Result<ValType, CodegenError> {
    match l.get(idx).and_then(word) {
        Some("i64") => Ok(ValType::I64),
        Some("i32") => Ok(ValType::I32),
        Some("i16") | Some("i8") => Ok(ValType::I64),
        _ => Err(CodegenError::CodegenError(format!(
            "unsupported value type in WAT: {:?}",
            l.get(idx)
        ))),
    }
}

fn encode_instr(
    node: &Node,
    f: &mut Function,
    locals: &HashMap<String, u32>,
    globals: &HashMap<String, u32>,
    funcs: &HashMap<String, u32>,
    num_imported_funcs: u32,
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
            let idx = local(&args[0])?;
            for a in args.iter().skip(1) {
                if matches!(a, Node::List(_)) {
                    encode_instr(a, f, locals, globals, funcs, num_imported_funcs)?;
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
                    encode_instr(a, f, locals, globals, funcs, num_imported_funcs)?;
                }
            }
            f.instruction(Instruction::GlobalSet(idx));
            return Ok(());
        }
        "br_if" => {
            let n = as_i64(&args[0])? as u32;
            if let Some(Node::List(_)) = args.get(1) {
                encode_instr(&args[1], f, locals, globals, funcs, num_imported_funcs)?;
            }
            f.instruction(Instruction::BrIf(n));
            return Ok(());
        }
        "if" => {
            let bt = block_type(args)?;
            let mut start = 0usize;
            if let Some(Node::List(first)) = args.first() {
                if head_word(first) == Some("result") {
                    start = 1;
                }
            }
            let mut body_start = start;
            if let Some(Node::List(cond)) = args.get(start) {
                let h = head_word(cond);
                if h != Some("then") && h != Some("else") {
                    encode_instr(&args[start], f, locals, globals, funcs, num_imported_funcs)?;
                    body_start = start + 1;
                }
            }
            f.instruction(Instruction::If(bt));
            for a in &args[body_start..] {
                if let Node::List(body) = a {
                    match head_word(body) {
                        Some("then") => {
                            for instr in &body[1..] {
                                encode_instr(instr, f, locals, globals, funcs, num_imported_funcs)?;
                            }
                        }
                        Some("else") => {
                            f.instruction(Instruction::Else);
                            for instr in &body[1..] {
                                encode_instr(instr, f, locals, globals, funcs, num_imported_funcs)?;
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
                encode_instr(instr, f, locals, globals, funcs, num_imported_funcs)?;
            }
            f.instruction(Instruction::End);
            return Ok(());
        }
        _ => {}
    }

    // Everything else is a flat instruction with immediate arguments.
    // Its operands may still be folded; emit them BEFORE the operator.
    for a in args {
        if matches!(a, Node::List(_)) {
            encode_instr(a, f, locals, globals, funcs, num_imported_funcs)?;
        }
    }

    let memarg = wasm_encoder::MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    };
    let instr = match head {
        "i64.const" => Instruction::I64Const(as_i64(&args[0])?),
        "i32.const" => Instruction::I32Const(as_i32(&args[0])?),
        "local.get" => Instruction::LocalGet(local(&args[0])?),
        "global.get" => Instruction::GlobalGet(global(&args[0])?),
        "call" => {
            // Imported functions occupy indices 0..num_imported_funcs-1;
            // defined functions start at num_imported_funcs.
            let idx = func(&args[0])?;
            Instruction::Call(idx)
        }
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
        "i64.lt_u" => Instruction::I64LtU,
        "i64.gt_s" => Instruction::I64GtS,
        "i64.le_s" => Instruction::I64LeS,
        "i64.ge_s" => Instruction::I64GeS,
        "i64.eqz" => Instruction::I64Eqz,
        "i32.add" => Instruction::I32Add,
        "i32.eqz" => Instruction::I32Eqz,
        "i32.load" => Instruction::I32Load(memarg),
        "i32.load8_u" => Instruction::I32Load8_U(memarg),
        "i32.load16_u" => Instruction::I32Load16_U(memarg),
        "i64.load" => Instruction::I64Load(memarg),
        "i32.store" => Instruction::I32Store(memarg),
        "i32.store8" => Instruction::I32Store8(memarg),
        "i32.store16" => Instruction::I32Store16(memarg),
        "i64.store" => Instruction::I64Store(memarg),
        "i64.extend_i32_u" => Instruction::I64ExtendI32U,
        // The NEAR ABI write path truncates an i64 state global to i32 before a
        // narrow store, and the read path widens back afterwards. Without this
        // entry the emitter's own output was rejected by its own assembler.
        "i32.wrap_i64" => Instruction::I32WrapI64,
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
