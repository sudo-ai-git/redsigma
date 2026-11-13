//! Hostile-input probe for the parser.
//!
//! Every input must produce either a clean `Ok` or a clean `Err` — never a
//! panic, an abort, or a hang. The probe prints a classification table so
//! failures can be triaged, then asserts no panics occurred.

use redsigma_parser::Parser;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn classify(input: &str) -> String {
    match catch_unwind(AssertUnwindSafe(|| Parser::parse(input))) {
        Ok(Ok(contracts)) => format!("OK ({} contract(s))", contracts.len()),
        Ok(Err(e)) => format!("ERR: {}", e),
        Err(_) => "PANIC".to_string(),
    }
}

#[test]
fn hostile_inputs_never_panic() {
    // Generated inputs are bound first: `&format!(...)` in the vec would be a
    // temporary dropped while the vec still borrows it.
    let deep_parens_100 = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1{}; }} }}",
        "(".repeat(100),
        ")".repeat(100)
    );
    let deep_parens_2000 = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1{}; }} }}",
        "(".repeat(2000),
        ")".repeat(2000)
    );
    let deep_unary_5000 = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1; }} }}",
        "-".repeat(5000)
    );
    let deep_blocks = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1;{} }} }}",
        "{ ".repeat(2000),
        " }".repeat(2000)
    );
    let many_statements = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1; }} }}",
        "x = x + 1; ".repeat(20000)
    );
    // Statement-nesting recursion (`if`/`while`/`for`) bypasses parse_expr.
    let deep_ifs = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1;{} }} }}",
        "if true { ".repeat(2000),
        " }".repeat(2000)
    );
    let deep_whiles = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1;{} }} }}",
        "while false { ".repeat(2000),
        " }".repeat(2000)
    );

    let cases: Vec<(&str, &str)> = vec![
        ("empty input", ""),
        ("whitespace only", "   \n\t  "),
        ("comment only", "// nothing here\n"),
        ("unterminated line comment", "// trailing"),
        (
            "unterminated block comment",
            "contract A { state { x: u64 } } /*",
        ),
        (
            "nested block comment",
            "/* /* */ contract A { state { x: u64 } }",
        ),
        (
            "empty block comment",
            "/**/ contract A { state { x: u64 } }",
        ),
        (
            "comment between tokens",
            "contract/*x*/A { state { x: u64 } }",
        ),
        (
            "division not comment",
            "contract A { state { x: u64 } fn f() -> u64 { x / 2; } }",
        ),
        (
            "division adjacent neg",
            "contract A { state { x: u64 } fn f() -> u64 { x / -2; } }",
        ),
        (
            "slash slash is comment",
            "contract A { state { x: u64 } fn f() -> u64 { x; } } // tail",
        ),
        (
            "string with slashes",
            r#"contract A { state { x: u64 } fn f() -> u64 { "http://x"; } }"#,
        ),
        (
            "unterminated string",
            r#"contract A { state { x: u64 } fn f() -> u64 { "abc; } }"#,
        ),
        (
            "underscore literal",
            "contract A { state { x: u64 } fn f() -> u64 { 1_000; } }",
        ),
        (
            "hex literal",
            "contract A { state { x: u64 } fn f() -> u64 { 0xFF; } }",
        ),
        (
            "float literal",
            "contract A { state { x: u64 } fn f() -> u64 { 1.5; } }",
        ),
        (
            "double negation",
            "contract A { state { x: i64 } fn f() -> i64 { --x; } }",
        ),
        (
            "double minus no space",
            "contract A { state { x: u64 } fn f() -> u64 { 1-1; } }",
        ),
        (
            "comparison chain",
            "contract A { state { x: u64 } fn f() -> u64 { 1 < 2 < 3; } }",
        ),
        (
            "assignment double equals",
            "contract A { state { x: u64 } fn f() -> u64 { x = = 1; } }",
        ),
        (
            "assignment in expression",
            "contract A { state { x: u64 } fn f() -> u64 { 1 + (x = 2); } }",
        ),
        (
            "empty function body",
            "contract A { state { x: u64 } fn f() -> u64 { } }",
        ),
        ("empty contract body", "contract A { }"),
        ("missing contract name", "contract { }"),
        (
            "missing state brace",
            "contract A { state { x: u64 fn f() -> u64 { 1; } }",
        ),
        (
            "missing fn body brace",
            "contract A { state { x: u64 } fn f() -> u64 { 1; ",
        ),
        (
            "invariant no semicolon",
            "contract A { invariant x >= 0 state { x: u64 } }",
        ),
        (
            "invariant no expr",
            "contract A { invariant ; state { x: u64 } }",
        ),
        (
            "verified no parens",
            "contract A { state { x: u64 } @verified fn f() -> u64 { 1; } }",
        ),
        (
            "verified unknown key",
            "contract A { state { x: u64 } @verified(foo: 1) fn f() -> u64 { 1; } }",
        ),
        (
            "verified unterminated",
            "contract A { state { x: u64 } @verified(pre: 1 fn f() -> u64 { 1; } }",
        ),
        (
            "unknown attribute",
            "contract A { state { x: u64 } @payable fn f() -> u64 { 1; } }",
        ),
        (
            "range operator",
            "contract A { state { x: u64 } fn f() -> u64 { for i in 0..3 { x = i; } x; } }",
        ),
        (
            "range inclusive",
            "contract A { state { x: u64 } fn f() -> u64 { for i in 0..=3 { x = i; } x; } }",
        ),
        (
            "nested generics",
            "contract A { state { x: Vec<Vec<u64>> } }",
        ),
        ("trailing comma state", "contract A { state { x: u64, } }"),
        ("no fields trailing comma", "contract A { state { , } }"),
        ("duplicate field", "contract A { state { x: u64, x: u64 } }"),
        (
            "unicode identifier",
            "contract Ünïcode { state { x: u64 } }",
        ),
        (
            "emoji in source",
            "contract A { state { x: u64 } fn f() -> u64 { \u{1F600}; } }",
        ),
        (
            "null byte",
            "contract A { state { x: u64 } fn f() -> u64 { 1; }\0 }",
        ),
        (
            "crlf",
            "contract A {\r\n state { x: u64 }\r\n fn f() -> u64 { 1; }\r\n}",
        ),
        ("deep parens 100", &deep_parens_100),
        ("deep parens 2000", &deep_parens_2000),
        ("deep unary 5000", &deep_unary_5000),
        ("deep blocks", &deep_blocks),
        ("deep if statements", &deep_ifs),
        ("deep while statements", &deep_whiles),
        ("many statements", &many_statements),
    ];

    let mut panics = 0;
    for (label, input) in &cases {
        // Print BEFORE parsing: a stack overflow aborts the process and no
        // post-hoc line is ever written.
        println!("[case] {}", label);
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let result = classify(input);
        println!("    => {}", result);
        if result == "PANIC" {
            panics += 1;
        }
    }
    assert_eq!(panics, 0, "{} hostile input(s) panicked", panics);
}

