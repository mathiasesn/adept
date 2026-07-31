# adept Architecture Documentation

> Generated: 2026-07-22 · Commit: `5a2ceb4` · Version: `0.1.0` (workspace-wide, unreleased)
> Re-read this file at the start of any session touching this codebase. Update it when the architecture changes (new major dependency, restructured layer, changed convention).

## 1. How to Read This Document

This is the architecture source of truth for `adept`. It is written for AI coding agents first and human contributors second: claims are concrete and traceable to real files, conventions are stated as rules to follow rather than described.

It is **not** a substitute for three documents that already exist and stay authoritative in their own domains:

| Document | Owns |
|---|---|
| `docs/MVP.md` | The originating spec: goals, non-goals, constraints, acceptance criteria. Do not re-litigate scope without reading it. |
| `docs/RULES.md` | The per-rule reference (`SL001`–`SL403`): what each rule flags and how to fix it. Machine-checked against the registry. |
| `docs/BACKLOG.md` | Known gaps and deliberate deferrals. Check here before "discovering" a bug. |

Sections 2–7 are universal; 8–10 cover the CLI and library surfaces; 11–14 cover the rule engine, MCP contract, testing conventions, and known divergences; 15 covers observability and secret handling. Section 16 is the short list of things not to violate.

**Update this file when**: a crate is added or a crate boundary moves, a new dependency with architectural weight lands, the rule-registration or config-precedence mechanism changes, or a hard invariant in section 16 changes.

## 2. Overview

`adept` is an extremely fast linter and formatter for **Agent Skills** — the folder-of-instructions pattern of a `SKILL.md` file (YAML frontmatter + Markdown body) plus optional companion files sitting beside it.

It exists because skills are proliferating with no tooling to check their quality. The defects it targets are the ones that make a skill fail to trigger, over-trigger, or bloat an agent's context: vague or overlong descriptions, malformed frontmatter, token-budget overruns, broken file references, and near-duplicate skills competing for the same requests.

The shape is modelled on ruff: stable rule codes, `path:line:col: CODE message` diagnostics, fix suggestions, CI-friendly exit codes, and a single static binary. Four surfaces ship from one binary:

- **`adept check`** — static, offline lint. No network, ever.
- **`adept fmt`** — prettier-style formatting: canonical frontmatter plus full Markdown reflow. Idempotent, atomic writes.
- **`adept eval`** — one evaluation surface, four analyses: LLM-assisted triggering accuracy, token bloat, and cross-skill overlap against any OpenAI-compatible endpoint (network-backed, run only when a model is configured), plus offline eval-dataset grading (`evals/evals.jsonl` graded against a harness-supplied `results.jsonl`, no network, no model required). `--select`/`--ignore` narrow which of the four run; see §8 and `docs/EVALS.md`.
- **`adept fix`** — LLM-assisted lint autofix for `FixKind::Llm` diagnostics (`SL206`, `SL301`, `SL302`). Preview-by-default; `--write` applies.
- **`adept create`** — LLM-assisted skill generation from a written brief: generate → screen → repair → generate-evals. Preview-by-default; `--write` applies. Also emits a synthetic eval dataset (`evals/evals.jsonl`) alongside the skill — see §9's `adept::evals` and `docs/EVALS.md`.
- **`adept mcp`** — a JSON-RPC 2.0 stdio MCP server exposing `check_skill`, `format_skill`, `eval_skill` (always advertised — grading needs no model), and the preview-only `create_skill`/`generate_evals` (conditionally advertised).

Architecturally it is a **cargo virtual workspace of four libraries and one binary**, with a strict dependency direction: everything depends on the core crate, nothing depends on the CLI, and `adept_agent` sits at the top of the stack, composing its siblings (see the amended rule in §16).

```
adept_cli  (bin: `adept`)
  ├── adept_agent ──┬── adept_fmt ──┐
  ├── adept_fmt ───────────────── ┤
  └── adept ◄──────────────────────┘   (core: data model, parser, diagnostics, rule engine, tokenizer, evals grader)
```

`adept_agent` is a rename of the crate formerly named `adept_fix`: it now houses `fix` (the original `adept fix` implementation) as a submodule, with `candidate`, `diff`, `prompts`, `writer`, and `gate` promoted to crate-level shared machinery reused by `create` (a sibling module implementing `adept create`). The rename kept every public item's name (`fix_skill`, `write_all_transactionally`, `FixKind`, `FixRegion`); only the crate and its module layout moved. `FixKind`/`FixRegion` still live in `adept`, untouched.

The former `adept_score` crate no longer exists: its LLM transport (`LlmClient`, `OpenAiCompatClient`, `MockLlmClient`, `CaptureSink`, `LlmConfig`) moved to `adept_agent::llm` and its four eval modules (`triggering`, `tokens`, `overlap`, `report`) moved to `adept_agent::eval`, both re-exported at `adept_agent`'s root. `adept_fmt` depends only on `adept` and knows nothing about `adept_agent`. `adept_agent` is the top-of-stack composing crate: it depends on `adept` (rules, tokens, the `evals` grader) and on `adept_fmt` (canonicalization), and owns its own LLM transport rather than depending on a sibling for it. Nothing in the library stack may depend on `adept_agent`; only `adept_cli` does.

## 3. Technology Stack

**Language**: Rust, edition 2021, `rust-version = "1.85"`. `rust-toolchain.toml` pins channel `stable` with `clippy` and `rustfmt` components — do not hardcode a different toolchain in CI or scripts.

Versions are declared once in the root `[workspace.dependencies]` and referenced by member crates as `{ workspace = true }`. **Add new shared dependencies to the workspace table, not to a member's `[dependencies]` with an inline version.**

| Crate | Version | Used for |
|---|---|---|
| `serde` / `serde_yaml` / `serde_json` | 1 / 0.9 / 1 | Frontmatter YAML, diagnostic and report JSON, JSON-RPC, LLM payloads |
| `thiserror` | 1 | All library error enums (`AdeptError`, `FmtError`, `EvalError` (two distinct types, in `adept::evals` and `adept_agent::eval`), `ConfigLoadError`) |
| `walkdir` | 2 | Skill discovery tree walk |
| `pulldown-cmark` | 0.12 | CommonMark event stream behind `adept::markdown` (lint rules + formatter AST) |
| `tiktoken-rs` | 0.12 | Token counting (`o200k_base` default, `cl100k_base` selectable) |
| `clap` | 4 (derive) | CLI argument parsing |
| `toml` | 0.8 | `adept.toml` config parsing (CLI only) |
| `reqwest` | 0.12 (json) | OpenAI-compatible HTTP client (`adept_agent::llm` only) |
| `tokio` | 1 (full) | Async runtime for `eval`/`fix`/`create`; the CLI builds it, `adept_agent` never does |
| `async-trait` | 0.1 | `LlmClient` trait object |
| `owo-colors` | 4 | Terminal color in diagnostic rendering |
| `similar` | 2 | Unified diff for `fmt --check` / `--diff` |
| `tracing` | 0.1 | Diagnostic events. In `adept_agent::llm` (the `send_once` funnel) — libraries emit, never subscribe |
| `tracing-subscriber` | 0.3 (env-filter) | **`adept_cli` only.** The one global subscriber, installed in `main` — see §16 |
| `jiff` | 0.2 | RFC 3339 timestamps and the timestamped capture folder name (`adept_agent::llm` capture) |
| `insta` / `criterion` / `proptest` / `assert_cmd` / `predicates` / `tempfile` | dev-only | Snapshots, benchmarks, property tests, CLI integration tests |
| `rustyline` | 14 | Interactive brief prompt for `adept create` when no `--from-file` and stdin is a TTY |

