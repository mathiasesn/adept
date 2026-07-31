//! All prompt templates used by `adept_agent`, gathered in one module so they
//! can be audited, mirroring `adept_score::prompts`. Template rendering
//! (`render`) is shared with `adept_score` rather than duplicated here.

/// System prompt for a description-scoped fix request (`SL301`
/// `description-tokens-over-budget` and/or `SL206` `no-negative-guidance`).
///
/// Both rules only ever touch the `description` field, so their
/// diagnostics are always batched into a single request with combined
/// constraints — never issued as two competing requests. Expects a JSON
/// response shaped like: `{"description": "..."}`.
pub const DESCRIPTION_FIX_SYSTEM: &str = r#"You are editing the YAML frontmatter `description` field of an AI agent skill (SKILL.md). You will be given the skill's name, its current description, its body (for context only), and a list of lint violations against the description with concrete constraints. Rewrite the description so that it satisfies every constraint while still accurately stating what the skill does and when it should trigger.

Respond with strict JSON only, no commentary, in exactly this shape:
{"description": "<the rewritten description>"}

Do not include a "body" or "companion_edits" key in your response."#;

/// User-message template for [`DESCRIPTION_FIX_SYSTEM`].
///
/// Format with `skill_name`, `description`, `body`, and `violations` (a
/// pre-rendered bullet list of violated rules and their constraints).
pub const DESCRIPTION_FIX_USER_TEMPLATE: &str = "Skill name: {skill_name}\n\nCurrent description:\n{description}\n\nCurrent body (context only, do not rewrite):\n{body}\n\nViolations to fix:\n{violations}";

/// System prompt for a body-scoped fix request (`SL302`
/// `body-tokens-over-budget`).
///
/// Instructs the model to relocate detailed material into a companion file
/// rather than deleting it outright, since [`crate::fix::relocate`] rejects any
/// candidate that loses content instead of moving it. Expects a JSON
/// response shaped like:
/// `{"body": "...", "companion_edits": [{"path": "...", "appended_content": "..."}]}`.
pub const BODY_FIX_SYSTEM: &str = r#"You are editing the Markdown body of an AI agent skill (SKILL.md) that is over its token budget. You will be given the skill's name, its description (for context only), its current body, and the concrete token budget it must fit within. Reduce the body's token count by relocating detailed reference material (long tables, exhaustive option lists, verbose examples) into a companion reference file rather than deleting it — content must be moved, not lost. Leave a brief pointer in the body to where the relocated material now lives.

Respond with strict JSON only, no commentary, in exactly this shape:
{"body": "<the shortened body>", "companion_edits": [{"path": "REFERENCE.md", "appended_content": "<the relocated material>"}]}

`companion_edits` may be an empty list only if the budget can be met by trimming genuinely redundant prose alone; prefer relocation over deletion whenever in doubt. Each `path` MUST be a plain relative filename inside the skill's own directory: no `..`, no absolute paths, and no subdirectories."#;

/// User-message template for [`BODY_FIX_SYSTEM`].
///
/// Format with `skill_name`, `description`, `body`, and `violations`.
pub const BODY_FIX_USER_TEMPLATE: &str = "Skill name: {skill_name}\n\nDescription (context only):\n{description}\n\nCurrent body:\n{body}\n\nViolations to fix:\n{violations}";
