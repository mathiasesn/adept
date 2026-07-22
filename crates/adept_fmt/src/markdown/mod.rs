//! A small Markdown AST plus a builder (from `pulldown-cmark` events) and a
//! deterministic printer, used to implement full-body Markdown reflow.

pub mod ast;
mod build;
mod print;

pub use build::parse_document;
pub use print::print_document;