Notably **absent by design**: no `rmcp` MCP SDK (the JSON-RPC transport is hand-rolled — see §12), and no `rayon` (parallelism is a recorded deferral, see §14).

## 4. Project Structure

```
Cargo.toml                 Virtual workspace root; single source of dependency versions
rust-toolchain.toml        stable + clippy + rustfmt
.github/workflows/ci.yml   Build, test, clippy -D warnings, fmt --check, perf smoke test
docs/                      MVP.md (spec), RULES.md (rule reference), BACKLOG.md (gaps)

crates/adept/              CORE LIBRARY — no dependency on any sibling crate
  src/
    lib.rs                 Public re-export surface + `parse_skill()` convenience fn
    skill.rs               `Skill`: path, frontmatter, body, body_line_offset, source
    frontmatter.rs         `Frontmatter`, `ExtraField` — line-annotated for diagnostics
    parser.rs              `SkillParser` trait + `AnthropicSkillParser` (LF/CRLF tolerant)
    skillset.rs            `SkillSet::discover` — walkdir, excludes hidden/target/node_modules
    diagnostic.rs          `Diagnostic`, `Severity` — the shared lint finding type
    error.rs               `AdeptError` — hard failures, distinct from lint findings
    reporting.rs           Human (colored) and JSON diagnostic renderers
    token.rs               `TokenCounter`, `Tokenizer`, process-wide BPE table cache
    text.rs                `word_bag`, `words`, `jaccard` — shared similarity primitives
    companion.rs           `discover_companion_files` — shared by SL303 and adept_agent::eval's token-bloat view
    evals.rs               `Assertion`/`EvalCase`/`SCHEMA_VERSION` (dataset schema) plus the offline `grade` function
                           (`CaseResult`, `EvalBenchmarkReport`, `CaseReport`) — see docs/EVALS.md
    markdown/              THE SHARED MARKDOWN LEXER — one pulldown-cmark parser, two views
      mod.rs               `parser()` (the only Parser construction site), MAX_NESTING_DEPTH
      ast.rs               Block / Inline / ListItem / Alignment (span-free, for the formatter)
      build.rs             events → AST (`parse_document`)
      query.rs             positioned queries: headings / link_destinations / inline_code_spans
    rules/
      mod.rs               THE RULE ENGINE: Rule/SkillRule/SetRule, Registry, LintConfig, Linter
      frontmatter.rs       SL00x  structure.rs  SL1xx (SL101-SL105)  description.rs  SL2xx
      tokens.rs            SL3xx  cross.rs      SL4xx
  benches/lint_100_skills.rs   criterion benchmark gating the perf acceptance criterion
  tests/                   parsing, skillset, rules (insta snapshots), docs_test

crates/adept_fmt/          FORMATTER — depends on adept for parsing and the markdown AST
  src/
    lib.rs                 format_str / format_skill / check_str / check_skill
    config.rs              FmtConfig + marker enums (bullet, emphasis, strong, fence, heading)
    frontmatter.rs         Canonical YAML emission (hand-rolled scalar quoting)
    diff.rs                CheckResult + unified diff
    markdown/
      mod.rs               Re-exports adept::markdown's ast / parse_document
      print.rs             AST → canonical Markdown (the deterministic printer)

crates/adept_agent/        LLM-ASSISTED AGENT CAPABILITIES — depends on adept (rules, tokens,
                            evals grader) and adept_fmt (canonicalization); top-of-stack, nothing
                            in the library stack may depend on it — see the amended dependency
                            rule in §16
  src/
    lib.rs                 Crate-root re-exports (llm + eval transport/analyses, plus fix/create)
    llm/                   LLM TRANSPORT — the former adept_score crate's client stack, moved here
      mod.rs                 Re-exports the module's public surface
      client.rs              LlmClient trait, OpenAiCompatClient, LlmConfig/ResolvedLlmConfig
      capture.rs              CaptureSink, RunMetadata, CapturedCall — on-disk payload artifacts
      mock.rs                 MockLlmClient — the only client tests may use
    eval/                  THE FOUR ANALYSES — the former adept_score crate's scoring code
      mod.rs                 EvalOptions, EvalError, `eval_skill` (the single entry point)
      prompts.rs              All prompt templates + PROMPT_VERSION (still "adept_score-prompts-v1" —
                              deliberately unmoved so scores don't shift; see §10)
      triggering.rs           Prompt generation, judging, precision/recall/F1
      tokens.rs               Token-bloat analysis + LLM trimming suggestions
      overlap.rs              Offline Jaccard shortlist → LLM adjudication of shortlisted pairs
      report.rs               EvalReport + human renderer (triggering/token_bloat/overlaps/evals)
    candidate.rs            Shared candidate helpers, companion-path sandboxing (resolve_companion_path)
    prompts.rs              Shared prompt-building helpers + per-surface PROMPT_VERSION constants
                            (fix/create; distinct from eval/prompts.rs's PROMPT_VERSION)
    diff.rs                 Multi-file unified diff rendering
    writer.rs               write_all_transactionally — atomic multi-file apply
    gate.rs                 Shared accept/reject gate: passes_severity_gate, improves_on (fix and create)
    fix/                   `adept fix`'s own implementation (not shared with create)
      mod.rs                 FixError, FixReport, `fix_skill` (the single entry point)
      options.rs             FixOptions
      relocate.rs             The SL302 token-conservation guard (`conserves_content`)
    create/                `adept create`'s own implementation: generate -> screen -> repair -> generate-evals
      mod.rs                 CreateError, CreateOutcome, `create_skill` (the single entry point);
                             computes only, never writes — see §8 for the gate and round semantics
      candidate.rs            GenerateResponse / EvalGenerationResponse (model JSON)
      options.rs              CreateOptions, DEFAULT_MAX_ROUNDS, DEFAULT_EVAL_CASES

crates/adept_cli/          BINARY `adept` — composes all three libraries
  src/
    main.rs                Dispatch; owns process exit codes
    cli.rs                 clap derive structs; TokenizerArg mirror enum
    config.rs              adept.toml discovery (walk up) + parsing
    logging.rs             The one global tracing subscriber — stderr-only, ADEPT_LOG / -v
    commands/{check,fmt,eval,fix,create,mcp}.rs
  tests/{cli.rs, fixtures/}
```

