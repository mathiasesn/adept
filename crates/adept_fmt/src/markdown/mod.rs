//! The formatter's Markdown layer: a deterministic printer over the shared
//! Markdown AST.
//!
//! The AST ([`ast`]) and the builder ([`parse_document`]) live in the
//! `adept` core crate ([`adept::markdown`]), shared with the `SL1xx` lint
//! rules so that the linter and the formatter cannot disagree about what a
//! heading or a link is. Only the printer is formatter-specific.

mod print;

pub use adept::markdown::{ast, parse_document, MAX_NESTING_DEPTH};
pub use print::print_document;
