# adept: a linter, formatter, and scorer for Agent Skills

## Problem / Why
Name: **adept** — repo `adept`, binary `adept`, crates `adept` (core), `adept_cli`,
`adept_fmt`. Tagline: "an extremely fast linter and formatter for Agent Skills."
Crates.io name verified available (2026-07-22).

Skills (the folder-of-instructions pattern: SKILL.md + companion files) are proliferating
across ecosystems (Anthropic, OpenAI, community repos), but there is no tooling to check
quality. Common defects: vague descriptions that fail to trigger (or over-trigger), token
bloat, malformed frontmatter, conflicting/overlapping skills, broken file references.
A fast, ruff-style CLI has first-mover potential.

## Goals
- `adept check <path>`: static lint of one skill or a directory of skills, with
  ruff-style diagnostics (rule codes, file:line, fix suggestions).
- `adept fmt <path>`: full prettier-style formatting of SKILL.md — canonical
  frontmatter (key order, quoting), plus complete markdown reflow: heading/list/emphasis
  normalization, fenced-code normalization, table alignment, line wrap at a configurable
  width (default 100), idempotent output. Built on a CommonMark parser (e.g.
  `pulldown-cmark`/`comrak`) with a custom printer.
- `adept score <path>`: LLM-assisted scoring — triggering accuracy (generate a small
  eval set of should-trigger / shouldn't-trigger prompts and judge), token bloat,
  conflict/overlap detection across a skill set, improvement suggestions.
- Distributable as a single Rust binary; fast static checks with no network by default.
- MCP server mode (`adept mcp`, stdio transport) so agents can lint/score skills
  themselves. v1 ships all four surfaces: check, fmt, score, MCP.

## Non-goals
- A hosted registry/marketplace or index (the "registry-style index" idea) — deferred
  entirely; future work.
- Executing or sandbox-testing skill scripts.
- Full agent-harness end-to-end evals (only prompt→trigger judgments in `score`).

## Constraints
- Rust; project structure modeled on ruff but simpler (cargo workspace with crates like
  `adept` (core rules), `adept_cli`, maybe `adept_fmt`).
- Static checks must be offline and fast; `score` calls any OpenAI-compatible
  chat-completions endpoint, configured via `ADEPT_BASE_URL`, `ADEPT_API_KEY`,
  `ADEPT_MODEL` (or CLI flags/config file). This covers OpenAI, local servers
  (Ollama, vLLM), and Anthropic via its OpenAI-compat layer.
- Skill format: Anthropic SKILL.md only for v1 (YAML frontmatter with required
  `name`, `description`; optional fields like `license` tolerated). Other ecosystems'
  formats are future work; keep the parser behind a trait to ease later pluggability.

## Proposed approach
- Parser crate: parse YAML frontmatter + markdown body into a `Skill` model; walk
  directories to build a `SkillSet`.
- Rule engine: rules identified by codes (e.g. `SL001 missing-description`,
  `SL1xx` structure, `SL2xx` description/triggering heuristics, `SL3xx` token budget,
  `SL4xx` cross-skill conflicts), each with severity, message, optional autofix.
- Static heuristics for description quality (length bounds, presence of trigger phrases,
  first/third person, "do not use for..."), token counting via the `tiktoken-rs` crate
  (o200k_base default, cl100k_base selectable) with thresholds for description/body
  token budgets, reference checking (files mentioned exist).
- `score`: LLM generates N candidate user prompts (positive/negative), then a judge model
  predicts whether the description would trigger; report precision/recall-style score.
  Overlap detection: pairwise description similarity + LLM adjudication.
- Output formats: human (colored), JSON; exit codes CI-friendly.

## Acceptance criteria
- `adept check` on the anthropics/skills repo runs clean-ish and flags seeded
  defects in fixture skills (golden-file tests per rule).
- `fmt` is idempotent (fmt(fmt(x)) == fmt(x)).
- `score` produces a numeric triggering-accuracy score plus a written report for a skill,
  given an API key.
- CI: `cargo test`, `cargo clippy -D warnings` pass.
- Performance: `check` lints 100 skills in under 1 second on a typical dev machine
  (measured via a criterion benchmark in CI-adjacent tooling).

## Open questions
(none)

## Risks
- Triggering-accuracy scoring is inherently noisy (LLM judge variance) — mitigate with
  fixed seeds/prompt sets and reporting confidence.
- Skill format fragmentation across ecosystems; schema may need to be pluggable.
- Full markdown reflow is the highest-effort v1 component; guard with idempotency and
  round-trip (semantic-equality) tests, and a `--check` mode before adopting broadly.