**Organizing principle**: layered by capability, not by technical role. Each sibling crate owns one user-facing surface end to end and depends on the core crate for the shared data model. Anything two surfaces need moves *down* into `adept` (this is why `text.rs` and `companion.rs` exist), never sideways between siblings.

## 5. Core Architecture Principles

These are the principles the code actually follows. Violating one is a design change, not a style preference.

1. **The core crate is the only shared vocabulary.** `Skill`, `Diagnostic`, `AdeptError`, `TokenCounter` all live in `adept`. When `adept_fmt` and `adept_agent` both need a behaviour, it moves into `adept` so the two cannot drift. `text.rs` and `companion.rs` were both extracted for exactly this reason and their module docs say so.

2. **Lint findings and hard failures are different types.** A `Diagnostic` is something wrong with an otherwise-parseable skill. An `AdeptError` is an I/O or parse failure. Discovery never aborts on a bad skill: `SkillSet` keeps `skills` and `errors` side by side, and `Linter::lint_set` converts the errors into `SL001`/`SL002`/`SL003` diagnostics so a broken skill still reports rather than vanishing.

3. **Static checks are offline and fast.** `check`, `fmt`, and eval-dataset grading (`adept eval --select evals`) never touch the network. The `triggering`/`token-bloat`/`overlap` analyses of `adept eval` do, and only run when a model is configured. The perf criterion — 100 skills in under 1s, currently ~20ms — is gated in CI.

4. **Everything expensive is constructed once.** BPE tables are cached process-wide in `token.rs`; the MCP server holds its `Linter` in a `static OnceLock`. This is why `Rule` requires `Send + Sync`.

5. **Pluggability lives behind traits, at the two seams that matter**: `SkillParser` (skill formats) and `LlmClient` (model backends). Both exist because the spec named them. Do not add speculative traits elsewhere.

6. **Diagnostics carry precise, stable locations.** Frontmatter fields are line-annotated at parse time (`name_line`, `description_line`, `ExtraField::line`), and `Skill::body_line_offset` translates body-relative lines back to file lines. Rule codes are stable and never reused — a retired code (`SL202`) stays retired so old configs fail closed.

## 6. Build System & Toolchain

Cargo workspace, no wrapper scripts, no Makefile. These are the commands, verbatim from `README.md` and `.github/workflows/ci.yml`:

```bash
cargo build --workspace              # CI uses: cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check           # README shows the shorter `cargo fmt --check`
```

Install / run:

```bash
cargo install --path crates/adept_cli
cargo build --release -p adept_cli && ./target/release/adept --help
cargo run -q -p adept_cli -- check <path>
```

Benchmark:

```bash
cargo bench -p adept --bench lint_100_skills -- --quick
```

**CI** (`.github/workflows/ci.yml`, one `ci` job on ubuntu-latest, triggered on push to `main` and on all PRs) runs the four commands above in order, then a **performance smoke test**: it runs the criterion bench with `--quick`, greps the `lint_100_skills   time: [...]` line, converts the point estimate to milliseconds, and fails if it exceeds **500ms**. The threshold is deliberately ~25× the observed ~20ms so it catches an order-of-magnitude regression without flaking on noisy runners. The 1s figure in `docs/MVP.md` is the acceptance criterion; 500ms is the CI gate.

Note the benchmark itself asserts nothing — the gate is bash text-parsing of criterion output. This is a known brittleness, recorded in `docs/BACKLOG.md`.

`clippy -D warnings` is enforced, so **new code must be clippy-clean including in tests and benches** (`--all-targets`).

## 7. Configuration

Three layers, precedence **CLI flag > config file value > built-in default**.

**Config file**: `adept.toml`, discovered by walking *up* from the target path (`crates/adept_cli/src/config.rs::discover_config_file`, best-effort canonicalized first). `--config <path>` forces a specific file and skips discovery; a missing explicit `--config` path is a hard error (exit 2), while a missing discovered file silently falls back to defaults.

```toml
[lint]                        # deserializes directly into adept::LintConfig
disabled = ["SL206"]
description_min_tokens = 6
description_max_tokens = 75
body_max_tokens = 1500
tokenizer = "o200k_base"      # or "cl100k_base"

[fmt]                         # deserializes directly into adept_fmt::FmtConfig
line-width = 100

[eval]                        # EvalFileConfig, CLI-local (renamed from [score])
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"
capture_dir = ".adept-capture"  # off by default; gitignore it

# [fix] (FixFileConfig) and [create] (CreateFileConfig) take those same four keys,
# plus:
#   max_rounds = 2    both; falls back to adept_agent::DEFAULT_MAX_ROUNDS
#   eval_cases = 10   [create] only; falls back to create::DEFAULT_EVAL_CASES, no CLI flag
```

**Key casing is inconsistent across sections — verify against the struct, not this table.** `[lint]` uses `snake_case` (serde default on `LintConfig`); `[fmt]` uses `kebab-case` (`#[serde(rename_all = "kebab-case")]`); `[eval]`/`[fix]`/`[create]` (`EvalFileConfig`/`FixFileConfig`/`CreateFileConfig`) carry **no** `rename_all`, so their keys are plain Rust field names (`base_url`, not `base-url`).

Those three sections **never fall back to one another** — the only thing they share is the `ADEPT_*` environment variables below, and `capture_dir` follows the same independence (pinned by `config.rs::capture_dir_sections_do_not_cross_fall_back`). They are structurally near-identical; `docs/BACKLOG.md` records that a third such section was the condition it named for revisiting a `#[serde(flatten)]`ed shared `LlmFileConfig` — deliberately not done here.