/// The defects this probe found must stay fixed: each input has an expected
/// clean outcome, not just "no panic".
#[test]
fn hostile_inputs_are_classified_cleanly() {
    let deep_parens = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1{}; }} }}",
        "(".repeat(2000),
        ")".repeat(2000)
    );
    let deep_unary = format!(
        "contract A {{ state {{ x: u64 }} fn f() -> u64 {{ {}1; }} }}",
        "-".repeat(5000)
    );
    let cases: Vec<(&str, &str, bool)> = vec![
        // (label, input, should_parse)
        (
            "string with slashes",
            r#"contract A { state { x: u64 } fn f() -> u64 { "http://x"; } }"#,
            true,
        ),
        (
            "unterminated string",
            r#"contract A { state { x: u64 } fn f() -> u64 { "abc; } }"#,
            false,
        ),
        (
            "unterminated block comment",
            "contract A { state { x: u64 } } /*",
            false,
        ),
        (
            "nested block comment",
            "/* /* */ contract A { state { x: u64 } }",
            true,
        ),
        (
            "underscore literal",
            "contract A { state { x: u64 } fn f() -> u64 { 1_000; } }",
            false,
        ),
        (
            "hex literal",
            "contract A { state { x: u64 } fn f() -> u64 { 0xFF; } }",
            true,
        ),
        (
            "malformed hex",
            "contract A { state { x: u64 } fn f() -> u64 { 0x; } }",
            false,
        ),
        (
            "float literal",
            "contract A { state { x: u64 } fn f() -> u64 { 1.5; } }",
            false,
        ),
        ("deep parens", &deep_parens, false),
        ("deep unary", &deep_unary, false),
        (
            "crlf",
            "contract A {\r\n state { x: u64 }\r\n fn f() -> u64 { 1; }\r\n}",
            true,
        ),
        (
            "unicode identifier",
            "contract Ünïcode { state { x: u64 } }",
            false,
        ),
        (
            "null byte",
            "contract A { state { x: u64 } fn f() -> u64 { 1; }\0 }",
            false,
        ),
    ];

    for (label, input, should_parse) in cases {
        let parsed = Parser::parse(&input);
        assert_eq!(
            parsed.is_ok(),
            should_parse,
            "{}: expected {}, got {:?}",
            label,
            if should_parse { "Ok" } else { "Err" },
            parsed.map(|c| c.len())
        );
    }
}
