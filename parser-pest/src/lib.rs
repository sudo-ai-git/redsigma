//! REDΣSIGMA Parser - Re-exports the hand-written recursive descent parser

pub use redsigma_parser::{
    Parser,
    Contract,
    Invariant,
    StateDecl,
    Type,
    Function,
    VerifiedAnnotation,
    Stmt,
    Expr,
    BinOp,
    UnOp,
    ParseError,
};