**A config file with a stale `[score]` section (the pre-rename name) is a hard error**, not a silently-ignored table: `config.rs::contains_legacy_score_section` checks for it explicitly (rather than relying only on `deny_unknown_fields`, which the config structs deliberately don't have — see `docs/BACKLOG.md`) and fails with exit `2` naming the fix ("`[score]` is no longer read; rename it to `[eval]`"), because silently ignoring it would otherwise quietly drop the user's `model`/`capture_dir`/etc.

**Capture directory resolution** (`config.rs::resolve_capture_dir`, used by `eval` and `fix`). Precedence is the standard **`--capture-dir` flag > `adept.toml` `capture_dir` > built-in default (off)**, but the two layers anchor a *relative* path differently:

| Source | Relative path resolves against |
|---|---|
| `--capture-dir` | the process CWD (passed through untouched; the OS resolves it) |
| `[eval]`/`[fix]` `capture_dir` | the directory containing the `adept.toml` that supplied it |

The config anchor comes from `AdeptConfig::origin_dir`, which is `#[serde(skip)]` — it is stamped by the loader, never deserialized, so a hostile `adept.toml` cannot redirect writes by declaring `origin_dir` itself. With no value from either layer, capture is off and nothing is written.

**Environment variables** (`ADEPT_LOG` applies to every subcommand; the rest are LLM-backed commands only — `eval` and `fix` — resolved in `adept_agent::LlmConfig::resolve`):

| Var | Flag | Purpose |
|---|---|---|
| `ADEPT_MODEL` | `--model` | Model identifier. Required by the `triggering`/`token-bloat`/`overlap` analyses, which exit 2 without it; grading alone (`--select evals`) needs no model. Over MCP its absence fails those analyses per-call rather than hiding the tool — see §12. |
| `ADEPT_BASE_URL` | `--base-url` | Defaults to `https://api.openai.com/v1` |
| `ADEPT_API_KEY` | *(none)* | Bearer token, if the endpoint requires one. Never accepted as a flag. Held as a `RedactedString` — see §16. |
| `ADEPT_LOG` | `-v`/`-vv`/`-vvv` | `EnvFilter` directive syntax (e.g. `adept_agent::llm::client=trace`). Overrides the `-v` count wholesale rather than raising it. Adept's own namespace, not `RUST_LOG`. |

There are no compile-time feature flags. `LintConfig`'s numeric thresholds each carry a doc comment justifying the specific number — **keep that rationale attached if you change a default.**

## 8. Command Structure & Exit Codes

`main.rs` parses with clap, resolves config from the *first* target path, dispatches to `commands::<name>::run`, and calls `std::process::exit` with the returned code. Command functions return `i32` — they never exit or panic themselves.

Global flags: `--config <PATH>`, `--no-color`, `-q/--quiet`, `-v/--verbose` (repeatable; see §15). `--quiet` and `-v` are independent — `--quiet` trims *stdout* results, `-v` adds *stderr* diagnostics, so `-q -vv` is meaningful. Color is enabled only when `!no_color && stdout().is_terminal()`, computed once in `run()` and threaded down.

**Exit code contract** — this is a public API, do not change it:

| Code | Meaning |
|---|---|
| `0` | Clean (no diagnostics; or nothing would be reformatted; or `--exit-zero`) |
| `1` | Diagnostics found (`check`) / files would be reformatted (`fmt --check`, `fmt --diff`) |
| `2` | Usage or I/O error: bad path, unreadable file, bad config, unresolvable model, runtime failure |

Each command module re-declares its own `EXIT_*` consts. Conventions per command:

- **`check`**: `--format human|json`, `--select`/`--ignore` (comma-separated or repeated; accept either a code `SL201` or a kebab name `description-too-short`), `--statistics`, `--exit-zero`, `--tokenizer`. `--select` is implemented as "disable everything not named" on top of `LintConfig::disabled` — see `apply_select_ignore`.
- **`fmt`**: `--check` (diff + exit 1), `--diff` (diff, exit 0), `--line-width`. Writes are **atomic**: temp file `.{name}.adept-tmp` in the same directory, `sync_all`, then `rename`. A failed format never clobbers the original.
- **`eval`**: `path` (a `SKILL.md` file or a skill directory), `--format human|json`, `--model`/`--base-url` (resolved against `[eval]`), `--num-prompts`/`--seed`/`--judge-samples` (triggering), `--tokenizer`, `--capture-dir`, `--results <PATH>` (harness-produced `results.jsonl`), `--evals <PATH>` (override dataset discovery, default `evals/evals.jsonl` relative to the skill directory), `--select`/`--ignore` over the four analysis names `triggering`, `token-bloat`, `overlap`, `evals`. `evals` runs iff `--results` is passed; the other three run iff a model is configured; explicitly `--select`ing an analysis with a missing precondition is exit `2` naming what's missing, and nothing available at all is exit `2`. Unselected analyses are `null` in the JSON report, not merely empty. Builds its own `tokio::runtime::Runtime` and calls `block_on`, but only when at least one LLM-backed analysis is actually selected — `adept eval --select evals` constructs no `LlmClient` and makes no network call. `adept_agent` never creates a runtime itself.
- **`fix`**: LLM-assisted autofix for `FixKind::Llm` diagnostics. **Preview by default** — computes and prints a `FixReport` (rendered summary or unified diff via `--diff`) without touching disk; `--write` applies pending files via `adept_agent::write_all_transactionally`, `--check` exits `1` if any skill has pending changes, printing the same diff `--diff` would (matching `fmt --check`). `--select`/`--ignore` restrict which diagnostics are attempted; `--max-rounds` bounds the fix/re-lint loop (default `adept_agent::DEFAULT_MAX_ROUNDS = 2`). Also builds its own `tokio::runtime::Runtime`, same as `eval`.
- **`eval` and `fix` additionally take `--capture-dir`** (§15), resolved before any request is issued so a bad directory is exit 2, not a silent skip.
- **`create`**: generates a new skill from a brief taken from `--from-file`, non-TTY stdin, or an interactive multi-line prompt (in that precedence order); exits `2` if none is available. `--out <dir>` (default cwd), `--name` (override the derived skill name), `--write`/`-w` (preview by default), `--overwrite` (opt-in to writing into a directory that already has a `SKILL.md`), `--max-rounds`, `--model`/`--base-url`/`--tokenizer` resolved against `[create]`, `--capture-dir`, `--format human|json`. Computes a candidate via generate → screen → repair (gate: zero Error/zero Warning on the candidate, Info passing) then generates an eval dataset (`evals/evals.jsonl`) for it; a clean candidate exits `0`, one that exhausts `max_rounds` with findings remaining still writes/prints the best candidate and exits `1`. `2` covers usage/I/O errors, an unparseable LLM response, and refusing to clobber an existing skill directory.
- **`mcp`**: no flags; reads stdin until EOF.

**Output**: diagnostics and reports go to stdout; every error message goes to stderr prefixed `adept: error: `. The `--quiet` flag suppresses only summary/progress lines, never diagnostics.

## 9. Core Library Surface (`adept`)

Everything public is re-exported from `lib.rs`; there are no public submodules except `reporting`, `text`, `markdown`, and `evals`. Adding a type means adding it to that re-export list.

**`Skill`** is the data model: `path`, `frontmatter`, `body`, `body_line_offset`, and the complete unmodified `source`. Both `body` and `source` are retained (roughly 2× file bytes) — deliberate, since `fmt` compares against `source` byte-for-byte; halving it is a recorded deferral.

**`SkillParser`** is the format seam. `parse` has a default body that reads the file and delegates to `parse_str`, so implementors only write `parse_str`. `AnthropicSkillParser` splits on `\n` and strips a trailing `\r` per line, so LF and CRLF are handled uniformly; frontmatter is `---`-delimited, required `name`/`description`, optional `license`, everything else preserved in `Frontmatter::extra` (a `BTreeMap`, so `fmt` gets deterministic alphabetical ordering for free).

**`SkillSet::discover`** accepts a `SKILL.md` file, a skill directory, or a tree. It skips hidden directories and `target`/`node_modules`, but never excludes the root itself even if the root's own name matches.

**`TokenCounter`** holds a `&'static CoreBPE`. `token.rs::load_bpe` caches each encoding in a `OnceLock<Result<CoreBPE, String>>` — the `Result` is cached too, so a load failure is reported to every caller instead of being retried forever. `tiktoken-rs` ships `*_singleton()` helpers but they panic on failure; the hand-rolled cache exists to preserve the `Result` API. **Never construct BPE tables outside `load_bpe`.**

**`text.rs`** owns the definition of a "word" (lowercased, split on non-alphanumeric) and `jaccard`. `word_bag` and `words` share one tokenizer so the set-based and order-based callers cannot diverge.

**`markdown`** is the shared Markdown lexer, and the only markdown-aware code in the workspace. It exposes two views over the same document: `parse_document` builds the span-free `Block`/`Inline` AST the formatter re-prints, while `headings` / `link_destinations` / `inline_code_spans` return `Located<T>` values (carrying a 1-based body line) for the `SL1xx` rules, which need positions but no tree. Both views go through `markdown::parser()`, the **single** `pulldown-cmark` `Parser` construction site in the workspace — `grep -rn "Parser::new" crates/` must keep yielding exactly one hit. **Do not construct a `Parser` anywhere else**: an `Options` flag enabled for one view and not the other would let the linter and the formatter drift back into disagreeing about what a heading or a link is. Fence matching, info strings, indented code, nested brackets and reference links all come from the parser, so a markdown-aware rule never needs its own line scan.

**Known limitation**: `SKILL_FILE_NAME` is a private `const` in `skillset.rs`, not part of `SkillParser`. A parser for a format using `skill.yaml` or `AGENT.md` can therefore never be handed a file — the pluggability seam is incomplete. See `docs/BACKLOG.md`.

**`adept::evals`** is the published eval-dataset schema *and offline grader*: `Assertion` (`contains`/`file_exists`/`file_contains`/`command`, `#[serde(tag = "kind")]`), a versioned case object (`schema_version`, independent of any `PROMPT_VERSION`), `validate` (every line parses, `schema_version` is understood, non-empty), plus `parse_results_jsonl` (parses a harness-produced `results.jsonl`) and `grade(cases, results) -> EvalBenchmarkReport` — adept's reference grader for the dataset it defines. The schema half is documented as a contract in `docs/EVALS.md`, machine-checked against the code by a docs test the same way `docs/RULES.md` is checked against the rule registry. `grade` is purely offline and deterministic (substring match, filesystem reads under a harness-supplied `cwd`, a lookup into a harness-supplied exit-code map) — it never spawns a subprocess itself, so **adept still never *executes* an eval dataset**: it grades results a harness already produced. See `docs/EVALS.md` for the full division of labour and the `results.jsonl` sidecar format.

**`adept::companion::is_eval_dataset(skill_dir, path)`** matches by directory name only, and only the first path component beneath `skill_dir` (`<skill>/evals/...`) — never a deeper or unrelated `evals` component. Applied at `SL303` and in `adept_agent::eval`'s token-bloat view so a generated dataset is not counted as skill content. Currently **dormant**: `discover_companion_files` is non-recursive, so a nested `evals/evals.jsonl` is never discovered as a companion at all and the predicate cannot fire against real output today. Kept as defence-in-depth should discovery ever become recursive.

## 10. Formatter and Evaluation Surfaces

### `adept_fmt`

Pipeline: `Skill` → canonical frontmatter string → `adept::markdown::parse_document(&skill.body)` (the shared lexer's `Block`/`Inline` AST — see §9) → `markdown::print_document` → output. The formatter owns only the printer; the AST and its builder live in the core crate. Exactly one blank line separates the closing `---` from the body.

Frontmatter order is fixed: `name`, `description`, `license` (if present), then extras alphabetically. YAML quoting is minimal-but-correct, emitted by hand in `frontmatter.rs`.

Nesting of block quotes, lists, and footnote definitions is bounded by `adept::markdown::MAX_NESTING_DEPTH = 100`; anything deeper becomes `Block::Raw` and is passed through verbatim rather than recursed into. This is a stack-overflow guard against adversarial input — keep it.

Prefer `format_skill` / `check_skill` (already-parsed `Skill`) over `format_str` / `check_str` (re-parses) whenever a `Skill` is in hand. The CLI always has one.

Documented limitations, each visible to users as an unexpected diff: reference-style link *definitions* are inlined at each use; Setext headings always become ATX; tight-list preservation is partial; text escaping covers a conservative subset. `HeadingStyle` and `StrongMarker` are single-variant placeholders kept so config files can express intent without a future breaking change — do not "simplify" them away.

### `adept_agent::eval` (the `adept eval` command)

Everything is `async` and reached through `&dyn LlmClient` (`adept_agent::llm`, the former `adept_score` crate's transport). The crate **never creates a tokio runtime**; callers drive it.

`eval_skill` is the entry point for the three LLM-backed analyses, run per `EvalOptions`:

1. **Triggering accuracy** (`eval/triggering.rs`) — the LLM generates N candidate prompts (half positive, half negative, `DEFAULT_NUM_PROMPTS = 10`), then a judge model sees *only the skill's name and description* (never the body, mirroring real tool selection) and predicts whether it would trigger. Reports precision/recall/F1. `seed` and `judge_samples` (majority vote) exist to damp judge variance.
2. **Token bloat** (`eval/tokens.rs`) — description/body/companion counts via `adept::TokenCounter`, plus LLM trimming suggestions. Companion discovery is `pub use adept::discover_companion_files` — the same walk `SL303` uses, re-exported rather than reimplemented.
3. **Overlap** (`eval/overlap.rs`) — a cheap offline `jaccard` shortlist at `DEFAULT_SIMILARITY_THRESHOLD = 0.25` over name+description, then LLM adjudication of only the shortlisted pairs. Results are filtered to pairs involving the scored skill.

The fourth analysis, **eval-dataset grading**, needs no LLM at all: `adept_cli`'s `eval` command calls `adept::evals::grade` directly (§9) and folds the resulting `EvalBenchmarkReport` into `EvalReport::evals`. This is what makes `adept eval --select evals` construct no `LlmClient` and touch no network.

Construct options via **`EvalOptions::for_model(model, tokenizer)`**, not a struct literal. The model name has to reach both `EvalOptions::model` and `TriggeringOptions::model`; that constructor is the one place that wiring lives, so the CLI and MCP tool cannot drift on defaults.

All LLM-analysis prompt templates live in `adept_agent::eval::prompts` and are stamped into every report as `PROMPT_VERSION`. **Its value is deliberately still `"adept_score-prompts-v1"`** — unmoved by the crate merge so old and new reports compare identically; bump it only when a template's wording changes in a way that could shift scores. This is a distinct constant from `adept_agent::prompts`'s per-surface `PROMPT_VERSION`s (fix/create), which live at the crate root.

**Deliberate divergence**: `adept_agent::eval`'s overlap shortlist uses name+description at 0.25 (tuned for recall — it only shortlists); `SL402` uses description-only at 0.6 (tuned for precision — it emits a diagnostic). Both call `adept::text::jaccard`. `check` and `eval` can therefore reach different conclusions about the same pair; this is intended, not a bug.

**`EvalReport`** (`adept_agent::eval::report`) carries all four analyses as independent optional fields (`triggering`, `token_bloat`, `overlaps`, `evals`) — an analysis that wasn't selected is absent (`null` in JSON), distinguishable from one that ran and found nothing. Two distinct `EvalError` types exist and must not be confused: `adept::evals::EvalError` (dataset/results parse failures, in core) and `adept_agent::eval::EvalError` (transport/JSON failures from the LLM analyses).

## 11. Rule Engine & Extension Points

`crates/adept/src/rules/mod.rs` is the file to read before touching rules.

**Three traits.** `Rule` carries identity — `code()`, `name()`, `default_severity()` — and requires `Send + Sync`. Rules are stateless unit structs, so this costs nothing and is what allows a `Registry` (hence a `Linter`) to live in a `static`. `SkillRule: Rule` checks one `Skill`; `SetRule: Rule` checks a whole `SkillSet`. Both `check` methods receive `&LintConfig` and a shared `&TokenCounter`.

**Adding a rule** — all four steps are required:

1. Add the unit struct and its `check` impl to the taxonomy file matching its code: `SL00x` → `rules/frontmatter.rs`, `SL1xx` → `structure.rs`, `SL2xx` → `description.rs`, `SL3xx` → `tokens.rs`, `SL4xx` → `cross.rs`.
2. Declare its identity with the macro, on one line next to the struct: `impl_rule!(MyRule, "SL107", "my-rule", Warning);`. Do not hand-write the three-method `impl Rule` block; import `impl_rule` from `super`.
3. Register it in `Registry::new`, in the appropriate `vec![]` literal, in code order.
4. Document it in `docs/RULES.md`. **`crates/adept/tests/docs_test.rs` fails the build if a registered code is undocumented** — this is a hard gate, not a nicety.

Then add a fixture under `crates/adept/tests/fixtures/rules/` and an insta snapshot test (§13).

**Opting a rule into LLM fixing.** `Rule::fix_kind()` defaults to `FixKind::None`; a rule signals it is safe for `adept fix` to attempt — and which part of the skill its diagnostics are about — by passing `Llm` plus a region to `impl_rule!`: `impl_rule!(MyRule, "SL107", "my-rule", Warning, Llm, Description);` (region is `Description` or `Body`; see `tokens::BodyTokenBudget`/`SL302` → `Body`, `tokens::DescriptionTokenBudget`/`SL301` → `Description`, `description::NoNegativeGuidance`/`SL206` → `Description`). This expands to `FixKind::Llm(FixRegion::Description)` etc. This is metadata only — `adept` itself never fixes anything; setting `FixKind::Llm(_)` just makes the rule visible to `adept_agent::fixable()`, and its `FixRegion` tells `adept_agent` which batched request (description-scope or body-scope) to route the diagnostic into — `adept_agent` hard-codes no rule-code list of its own. Tagging a new rule `FixKind::Llm` is therefore immediately sufficient to make `adept fix` attempt it; a `crates/adept/src/rules/mod.rs` unit test (`only_expected_rules_are_tagged_llm_fixable`) pins the current `Llm`-tagged set and each one's region so the two don't silently drift apart. `FixKind::Deterministic` exists for a future non-LLM autofixer, carries no region, and nothing currently returns it. Only `SkillRule` diagnostics are ever attempted by `adept_agent`; `SetRule` (cross-skill) findings are reported by `adept check` but never auto-rewritten (see `crates/adept_agent/src/lib.rs` module docs and `docs/BACKLOG.md`).

**Severity and enablement** are applied by the `Linter`, never by the rule. Rules build diagnostics with their `default_severity()`; `LintConfig::apply_overrides` rewrites it afterwards. Both `disabled` and `severity_overrides` accept either the code or the kebab-case name.

**Rule codes are permanent.** `SL202` is retired (it duplicated `SL301` exactly) and its code is never reused, so an old config naming it fails closed rather than silently acquiring a new meaning. `docs/RULES.md` documents retired codes explicitly.

**`SL003` is special.** A skill with malformed frontmatter has no `Skill` to run rules against, so `SL001`/`SL002`/`SL003` are synthesized by `parse_error_diagnostic`, a `match` over `AdeptError` in `Linter::lint_set`. Its metadata lives in the module-level `const PARSE_ERROR_META` so it is still listed, documented, and disableable like any other rule. The consequence: that `match` is closed over `AdeptError`, so a custom `SkillParser` has no way to contribute its own error codes. A third rule flavour (`ParseErrorRule`) is the recorded fix.

**Output ordering** is `(path, line, column, code)`, implemented once in the public `adept::sort_diagnostics`. It is the user-visible ordering contract — call it, never re-implement the comparator at a call site.

## 12. MCP Server Contract

`crates/adept_cli/src/commands/mcp.rs`. Hand-rolled JSON-RPC 2.0 over newline-delimited stdio, protocol version `2024-11-05`. The `rmcp` SDK was deliberately not used: a direct implementation keeps both the dependency footprint and the risk of an SDK writing to stdout on our behalf at zero.

Five tools: `check_skill`, `format_skill` and `eval_skill` are advertised unconditionally; `create_skill` and `generate_evals` only when a model resolves. **The latter two are preview-only and never touch the filesystem** — they return the generated skill/dataset as data, mirroring why capture is CLI-only (§15): an MCP client must not be able to make the server write to arbitrary paths. Writing stays a CLI capability (`adept create --write`). **`eval_skill` is read-only** (it grades and reports; nothing it does writes to the skill directory) but not preview-only in the same sense — there is no "real" write it is previewing.

**The hard invariant: stdout carries only JSON-RPC response messages. Everything else goes to stderr.** `serve()` is the only function that writes to stdout. `handle_message` is pure with respect to I/O — it never touches stdin, stdout, or stderr — which is what lets tests drive it directly without spawning the binary. Any `println!` added below `serve` breaks every MCP client silently. Use `eprintln!`.

Other contract points:

- The `Linter` is built once into a `static OnceLock<Result<Linter, String>>`. `Linter::new` loads the tiktoken BPE tables, which costs far more than the lint itself; rebuilding it per tool call is not acceptable.
- `eval_skill` **opts out of the advertisement gate** the other two LLM tools apply (`LlmConfig::default().resolve().is_ok()`), because grading needs no model — gating it would hide a tool that works with no `ADEPT_MODEL` set. Preconditions are instead enforced per-analysis, exactly as the CLI's `--select`/`--ignore`: an explicitly selected analysis with a missing precondition (no model, or no `results`) is a structured tool error naming what's missing, never a silent skip. It takes `results` as an inline JSON array — not a path, since an MCP client may not share a filesystem with the server — plus optional `evals`/`select`/`ignore`. `results` alongside raw `content` (no `path`) grades `contains` only and reports `file_exists`/`file_contains` as *skipped*, naming the missing directory; that is not an error. LLM-backed analyses are bounded by a 30s timeout; grading needs none, making no network call.
- `format_skill`'s `line_width` argument is validated to `MIN_LINE_WIDTH..=MAX_LINE_WIDTH` (20..=500); out-of-range or zero values return a structured tool error instead of producing degenerate one-word-per-line output.
- `create_skill`'s `max_rounds` (1..=10) and both tools' `eval_cases` (1..=50) are bounded the same way: out-of-range values are rejected with a structured tool error, never clamped — an MCP client talks to this server over public JSON-RPC with no other gate on LLM spend, so every numeric argument driving LLM calls needs an explicit bound.
- Notifications (messages with no `id`, e.g. `notifications/initialized`) return `None` and produce no output line.

**Overlap detection over MCP uses real siblings.** `overlap_skillset` in `mcp.rs` discovers them via an optional `directory` argument, or `adept::sibling_root(path)` when a real on-disk `path` is given, mirroring the `adept eval` CLI. Passing just `[skill]` as the skillset makes the analysis inert — it can only find overlaps against skills it can see.

## 13. Testing & Snapshot Conventions

336 tests across 19 suites (16 unit/integration + 3 doc-test suites), a few seconds for the full workspace run. Tests live beside the code (`#[cfg(test)] mod tests`) for unit-level behaviour and in `tests/` for integration.

**Rule tests are snapshot tests.** Each rule gets a fixture directory under `crates/adept/tests/fixtures/rules/<sl_code>_<slug>/` containing a `SKILL.md` that triggers exactly that rule, plus an `insta` snapshot under `crates/adept/tests/snapshots/rules__snapshot_<name>.snap`. There is also a `cross_clean` fixture asserting a well-formed pair produces nothing. Review snapshot diffs — an accepted snapshot is an accepted behaviour change.

**Formatter tests** are fixture-in / snapshot-out (`crates/adept_fmt/tests/fixtures/*.md` → `snapshots/format_tests__*.snap`), one file per construct family (headings, tables, nested lists, code fences, blockquotes, HTML blocks, long prose, links + inline code, already-formatted). Plus `proptest_idempotency.rs` asserting `fmt(fmt(x)) == fmt(x)`. Note the proptest currently excludes tables, fences, HTML, and emphasis, and the broad idempotency loop is gated on `reflow_prose: false` — **prose reflow is the least-covered high-risk path in the repo.**

**No test may perform network I/O.** `adept_agent::llm`/`eval` tests use `MockLlmClient` (`with_texts` seeds a FIFO queue of scripted responses); the module doc states this as a rule. `adept_cli`'s eval test drives `run_with_client`, which exists solely as a `#[cfg(test)]` seam taking `&dyn LlmClient`. Eval-dataset grading (`adept::evals::grade`) needs no client at all and is tested with no model configured and no network access, pinning the claim that `--select evals` never constructs an `LlmClient`.

**CLI tests** use `assert_cmd` + `predicates` against `tests/fixtures/{clean-skill,defective-skill}`. `tests/tracing.rs` is the guard for §15: it drives the real binary under `-vvv` and `ADEPT_LOG=trace` and asserts MCP stdout stays pure JSON-RPC and byte-identical, that `check`/`fmt --check` output is unaffected by the logging layer (and writes nothing to stderr by default), and that an uncreatable `--capture-dir` exits 2. It resolves the capture sink before any request is issued, so it needs no network.

**`docs_test.rs`** asserts every registered rule code appears in `docs/RULES.md`. If you add a rule and skip the docs, CI fails.

**Benchmark**: `crates/adept/benches/lint_100_skills.rs`, criterion, `harness = false`. Currently ~20ms. See §6 for how CI gates it.

Not covered: no fixture exercises a real skills corpus. The "runs clean-ish on anthropics/skills" acceptance criterion is verified manually by cloning the repo and running `check` against it; a vendored mini-corpus would make it a test.

## 14. Known Divergences & Deferrals

Read `docs/BACKLOG.md` for the full list. The two worth knowing before you write code:

**`SL104`'s heuristic filters are not lexing.** The URL-scheme, glob, `~`, `@scope/name` and template-placeholder filters in `rules/structure.rs` are genuine domain judgements about what a repo-relative path looks like, with one consumer. They sit *above* the shared lexer (§9) and should stay hand-written — do not try to push them into `adept::markdown`.

**Performance work is deliberately not done**, since `check` runs ~20ms against a 1s target: skill discovery and the per-skill lint loop are sequential and embarrassingly parallel (`rayon` would cut wall time by roughly Ncores); `SL402`/`SL403` are each O(n²) pairwise Jaccard, fine at 100 skills but 500k pairs at 1000; hashing words to `u64` would remove the per-word `String` allocation; `Skill` retaining both `source` and `body` costs ~2× file bytes. Do not treat any of these as bugs.

## 15. Observability, Capture & Secret Handling

Two independent layers, added by `specs/cli-tracing.md`. Both exist to make an LLM run reproducible after the fact; neither is on by default, and neither changes a byte of output when off.

### Layer 1 — `tracing` to stderr

`crates/adept_cli/src/logging.rs::init` installs the **one** global subscriber, from `main`, before subcommand dispatch — every subcommand including `mcp`. Level comes from the global `-v` count (`ArgAction::Count`: `0` off, `1` info, `2` debug, `3+` trace), scoped to adept's own crate targets so `-vvv` does not drown the user in `hyper`/`reqwest` internals. `ADEPT_LOG` replaces that filter wholesale with `EnvFilter` directive syntax.

**The writer is `std::io::stderr`, explicitly, with `.with_ansi(false)`.** This is the load-bearing line in the file: `tracing_subscriber::fmt()` defaults to **stdout**, and `adept mcp` speaks JSON-RPC on stdout, so a subscriber that lands there breaks every MCP client silently — no error, no diagnostic, just a client that stops parsing. Do not drop `.with_writer`, and do not add a second subscriber anywhere. `crates/adept_cli/tests/tracing.rs` drives the real binary under `-vvv` and `ADEPT_LOG=trace` and asserts MCP stdout is unchanged byte-for-byte and every line still parses as JSON-RPC.

**Libraries emit, they never subscribe.** `adept_agent::llm::client::send_once` — the single funnel for both `eval` and `fix` — emits the serialized request body and the raw response text at `DEBUG`/`TRACE`, before parsing, so a body that fails `parse_chat_response` is still on the wire in the log. No library crate calls `try_init`.

Default level is `off`, so with no `-v` and no `ADEPT_LOG`, stdout *and* stderr are identical to a build with no logging at all — also pinned by test.

### Layer 2 — `--capture-dir`

`adept_agent::llm::CaptureSink` writes verbatim per-call artifacts, in the shape `<capture-dir>/<timestamp>/{run_metadata.json, call_NNNN/{request.json, response.json, call_metadata.json}}`. Bodies are written **at the moment of receipt, before parsing, never truncated**, and non-2xx responses are captured too — an error body is exactly the evidence a live-endpoint validation needs. Each invocation gets its own timestamped folder, so a capture dir is only ever appended to; re-running never overwrites a previous run.

The sink is opt-in via `OpenAiCompatClient::with_capture(Arc<CaptureSink>)`. `eval`/`fix` resolve it **before** any request is issued, so a directory that cannot be created is a usage error (exit 2, `adept: error: failed to create capture directory ...`) rather than a silent skip. `run_metadata.json` is self-describing by design: adept version, `PROMPT_VERSION`, subcommand, resolved options, the *source* of each resolved value (flag / `adept.toml` / env / default), timestamps, target path, and the process exit code stamped by `finalize`.

**Capture is CLI-only and never reachable from MCP.** `--capture-dir` / `capture_dir` exist on `eval` and `fix` alone; the MCP `eval_skill` tool never captures and the MCP tool schema is unchanged. MCP gets stderr tracing only. This keeps capture off the public JSON-RPC contract. Capture artifacts contain full prompts and full model output — steer users to a gitignored path.

### Secret handling

The API key is wrapped in `adept_agent::llm::RedactedString` and that newtype is **adopted at the `ResolvedLlmConfig::api_key` field**, not merely defined. Its `Debug` *and* `Display` impls both render `****`, so `{:?}` on a resolved config — the realistic leak, since a tracing event or a panic message will happily format a whole struct — cannot expose it. Never add a field, accessor, or log line that renders the key by another route, and never derive `Serialize` onto a struct holding one.

The precise invariant is therefore: **once configuration is resolved, the key exists only inside a `RedactedString`, and `expose_secret()` is the sole way back out.** The deliberate carve-out is the *input* side — `LlmConfig::api_key` is a plain `String` in a `#[derive(Debug)]` struct, because it is assembled from CLI flags and config files before anything can be redacted. `LlmConfig::resolve` is the single place the wrap happens; that keeps the unredacted type confined to pre-resolution config plumbing, which is never logged or captured. Do not widen that carve-out by logging or serializing an unresolved `LlmConfig`.

Three rules follow and are not negotiable: the `Authorization` header is **omitted entirely** from captured request metadata (not masked, not present — `request_header_map` strips it from the real outgoing headers); `run_metadata.json` records only `api_key_present: bool`; and every request and response body passes through `OpenAiCompatClient::scrub` before it is logged or written, replacing any verbatim occurrence of the resolved key with `****`. The scrub is a defensive backstop, not the primary defence — no body carries the key today — and is deliberately an exact-substring match rather than a secret-shaped-string heuristic. Unit tests assert a configured key appears in no emitted tracing output and no capture artifact, including when smuggled into a prompt body.

## 16. Summary & Key Architectural Decisions

The things not to violate:

- **Dependency direction is one-way: `adept_fmt` depends only on `adept`.** `adept_agent` is the top-of-stack composing crate: it depends on `adept` (rules, tokens, the `evals` grader) and on `adept_fmt` (reuses `format_skill` to canonicalize every candidate before re-linting/diffing it), and owns its own LLM transport (`adept_agent::llm`, the former `adept_score` crate) rather than depending on a sibling for it — the old "one deliberate exception" clause no longer applies, since there is no longer a sibling transport crate to depend on. **Nothing in the library stack may depend on `adept_agent`**; only `adept_cli` does. Do not add a sideways dependency between `adept_fmt` and `adept_agent`'s siblings.
- **`check`, `fmt`, and eval-dataset grading never touch the network.** The `triggering`, `token-bloat`, and `overlap` analyses of `adept eval` do, and they run only when a model is configured; `adept eval --select evals` makes no network call and requires no API key.
- **MCP stdout carries only JSON-RPC.** All logging goes to stderr; `handle_message` stays I/O-pure. `create_skill`/`generate_evals` are preview-only and never write to disk; `eval_skill` is read-only and never writes to disk either, and is always advertised regardless of whether a model is configured.
- **adept spawns no subprocess, ever.** Not even the `command` eval assertion `create` can generate — adept defines and validates that vocabulary but never executes it; `adept::evals::grade` grades a `command` assertion only from a harness-supplied exit code, never by running the command itself.
- **The tracing writer is stderr, never stdout.** `tracing_subscriber::fmt()` defaults to stdout; `logging.rs` overrides that with an explicit `.with_writer(std::io::stderr)`. A subscriber that lands on stdout breaks every MCP client silently. Only `main` installs one — libraries emit events and never subscribe. See §15.
- **The API key never leaves `RedactedString`.** `Debug` and `Display` both render `****`, and the newtype is adopted at `ResolvedLlmConfig::api_key`. `Authorization` is omitted entirely from captured metadata; `run_metadata.json` carries only `api_key_present: bool`.
- **Exit codes are a public contract**: `0` clean, `1` findings, `2` usage/I/O error.
- **Rule codes are permanent and never reused.** Retired codes stay retired and documented.
- **Every registered rule must appear in `docs/RULES.md`** — `docs_test.rs` enforces it.
- **Use `impl_rule!` for a rule's identity, register it in `Registry::new`, and let the `Linter` apply severity.** Rules never resolve their own enablement or severity.
- **Construct `EvalOptions` via `for_model`** (`adept_agent::EvalOptions::for_model`), and bump `PROMPT_VERSION` in `adept_agent::eval::prompts` when prompt wording changes meaningfully.
- **Never load BPE tables outside `token.rs::load_bpe`**; build expensive objects once (`static OnceLock`), which is why `Rule: Send + Sync`.
- **`fmt` writes atomically** (temp file + rename) and is idempotent. Both properties are tested; keep them.
- **`cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass**, tests and benches included.
