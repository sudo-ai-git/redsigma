//! Deep-AST probes.
//!
//! The parser's source-level depth guard only counts *recursive* nesting. The
//! postfix index loop (`a[0][0]...`) and the binary-operator loop
//! (`1+1+1+...`) build arbitrarily deep ASTs iteratively, so a hostile file can
//! still produce a tree that overflows the stack in any recursive walk
//! (semantic projection, dual-stream hashing, codegen, reflection).

use redsigma_codegen::CodegenPipeline;
use redsigma_dualstream::DualStreamAnalyzer;
use redsigma_parser::Parser;
use redsigma_reflection::ReflectionEngine;
use redsigma_semantic::SemanticAnalyzer;

fn run_pipeline(src: &str) {
    let contracts = Parser::parse(src).expect("must parse");
    let mut sem = SemanticAnalyzer::new();
    let _ = sem.analyze(&contracts[0]);
    let mut ds = DualStreamAnalyzer::new();
    let _ = ds.analyze(&contracts[0]);
    let mut refl = ReflectionEngine::new();
    let _ = refl.reflect(&contracts[0]);
    let mut cg = CodegenPipeline::new();
    let _ = cg.compile(&contracts[0]);
}

#[test]
fn deep_index_chain_is_rejected_cleanly() {
    // 5000 nested indexes: built iteratively by the postfix loop, so the
    // source-level guard misses it.
    let src = format!(
        "contract Idx {{ state {{ a: Vec<u64> }} fn f() -> u64 {{ a{}; }} }}",
        "[0]".repeat(5000)
    );
    let err = Parser::parse(&src).unwrap_err();
    assert!(err.contains("too deeply nested"), "got: {}", err);
}

#[test]
fn deep_binary_chain_is_rejected_cleanly() {
    // 5000 terms joined by `+`: a left-leaning tree built in a loop.
    let src = format!(
        "contract Add {{ state {{ x: u64 }} fn f() -> u64 {{ 1{}; }} }}",
        "+1".repeat(4999)
    );
    let err = Parser::parse(&src).unwrap_err();
    assert!(err.contains("too deeply nested"), "got: {}", err);
}

#[test]
fn full_pipeline_survives_a_bounded_index_chain() {
    // A chain just under the limit must still parse AND survive every
    // recursive walk in the pipeline.
    let src = format!(
        "contract Idx {{ state {{ a: Vec<u64> }} fn f() -> u64 {{ {}; }} }}",
        "a[0]".repeat(50)
    );
    run_pipeline(&src);
}
