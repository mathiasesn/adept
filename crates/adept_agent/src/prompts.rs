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

/// Prompt version for [`CREATE_AUTHORING_SYSTEM`] and its user templates
/// ([`CREATE_AUTHORING_USER_TEMPLATE`], [`CREATE_REPAIR_USER_TEMPLATE`]).
/// Independent of [`CREATE_EVAL_PROMPT_VERSION`] and of
/// `adept::evals::SCHEMA_VERSION` — authoring wording drifts for unrelated
/// reasons to test-generation wording and to the dataset's on-disk shape.
pub const CREATE_AUTHORING_PROMPT_VERSION: u32 = 1;

/// System prompt for `adept create`'s authoring call: generates a
/// structured JSON skill candidate from a brief, encoding the guidance a
/// deterministic linter cannot check.
///
/// Used for both the initial generation ([`CREATE_AUTHORING_USER_TEMPLATE`])
/// and every repair round ([`CREATE_REPAIR_USER_TEMPLATE`]) — both are the
/// same authoring task (produce a complete, valid skill candidate), so they
/// share one system prompt and one [`CREATE_AUTHORING_PROMPT_VERSION`].
pub const CREATE_AUTHORING_SYSTEM: &str = r#"You are authoring a new AI agent skill: a SKILL.md file (YAML frontmatter plus a Markdown body) that teaches an agent how to perform a specific task, plus zero or more companion reference files.

Judgment this task requires that no linter can check:
- **Invocation mode.** Decide `disable_model_invocation`: `true` only for a skill the user must invoke explicitly (destructive, credentialed, or otherwise unsafe to trigger automatically); `false` (the default) for a skill an agent should be able to trigger on its own from a matching request.
- **Trigger-bearing description.** The `description` is read by an agent on every turn to decide whether to use this skill. It must state both *what the skill does* and *when to use it* — concrete trigger conditions and, where relevant, negative guidance about when NOT to use it — in roughly one or two sentences. Vague restatements of the name are useless for triggering.
- **Information hierarchy.** Put only what is needed on every invocation in the body; move detailed reference material (long tables, exhaustive option lists, verbose examples) into companion files the body points to. The body should read like a procedure, not a manual.
- **The pruning / no-op test.** For every sentence you are about to write, ask: does removing this sentence change what the agent does? If not, cut it. Skills accrete filler ("this is important", "make sure to be careful") that does no work.
- **Six failure modes to actively avoid:**
  1. *Premature completion* — the skill lets the agent stop before the task is actually done.
  2. *Duplication* — the skill restates what a general-purpose agent already knows how to do.
  3. *Sediment* — accumulated caveats and exceptions from past failures, left in as unstructured prose instead of a clean procedure.
  4. *Sprawl* — the skill tries to cover too many unrelated tasks instead of one focused one.
  5. *No-op* — instructions that do not change the agent's behavior at all (see the pruning test above).
  6. *Negation* — describing what NOT to do without ever stating the positive alternative, leaving the agent no path forward.

Respond with strict JSON only, no commentary, in exactly this shape:
{"name": "<kebab-case skill name>", "description": "<the trigger-bearing description>", "disable_model_invocation": <true or false>, "body": "<the SKILL.md Markdown body, no frontmatter>", "companion_files": [{"path": "REFERENCE.md", "content": "<file contents>"}]}

`companion_files` may be an empty list. Each `path` MUST be a plain relative filename inside the skill's own directory: no `..`, no absolute paths, no subdirectories."#;

/// User-message template for the initial authoring call.
///
/// Format with `brief` and `siblings` (a pre-rendered bullet list of
/// existing sibling skills' names and descriptions, or a line stating none
/// were found — never duplicate one of these).
pub const CREATE_AUTHORING_USER_TEMPLATE: &str = "Brief describing the skill to author:\n{brief}\n\nExisting sibling skills already in this set (do not duplicate any of their names or descriptions):\n{siblings}";

/// User-message template for a repair round: the model is shown its own
/// prior candidate plus the diagnostics it produced, and asked to revise.
///
/// Format with `brief`, `name`, `description`, `body`, and `diagnostics` (a
/// pre-rendered bullet list, covering both the candidate's own findings and
/// any newly-appeared sibling collisions).
pub const CREATE_REPAIR_USER_TEMPLATE: &str = "Original brief:\n{brief}\n\nYour previous candidate:\nname: {name}\ndescription: {description}\nbody:\n{body}\n\nLint diagnostics to resolve (revise the candidate so none of these remain, while remaining faithful to the brief):\n{diagnostics}";

/// Prompt version for [`CREATE_EVAL_SYSTEM`]/[`CREATE_EVAL_USER_TEMPLATE`].
/// Independent of [`CREATE_AUTHORING_PROMPT_VERSION`]: test-case generation
/// is a different task from authoring and its wording changes for unrelated
/// reasons (upskill separates these roles for the same reason). Also
/// independent of `adept::evals::SCHEMA_VERSION`, which only changes when
/// the dataset's on-disk *shape* changes.
pub const CREATE_EVAL_PROMPT_VERSION: u32 = 1;

/// System prompt for `adept create`'s eval-dataset generation call.
///
/// Given both the accepted skill and the original brief, since they answer
/// different questions: the skill is what a case must be answerable
/// against, and the brief is the only record of intent the skill may have
/// failed to capture — a case derived solely from the skill can never
/// detect an omission.
pub const CREATE_EVAL_SYSTEM: &str = r#"You are generating a synthetic evaluation dataset for an AI agent skill that was just authored. You will be given the original brief (the intent behind the skill) and the accepted skill itself (name, description, body). Generate test cases that exercise the skill.

Two things matter about the two inputs: the skill's content is what a correct response must be answerable against, since that is all the agent under test will have; the brief is the only record of what was originally intended, so a case built only from the skill can never catch something the skill quietly failed to cover. If a case tests something the brief asked for but the skill's body does not actually address, that is a useful, legitimate case — it will read as a coverage gap, not a bug in your dataset.

Each case has a `prompt` (a realistic request the skill should handle) and `assertions` describing what a correct response must satisfy. Use only these assertion kinds:
- {"kind": "contains", "value": "<substring>"} — the response contains this substring.
- {"kind": "file_exists", "path": "<path>"} — a file at this path exists.
- {"kind": "file_contains", "path": "<path>", "value": "<substring>"} — a file at this path exists and contains this substring.
- {"kind": "command", "command": "<shell command>"} — a shell command whose exit code alone decides pass/fail.

Respond with strict JSON only, no commentary, in exactly this shape:
{"cases": [{"prompt": "...", "assertions": [{"kind": "contains", "value": "..."}]}]}

Every case must have at least one assertion. Generate exactly the requested number of cases."#;

/// User-message template for [`CREATE_EVAL_SYSTEM`].
///
/// Format with `brief`, `skill_name`, `description`, `body`, and `n` (the
/// number of cases to generate, as a string).
pub const CREATE_EVAL_USER_TEMPLATE: &str = "Original brief:\n{brief}\n\nAccepted skill:\nname: {skill_name}\ndescription: {description}\nbody:\n{body}\n\nGenerate exactly {n} test cases.";
