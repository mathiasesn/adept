//! LLM-assisted agent capabilities for Agent Skills (`adept fix` and
//! `adept create`).
//!
//! [`candidate`] (model JSON response parsing and companion-path
//! sandboxing), [`diff`] (multi-file unified diff rendering), [`prompts`]
//! (prompt templates), [`writer`] (atomic/transactional file writes), and
//! [`gate`] (accept/reject scoring for a candidate against its baseline) are
//! shared machinery, kept at the crate root so sibling modules ([`fix`] and
//! [`create`]) can reuse them without depending on each other. [`fix`] is the
//! `adept fix` command's own implementation: its options and the SL302
//! token-conservation guard are specific to that command and live under
//! `fix::`.
//!
//! This crate is the deliberate top-of-stack exception to the workspace's
//! one-way dependency rule: it may compose `adept_score` and `adept_fmt`;
//! nothing in the library stack may depend on it; only `adept_cli` consumes
//! it.

pub mod candidate;
pub mod create;
pub mod diff;
pub mod fix;
mod gate;
pub mod prompts;
pub mod writer;

pub use candidate::{
    resolve_companion_path, CompanionEdit, FixCandidate, FixResponse, UnsafeCompanionPath,
};
pub use create::{
    create_skill, generate_evals, CreateError, CreateOptions, CreateOutcome, CreateReport,
};
pub use fix::{fix_skill, FixError, FixOptions, FixOutcome, FixReport, DEFAULT_MAX_ROUNDS};
pub use prompts::{
    BODY_FIX_SYSTEM, BODY_FIX_USER_TEMPLATE, CREATE_AUTHORING_PROMPT_VERSION,
    CREATE_AUTHORING_SYSTEM, CREATE_AUTHORING_USER_TEMPLATE, CREATE_EVAL_PROMPT_VERSION,
    CREATE_EVAL_SYSTEM, CREATE_EVAL_USER_TEMPLATE, CREATE_REPAIR_USER_TEMPLATE,
    DESCRIPTION_FIX_SYSTEM, DESCRIPTION_FIX_USER_TEMPLATE,
};
pub use writer::{write_all_transactionally, write_atomically};
