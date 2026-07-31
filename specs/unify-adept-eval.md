# Unify `adept score` and eval-dataset grading into `adept eval`

Three pieces of work, in dependency order:

1. **Refactor** — dissolve the `adept_score` crate into `adept_agent`, so
   the scoring code sits alongside `fix` and `create` and LLM transport has
   one home.
2. **Feature** — grade a skill against its `evals/evals.jsonl` using run
   results supplied by an external harness, reporting pass rate, assertion
   success, and skill lift, in the spirit of
   [huggingface/upskill](https://github.com/huggingface/upskill)'s
   `upskill eval`.
3. **Unify** — merge the `score` command into `eval`, so **one** command
   answers "how good is this skill?" across all four analyses: triggering
   accuracy, token bloat, cross-skill overlap, and eval-dataset
   performance. `adept score` ceases to exist as a separate surface.

Steps 1 and 2 are independent; step 3 depends on both.

## Problem / Why

### The crate split

The workspace currently splits LLM-assisted work across two crates:

- `adept_score` — LLM transport (`LlmClient`, `OpenAiCompatClient`,
  `MockLlmClient`, `CaptureSink`) **plus** the three scoring analyses
  (`triggering`, `tokens`, `overlap`) and `ScoreReport`.
- `adept_agent` — `fix` and `create`, built on shared machinery
  (`candidate`/`diff`/`prompts`/`writer`/`gate`), composing `adept_score`
  for transport and `adept_fmt` for canonicalization.

That split makes `adept_score` two things at once: a transport layer every
LLM surface depends on, and one particular capability that is a sibling of
`fix` and `create`. The result is an awkward dependency story
(`adept_agent` is a documented exception to the one-way rule purely so it
can reach transport), scattered prompt-version constants
(`adept_score::prompts::PROMPT_VERSION` vs.
`adept_agent::prompts::CREATE_*_PROMPT_VERSION`), and no single home for
"the LLM-assisted capabilities of adept".

### The ungraded eval dataset

`adept create` writes a synthetic eval dataset for every skill it
generates, and `docs/EVALS.md` publishes the schema — but adept has no way
to grade one, so the datasets it produces are inert. A skill's *form* is
measured by `check`; its *behaviour* is measured by nothing.

### Two commands answering one question

Once grading exists, `score` and a separate grading command would both
answer "how good is this skill?" — one from the outside (would it trigger?
is it bloated? does it collide?), one from the inside (does it actually
work?). Users would have to know which surface holds which half of the
answer, and the two reports would drift apart in format and flags. One
`adept eval` with four analyses is the coherent surface.

## Goals

- One evaluation command, `adept eval`, covering triggering accuracy,
  token bloat, cross-skill overlap, and eval-dataset performance, with a
  single unified report in both `human` and `json` formats.
- A grading-only `adept eval` run works fully offline, with no
  `ADEPT_MODEL` configured and no network access.
- The scoring code lives alongside `fix` and `create` in `adept_agent`;
  LLM transport has one clear home named for transport, not for scoring.
- `docs/ARCHI.md`, `CLAUDE.md`, `docs/EVALS.md`, and `README.md` describe
  the new layout, the revised dependency-direction invariant, and the
  revised network invariant.

## Non-goals

- **adept does not become an eval runner.** It never executes a case,
  never spawns a subprocess, and never calls a model to grade an
  assertion. Driving the agent under test stays the harness's job.
- No multi-model benchmarking, run-history storage, or plotting
  (upskill's `upskill runs`); adept grades one results file and prints one
  report.
- No change to `check`, `fmt`, `fix`, or `create` behaviour, flags, or
  output.
- No change to the eval *dataset* schema or `adept::evals::SCHEMA_VERSION`
  — the results sidecar is a separate format.
- No change to scoring behaviour, prompt wording, or prompt versions: the
  three existing analyses produce identical numbers for identical inputs
  before and after. `PROMPT_VERSION`'s value does not move.
- No new lint rules; no rule codes added or retired.

## Constraints

- Rust virtual workspace, five crates today, one binary (`adept`).
- Existing invariants that name `adept_score` must be updated, not
  silently broken:
  - "No test may perform network I/O — use `adept_score::MockLlmClient`."
  - "`adept_fmt` and `adept_score` depend on `adept` and never on each
    other… `adept_agent` is the one deliberate exception."
  - "Construct `ScoreOptions` via `ScoreOptions::for_model`, and bump
    `PROMPT_VERSION` in `adept_score::prompts`…"
- The invariant "`check` and `fmt` never touch the network. Only
  `score`/`fix` (and the MCP `score_skill` tool) do" must be restated for
  a command that is *conditionally* network-backed (see below).
- `adept_agent` must remain fully async and must never create a tokio
  runtime.
- Exit codes are a public contract: `0` clean, `1` findings, `2`
  usage/I/O error.
- CI gates must stay green: `cargo build --workspace --all-targets`,
  `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, plus the `lint_100_skills` perf smoke
  test.
- Nothing in the library stack may depend on `adept_agent`; only
  `adept_cli` may.

## Proposed approach — part 1: dissolve `adept_score`

The `adept_score` crate is deleted and its contents land in `adept_agent`
as two module trees — transport and capability.

Target layout:

```
crates/
  adept/        core: parse, rules, tokens, markdown, evals (schema + grader)
  adept_fmt/    formatter
  adept_agent/
    llm/        client.rs, capture.rs, mock.rs  (transport + test double)
    eval/       triggering.rs, tokens.rs, overlap.rs, report.rs, prompts
    fix/        unchanged
    create/     unchanged
    candidate.rs diff.rs prompts.rs writer.rs gate.rs  (shared machinery)
  adept_cli/    clap CLI + adept.toml + MCP stdio server
```

Steps:

1. `git mv crates/adept_score/src/{client,capture,mock}.rs` into
   `crates/adept_agent/src/llm/`, with an `llm/mod.rs` re-exporting the
   same public names (`LlmClient`, `OpenAiCompatClient`, `MockLlmClient`,
   `CaptureSink`, `CapturedCall`, `RunMetadata`, `ChatMessage`/`Request`/
   `Response`/`Role`, `LlmConfig`, `ResolvedLlmConfig`, `RedactedString`,
   `LlmError`, `ConfigError`, `DEFAULT_BASE_URL`, `ENV_*`).
2. `git mv` `triggering.rs`, `tokens.rs`, `overlap.rs`, `report.rs` and
   the score `prompts.rs` into `crates/adept_agent/src/eval/`. The
   crate-level `score_skill`, `ScoreOptions` and `ScoreError` move to
   `eval/mod.rs` under their new names (part 3).
3. Resolve the `prompts` collision: the eval prompt templates and
   `PROMPT_VERSION` live at `adept_agent::eval::prompts`; the crate-root
   `adept_agent::prompts` keeps the fix/create templates.
4. Merge `crates/adept_score/Cargo.toml` deps into
   `crates/adept_agent/Cargo.toml`, drop the `adept_score` dependency, and
   remove the crate from the workspace members list.
5. Re-export from `adept_agent`'s root so `adept_cli` call sites change by
   path only.
6. Rewrite `adept_cli` imports (`commands/{score,fix,create,mcp}.rs`,
   `config.rs`, tests) from `adept_score::` to `adept_agent::`.

The dependency-direction invariant becomes: `adept_fmt` depends only on
`adept`; `adept_agent` is top-of-stack and may compose `adept` and
`adept_fmt`; nothing in the library stack may depend on `adept_agent`;
only `adept_cli` consumes it. The "one deliberate exception" clause for
`adept_score` is removed as no longer applicable.

## Proposed approach — part 2: eval-dataset grading

**adept does not become the runner.** upskill's runner executes an agent
in a working directory; adept's invariants forbid that ("adept never
executes an eval dataset"; "adept spawns no subprocess, ever"). Instead
**adept provides the CLI the harness calls**: the harness (an agent, a CI
job, an `upskill`-style runner) executes each case and hands the results
to adept, which owns dataset parsing, grading, and metric aggregation.
Both invariants survive unchanged. `docs/EVALS.md`'s "grading is the job
of a separate harness" becomes "grading is the job of `adept`, invoked by
a separate harness" — adept stops merely *defining* a shape and becomes
the reference grader for it.

The division of labour follows from the subprocess invariant. Only
`command` needs to spawn anything; `contains` is a substring check and
`file_exists`/`file_contains` are plain filesystem reads, which adept does
throughout:

| Assertion | Graded by | Why |
| --- | --- | --- |
| `contains` | **adept** | substring match on harness-supplied response text |
| `file_exists` | **adept** | filesystem read, relative to a harness-supplied working directory |
| `file_contains` | **adept** | filesystem read + substring match |
| `command` | **harness** | requires a subprocess; harness reports the exit code back |

### The results sidecar

The dataset schema is already published, so the harness enumerates cases
and sets up arms itself, runs them, and hands adept one file.
`results.jsonl` is one JSON object per line:

| Field | Required | Meaning |
| --- | --- | --- |
| `case` | yes | Which dataset case this result is for — the 1-indexed line number in `evals/evals.jsonl` |
| `arm` | no | `"skill"` (default) or `"baseline"`; the baseline arm is what makes skill lift computable |
| `response` | yes | The agent's response text, graded by `contains` |
| `cwd` | no | Working directory the case ran in; `file_exists`/`file_contains` paths resolve against it. Absent ⇒ those assertions are `skipped` |
| `command_exit_codes` | no | Map of command string → exit code, as observed by the harness. A `command` assertion with no entry is `skipped` |
| `tokens` | no | `{"in": N, "out": N}`; aggregated when present |

This is a sidecar format versioned separately from the dataset;
`adept::evals::SCHEMA_VERSION` does not move.

### Where the grader lives

Grading needs **no LLM**: a substring match, two filesystem reads, and
some arithmetic. So the grader itself is offline and deterministic, and
lives in **`adept::evals`** (core), extending the module that already owns
the schema:

```rust
pub fn grade(cases: &[EvalCase], results: &[CaseResult]) -> EvalBenchmarkReport
```

`adept_agent::eval` composes it as one of the four analyses. Two similarly
named things, deliberately: `adept::evals` is the dataset — schema plus
grader, offline, no network; `adept_agent::eval` is the orchestrated
evaluation surface that runs all four analyses.

### Grading rules

- Dataset discovery: `evals/evals.jsonl` relative to the skill directory
  (the shorthand "`eval.jsonl`" in the original request — the real path is
  the one `create` writes and `docs/EVALS.md` publishes), parsed and
  validated via the existing `adept::evals::{parse_jsonl, validate}`.
  `--evals <PATH>` overrides it.
- Metrics, following upskill: per-case pass/fail, **pass rate**,
  **assertion success rate** (assertions met / assertions checked),
  **skill lift** (with-skill pass rate minus baseline pass rate, in
  percentage points) when both arms are present, and token usage when
  supplied. Lift is omitted, not zeroed, when there is no baseline arm.
- Every assertion outcome is `pass` / `fail` / `skipped`, and a skipped
  assertion **never** counts as a pass. Skipped assertions are excluded
  from the assertion-success denominator and reported separately with
  their reason, so a run that silently graded nothing cannot look like a
  perfect score.
- A case is `pass` only if every non-skipped assertion passed **and** at
  least one assertion was actually checked.
- A result naming a `case` outside the dataset, or a dataset case with no
  result, is reported explicitly rather than ignored.
- `file_exists`/`file_contains` resolve strictly inside `cwd` — a `path`
  escaping it (`../`, absolute) is an error, reusing the existing
  companion-path sandboxing rather than a second implementation.

## Proposed approach — part 3: unify under `adept eval`

`adept score` is replaced by `adept eval`, which runs four analyses and
prints one report.

```console
$ adept eval ./my-skill --results r.jsonl
triggering   P .92  R .88  F1 .90
token bloat  4.2k tokens, 3 suggestions
overlap      1 conflict: pdf-tools
evals        78% pass (39/50), baseline 42%, lift +36pp
             assertions 31/40 met (9 skipped: command)

$ adept eval ./my-skill --results r.jsonl --select evals   # offline
$ adept eval ./my-skill                                    # LLM analyses only
```

### Analysis selection

The four analyses are named `triggering`, `token-bloat`, `overlap`, and
`evals`, and are narrowed with `--select` / `--ignore`, reusing `check`'s
existing flag vocabulary and comma-separated/repeatable parsing (names
only — analyses have no `SL`-style codes).

Default selection is derived from what is available, so the common cases
need no flags:

- `evals` runs when `--results` is passed, and only then. Without run
  results there is nothing to grade.
- `triggering`, `token-bloat`, and `overlap` run when a model is
  configured (`--model` / `ADEPT_MODEL` / `[eval] model`).
- If an analysis is explicitly `--select`ed but its precondition is
  missing (e.g. `--select triggering` with no model, `--select evals` with
  no `--results`), that is a usage error (exit `2`) naming the missing
  input — never a silent skip.
- If nothing at all is available (no model, no results), exit `2` with a
  message saying so, rather than printing an empty report.
- Analyses not selected are absent from the report, distinguishably from
  an analysis that ran and found nothing.

### The network invariant, restated

`adept eval` is the first command that is *conditionally* network-backed.
The invariant becomes:

> `check`, `fmt`, and eval-dataset grading never touch the network. The
> `triggering`, `token-bloat`, and `overlap` analyses do, and they run
> only when a model is configured; `adept eval --select evals` makes no
> network call and requires no API key.

This is a testable claim, not just prose: a grading-only run must pass
with no model configured and no `LlmClient` ever constructed.

### Renames

The merge renames the surface consistently rather than leaving `score`
scattered through the internals:

| Before | After |
| --- | --- |
| `adept score <path>` | `adept eval <path>` |
| `adept_score` crate | `adept_agent::{llm, eval}` |
| `adept_agent::score` (part 1's interim name) | `adept_agent::eval` |
| `score_skill()` | `eval_skill()` |
| `ScoreOptions` / `ScoreError` / `ScoreReport` | `EvalOptions` / `EvalError` / `EvalReport` |
| `ScoreOptions::for_model` | `EvalOptions::for_model` |
| `[score]` in `adept.toml` | `[eval]` |
| MCP tool `score_skill` | MCP tool `eval_skill` |
| `[score] capture_dir` | `[eval] capture_dir` |

`EvalReport` gains a fourth optional field (`evals: Option<EvalBenchmarkReport>`)
alongside `triggering`, `token_bloat`, and `overlaps`. `adept::evals::EvalError`
(dataset parse errors) and `adept_agent::eval::EvalError` (transport/JSON
errors) are distinct types in distinct crates; the CLI already surfaces
those two failure classes separately, so this must not be collapsed.

`[eval]` remains independent of `[fix]` and `[create]` — still no fallback
between sections; the `ADEPT_MODEL` / `ADEPT_BASE_URL` / `ADEPT_API_KEY`
env vars stay the only shared thing.

### The `eval_skill` MCP tool

`score_skill` becomes `eval_skill`, gaining the `evals` analysis so an
agent harness can grade over the same stdio server it already uses for
lint/format. Two behaviours have to change rather than carry over:

**Advertisement is no longer gated on a model.** `mcp.rs` currently
advertises `score_skill` only when `LlmConfig::default().resolve().is_ok()`
(alongside `create_skill`/`generate_evals`), so agents never discover a
tool guaranteed to fail. `eval_skill` breaks that grouping: grading works
with no model at all. So `eval_skill` is **always advertised**, and its
description states which analyses need `ADEPT_MODEL` and that grading does
not. `create_skill`/`generate_evals` keep the existing gate — this is a
divergence from a shared code path that currently resolves once for all
three tools, so the resolution must stay shared while the gating stops
being uniform.

**Grading needs a real directory, so `content` mode can't grade files.**
`score_skill` accepts either `path` or raw `content`. `file_exists` and
`file_contains` resolve against the harness's `cwd`, which raw `content`
has no equivalent of. Therefore: passing `results` alongside `content`
grades `contains` only, and reports the `file_*` assertions as `skipped`
with that reason — it is not an error, and it must not be silently
reported as a pass.

Input schema, extending `score_skill`'s (`path` / `content` / `directory` /
`model` / `base_url`, `anyOf` requiring `path` or `content`):

| Argument | Meaning |
| --- | --- |
| `results` | Array of result objects, same fields as the `results.jsonl` lines — passed inline as JSON rather than as a file path, since an MCP client may not share a filesystem with the server |
| `evals` | Optional path to the dataset, overriding `evals/evals.jsonl` discovery relative to `path` |
| `select` / `ignore` | Optional arrays of analysis names, same four names and semantics as the CLI flags |

Preconditions behave as they do on the CLI: `evals` runs when `results` is
supplied, the three LLM analyses run when a model resolves, and explicitly
selecting an analysis whose precondition is missing is an error naming the
missing input.

Unlike `create_skill`/`generate_evals`, `eval_skill` is not "preview-only"
— it is simply read-only. It must never write to the skill directory, and
the existing timeout treatment for network-backed tools still applies to
the LLM analyses.

### Clean break, loudly

No compatibility shims: `adept score`, `[score]`, and the MCP `score_skill`
tool are removed outright rather than aliased. adept is pre-1.0 with one
surface per job, and permanent aliases mean two spellings to document and
test forever.

"Removed" must not mean "silently ignored", and one case needs real work:
**`crates/adept_cli/src/config.rs` has no `deny_unknown_fields`**, so a
stale `[score]` section in an existing `adept.toml` would today be parsed,
ignored, and the user's `model` / `capture_dir` / `num_prompts` would
quietly stop applying — a config that looks configured but isn't. So:

- Parsing a config containing `[score]` is a hard error (exit `2`) naming
  the replacement: "`[score]` is no longer read; rename it to `[eval]`".
  Implemented as an explicit legacy-section check rather than only
  `deny_unknown_fields`, so the message can name the fix.
- `adept score` produces clap's unrecognized-subcommand error. Adding a
  "`adept score` is now `adept eval`" tip line is worth doing if it can be
  achieved without a hidden subcommand that would leak into shell
  completions.
- An MCP `tools/call` for `score_skill` returns a JSON-RPC method/tool
  error naming `eval_skill`, not a silent no-op.

Whether to add `deny_unknown_fields` to the config structs generally is a
sensible follow-on but is **out of scope here** — it would turn every
unrelated typo into a hard error, which is a separate decision.

## Acceptance criteria

Refactor:

- `cargo build --workspace --all-targets`, `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`, and
  `cargo fmt --all -- --check` all pass.
- `crates/adept_score/` no longer exists; the workspace has four members;
  `grep -rn "adept_score" crates/ docs/ CLAUDE.md README.md` yields zero
  hits.
- The three pre-existing analyses produce byte-identical numbers to
  `adept score` for the same mock-client fixtures — snapshot content
  changes only where a label was renamed `score` → `eval`.
- `docs/ARCHI.md` crate list and dependency-direction rule, and the
  invariants in `CLAUDE.md`, match the new layout.

Grading:

- Fixture-in / insta-snapshot-out coverage for: all-pass, all-fail, mixed,
  baseline+skill arms (lift computed), skill-arm-only (lift omitted),
  every assertion kind, and every `skipped` reason (`command` with no exit
  code, `file_*` with no `cwd`).
- A skipped assertion is never counted as a pass, and a case whose
  assertions were all skipped is not reported as passing — pinned by a
  dedicated test, since this is the failure mode that would make the whole
  feature quietly useless.
- Malformed `results.jsonl`, a `case` index out of range, a dataset case
  with no result, and a `path` escaping `cwd` each produce a clear error
  naming the offending line — none panics.
- `grep -rn "Command::new\|process::Command" crates/adept/src/` stays
  empty; the whole suite still runs offline.

Unified surface:

- `adept eval <skill> --results r.jsonl` reports all four analyses in one
  `human` report and one `json` document.
- `adept eval <skill> --results r.jsonl --select evals` succeeds with **no
  model configured and no API key**, constructs no `LlmClient`, and makes
  no network call — asserted by a test that would fail if transport were
  constructed eagerly.
- `--select` / `--ignore` accept the four analysis names, reject unknown
  names with a clear error, and an explicitly selected analysis with a
  missing precondition exits `2` naming what is missing.
- Exit codes: `0` clean, `1` findings (any failed case, or any analysis
  reporting a problem, matching `score`'s current definition of a
  finding), `2` usage/I/O error.
- `adept score` is gone from `adept --help` and from shell completions;
  invoking it fails rather than running anything.
- `[eval]` is read from `adept.toml` with CLI-flag-wins precedence,
  covered by the existing config tests.
- An `adept.toml` containing `[score]` fails with exit `2` and a message
  naming `[eval]` — pinned by a test, since the current config parser
  would otherwise ignore it silently and silently drop the user's model
  and capture-dir settings.
- An MCP `tools/call` for `score_skill` returns an error naming
  `eval_skill`; `tools/list` advertises `eval_skill` and not
  `score_skill`.
- The MCP server exposes `eval_skill`, taking inline `results` plus
  optional `evals`/`select`/`ignore`, and its four analyses agree with the
  CLI's for the same inputs.
- `eval_skill` appears in `tools/list` **even with no model configured**
  (unlike `create_skill`/`generate_evals`, which keep their gate), and a
  grading-only `eval_skill` call succeeds in that state — pinned by a test
  with the LLM env vars unset.
- `eval_skill` with `content` instead of `path` grades `contains` and
  reports `file_exists`/`file_contains` as `skipped` naming the missing
  directory — not as passes, and not as an error.
- `eval_skill` writes nothing to the skill directory, and MCP stdout still
  carries only JSON-RPC.
- `docs/EVALS.md` documents the sidecar format, the per-assertion division
  of labour, and adept's new role as reference grader.
- `README.md`'s four-surface list becomes `check` / `fmt` / `eval` /
  `create` / `mcp`, with the `score` section rewritten as `eval` and the
  grading flags documented.

## Open questions

_None._

## Risks

- Wide mechanical diff (imports, docs, snapshot labels) — easy to lose a
  reference; the grep gates above are the guard.
- The rename breaks three public contracts at once (CLI, `adept.toml`, MCP
  tool names) with no shims. Anything already scripted against `adept
  score` or `[score]` breaks on upgrade — deliberately, and loudly. The
  `[score]`-rejection test is the one guarding the failure mode that would
  otherwise be silent.
- `docs/EVALS.md` states "adept never executes an eval dataset" as a
  standing promise. This task keeps that literally true while changing
  what adept does around it, so the doc's framing must be rewritten
  carefully rather than patched, or it will read as self-contradictory.
- The `results.jsonl` sidecar is a new published format, and formats are
  hard to change once a harness depends on one. Keeping it minimal and
  separately versioned is the mitigation.
- Referencing dataset cases by 1-indexed line number is simple but brittle
  if a harness reorders or filters cases. Stable case ids would mean
  bumping the dataset schema, which is out of scope; revisit if it bites.
- MCP tool advertisement stops being uniform: three network-backed tools
  share one `llm_configured` resolution today, and `eval_skill` now opts
  out of the gate while still using the resolution. Easy to "tidy" back
  into uniformity later and thereby hide the offline grading path again;
  the no-model `tools/list` test is what catches that.
- Conditional network access is a subtler invariant than "never" or
  "always", and subtle invariants rot. The offline-grading test is what
  keeps it honest.
- Two `EvalError` types and an `adept::evals` / `adept_agent::eval` pair
  are a legible-but-close naming space; a reviewer should check the
  re-exports don't accidentally shadow one with the other.
