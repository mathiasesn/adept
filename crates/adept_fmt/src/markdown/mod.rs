//! A small Markdown AST plus a builder (from `pulldown-cmark` events) and a
//! deterministic printer, used to implement full-body Markdown reflow.

pub mod ast;
mod build;
mod print;

pub use build::parse_document;
pub use print::print_document;

/// Maximum nesting depth (of block quotes, lists, and footnote definitions)
/// that will be parsed/printed as a proper structured tree. Chosen in line
/// with common CommonMark reference implementations, which bound container
/// nesting to a similar order of magnitude (e.g. `cmark`'s own recursion
/// guard) to avoid unbounded-recursion stack overflows on adversarial or
/// pathological input while comfortably covering any realistic document.
/// Content nested deeper than this is preserved verbatim as
/// [`ast::Block::Raw`] instead of being recursed into further.
pub(crate) const MAX_NESTING_DEPTH: usize = 100;
