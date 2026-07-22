//! The shared Markdown lexer: one `pulldown-cmark` parser construction with
//! two views over it.
//!
//! - [`parse_document`] builds a span-free [`ast::Block`] tree, used by the
//!   formatter (`adept_fmt`) to re-print whole documents.
//! - [`headings`], [`link_destinations`] and [`inline_code_spans`] are
//!   positioned queries, used by the `SL1xx` lint rules, which need a line
//!   number for each finding but no tree.
//!
//! Both views go through [`parser`], so the linter and the formatter cannot
//! disagree about what a heading, a link, or a code block is. Fence
//! matching, info strings, indented code blocks, nested brackets and
//! reference links all come from `pulldown-cmark` rather than from a
//! hand-rolled line scan.

pub mod ast;
mod build;
mod query;

pub use build::parse_document;
pub use query::{headings, inline_code_spans, link_destinations, Heading, Located};

use pulldown_cmark::{Options, Parser};

/// Construct the shared `pulldown-cmark` parser for a document body.
///
/// This is the **single** parser-construction site in the workspace, and it
/// is deliberately shared by *both* views of a document: the formatter's
/// [`parse_document`] AST and the linter's positioned queries in
/// [`query`]. Adding a `pulldown_cmark::Options` feature flag here changes
/// both at once — which is the point. Do not construct a `Parser` anywhere
/// else: a flag enabled for one view and not the other would let the
/// linter and the formatter drift back into disagreeing about what a
/// heading or a link is.
pub fn parser(source: &str) -> Parser<'_> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);
    Parser::new_ext(source, options)
}

/// Maximum nesting depth (of block quotes, lists, and footnote definitions)
/// that will be parsed/printed as a proper structured tree. Chosen in line
/// with common CommonMark reference implementations, which bound container
/// nesting to a similar order of magnitude (e.g. `cmark`'s own recursion
/// guard) to avoid unbounded-recursion stack overflows on adversarial or
/// pathological input while comfortably covering any realistic document.
/// Content nested deeper than this is preserved verbatim as
/// [`ast::Block::Raw`] instead of being recursed into further.
pub const MAX_NESTING_DEPTH: usize = 100;
