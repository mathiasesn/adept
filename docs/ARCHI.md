# adept Architecture Documentation

> Generated: 2026-07-22 · Commit: `5a2ceb4` · Version: `0.1.0` (workspace-wide, unreleased)
> Re-read this at the start of any session touching this codebase. Update it when the architecture changes.

## 1. How to Read This Document

The architecture source of truth for `adept`, written for AI coding agents
first: claims are concrete and traceable to real files, conventions are stated
as rules to follow rather than described.

It does **not** override these, authoritative in their own domains:

| Document | Owns |
|---|---|
| `docs/RULES.md` | The per-rule reference (`SL001`–`SL403`). Machine-checked against the registry. |
| `docs/EVALS.md` | The eval-dataset and `results.jsonl` contract. Machine-checked against `adept::evals`. |
| `docs/BACKLOG.md` | Known gaps and deliberate deferrals. Check here before "discovering" a bug. |

**Update this file when**: a crate is added or a boundary moves, a dependency
with architectural weight lands, the rule-registration or config-precedence
mechanism changes, or a §16 invariant changes.

## 2. Overview

`adept` is a fast linter and formatter for **Agent Skills** — a `SKILL.md`
(YAML frontmatter + Markdown body) plus optional companion files beside it.

It targets the defects that make a skill fail to trigger, over-trigger, or
bloat an agent's context: vague or overlong descriptions, malformed
frontmatter, token-budget overruns, broken file references, and near-duplicate
skills competing for the same requests.

The shape is modelled on ruff: stable rule codes, `path:line:col: CODE message`
diagnostics, fix suggestions, CI-friendly exit codes, one static binary. Six
surfaces ship from that binary:

- **`adept check`** — static, offline lint. No network, ever.
- **`adept fmt`** — canonical frontmatter plus full Markdown reflow. Idempotent, atomic writes.
- **`adept eval`** — one surface, four analyses: LLM-assisted triggering accuracy, token bloat, and cross-skill overlap (network-backed, only when a model is configured), plus offline eval-dataset grading (`evals/evals.jsonl` against a harness-supplied `results.jsonl` — no network, no model). `--select`/`--ignore` narrow which run; see §8 and `docs/EVALS.md`.
- **`adept fix`** — LLM-assisted autofix for `FixKind::Llm` diagnostics (`SL206`, `SL301`, `SL302`). Preview-by-default; `--write` applies.
- **`adept create`** — LLM-assisted skill generation from a brief: generate → screen → repair → generate-evals. Preview-by-default. Also emits `evals/evals.jsonl` alongside the skill.
- **`adept mcp`** — JSON-RPC 2.0 stdio MCP server.

Architecturally it is a **cargo virtual workspace of four libraries and one
binary**:

```
adept_cli  (bin: `adept`)
  ├── adept_agent ──┬── adept_fmt ──┐
  ├── adept_fmt ───────────────── ┤
  └── adept ◄──────────────────────┘   (core: data model, parser, diagnostics, rule engine, tokenizer, evals grader)
```

`adept_fmt` depends only on `adept`. `adept_agent` is the top-of-stack
composing crate — it depends on `adept` (rules, tokens, the `evals` grader) and
`adept_fmt` (canonicalization), and owns its own LLM transport rather than
depending on a sibling for it. **Nothing in the library stack may depend on
`adept_agent`; only `adept_cli` does.**

Two historical renames explain names you will find in the code. `adept_agent`
was formerly `adept_fix` (`fix` is now a submodule; `candidate`/`diff`/
`prompts`/`writer`/`gate` were promoted to crate-level machinery shared with
`create`). The former `adept_score` crate is gone: its transport became
`adept_agent::llm` and its four scoring modules became `adept_agent::eval`,
both re-exported at the crate root. Both renames kept every public item's name.

## 3. Technology Stack

**Rust**, edition 2021, `rust-version = "1.85"`. `rust-toolchain.toml` pins
channel `stable` with `clippy` and `rustfmt` — do not hardcode a different
toolchain in CI or scripts.

Versions are declared once in the root `[workspace.dependencies]` and
referenced as `{ workspace = true }`. **Add new shared dependencies to the
workspace table, not to a member's `[dependencies]` with an inline version.**

| Crate | Version | Used for |
|---|---|---|
| `serde` / `serde_yaml` / `serde_json` | 1 / 0.9 / 1 | Frontmatter YAML, diagnostic and report JSON, JSON-RPC, LLM payloads |
| `thiserror` | 1 | Library error enums (`AdeptError`, `FmtError`, `EvalError` ×2, `ConfigLoadError`) |
| `walkdir` | 2 | Skill discovery tree walk |
| `pulldown-cmark` | 0.12 | CommonMark event stream behind `adept::markdown` |
| `tiktoken-rs` | 0.12 | Token counting (`o200k_base` default, `cl100k_base` selectable) |
| `clap` | 4 (derive) | CLI argument parsing |
| `toml` | 0.8 | `adept.toml` parsing (CLI only) |
| `reqwest` | 0.12 (json) | OpenAI-compatible HTTP client (`adept_agent::llm` only) |
| `tokio` | 1 (full) | Async runtime for `eval`/`fix`/`create`; the CLI builds it, `adept_agent` never does |
| `async-trait` | 0.1 | `LlmClient` trait object |
| `owo-colors` | 4 | Terminal color in diagnostic rendering |
| `similar` | 2 | Unified diff for `fmt --check` / `--diff` |
| `tracing` | 0.1 | Diagnostic events — libraries emit, never subscribe |
| `tracing-subscriber` | 0.3 | **`adept_cli` only.** The one global subscriber, installed in `main` — see §15 |
| `jiff` | 0.2 | RFC 3339 timestamps and capture folder names |
| `rustyline` | 14 | Interactive brief prompt for `adept create` |
| `insta` / `criterion` / `proptest` / `assert_cmd` / `predicates` / `tempfile` | dev-only | Snapshots, benchmarks, property tests, CLI tests |

**Absent by design**: no `rmcp` MCP SDK (the JSON-RPC transport is hand-rolled
— §12), and no `rayon` (§14).

## 4. Project Structure

```
Cargo.toml                 Virtual workspace root; single source of dependency versions
rust-toolchain.toml        stable + clippy + rustfmt
pyproject.toml             maturin `bindings = "bin"` — packs crates/adept_cli into a wheel (§6)
python/adept/              Binary-discovery + `python -m adept` dispatch shim, no API surface
.github/workflows/ci.yml   ci job (build/test/clippy/fmt/perf) + python-packaging job (§6)
.github/workflows/release.yml  release-plz: release + release-pr jobs (§6)
release-plz.toml           release-plz config: shared version_group
docs/                      RULES.md, EVALS.md, BACKLOG.md

crates/adept/              CORE LIBRARY — no dependency on any sibling crate
  src/
    lib.rs                 Public re-export surface + `parse_skill()`
    skill.rs               `Skill`: path, frontmatter, body, body_line_offset, source
    frontmatter.rs         `Frontmatter`, `ExtraField` — line-annotated for diagnostics
    parser.rs              `SkillParser` trait + `AnthropicSkillParser` (LF/CRLF tolerant)
    skillset.rs            `SkillSet::discover` — walkdir, excludes hidden/target/node_modules
    diagnostic.rs          `Diagnostic`, `Severity`
    error.rs               `AdeptError` — hard failures, distinct from lint findings
    reporting.rs           Human (colored) and JSON renderers
    token.rs               `TokenCounter`, `Tokenizer`, process-wide BPE cache
    text.rs                `word_bag`, `words`, `jaccard` — shared similarity primitives
    companion.rs           `discover_companion_files` — shared by SL303 and eval's token-bloat view
    evals.rs               Dataset schema (`Assertion`/`EvalCase`/`SCHEMA_VERSION`) + offline `grade`
    markdown/              THE SHARED MARKDOWN LEXER — one pulldown-cmark parser, two views
      mod.rs               `parser()` (the only Parser construction site), MAX_NESTING_DEPTH
      ast.rs               Block / Inline / ListItem / Alignment (span-free, for the formatter)
      build.rs             events → AST (`parse_document`)
      query.rs             positioned queries: headings / link_destinations / inline_code_spans
    rules/
      mod.rs               THE RULE ENGINE: Rule/SkillRule/SetRule, Registry, LintConfig, Linter
      frontmatter.rs SL00x   structure.rs SL1xx   description.rs SL2xx
      tokens.rs      SL3xx   cross.rs     SL4xx
  benches/lint_100_skills.rs   criterion benchmark gating the perf criterion
  tests/                   parsing, skillset, rules (insta snapshots), docs_test

crates/adept_fmt/          FORMATTER — depends on adept only
  src/
    lib.rs                 format_str / format_skill / check_str / check_skill
    config.rs              FmtConfig + marker enums (bullet, emphasis, strong, fence, heading)
    frontmatter.rs         Canonical YAML emission (hand-rolled scalar quoting)
    diff.rs                CheckResult + unified diff
    markdown/{mod,print}.rs  Re-exports adept's AST; AST → canonical Markdown printer

crates/adept_agent/        LLM-ASSISTED CAPABILITIES — top-of-stack (§2)
  src/
    lib.rs                 Crate-root re-exports (llm + eval, plus fix/create)
    llm/                   LLM TRANSPORT
      client.rs              LlmClient trait, OpenAiCompatClient, LlmConfig/ResolvedLlmConfig
      capture.rs             CaptureSink, RunMetadata, CapturedCall — on-disk artifacts
      mock.rs                MockLlmClient — the only client tests may use
    eval/                  THE FOUR ANALYSES
      mod.rs                 EvalOptions, EvalError, `eval_skill` (single entry point)
      prompts.rs             Templates + PROMPT_VERSION (still "adept_score-prompts-v1" — §10)
      triggering.rs          Prompt generation, judging, precision/recall/F1
      tokens.rs              Token-bloat analysis + LLM trimming suggestions
      overlap.rs             Offline Jaccard shortlist → LLM adjudication
      report.rs              EvalReport + human renderer
    candidate.rs           Shared candidate helpers, companion-path sandboxing
    prompts.rs             Shared prompt builders + per-surface PROMPT_VERSIONs (fix/create)
    diff.rs                Multi-file unified diff rendering
    writer.rs              write_all_transactionally — atomic multi-file apply
    gate.rs                Shared accept/reject gate: passes_severity_gate, improves_on
    fix/                   `adept fix`: FixError, FixReport, `fix_skill`, FixOptions,
                             relocate.rs (the SL302 token-conservation guard)
    create/                `adept create`: generate → screen → repair → generate-evals.
                             Computes only, never writes — see §8

crates/adept_cli/          BINARY `adept`
  src/
    main.rs                Dispatch; owns process exit codes
    cli.rs                 clap derive structs; TokenizerArg mirror enum
    config.rs              adept.toml discovery (walk up) + parsing
    logging.rs             The one global tracing subscriber — stderr-only
    commands/{check,fmt,eval,fix,create,mcp}.rs
  tests/{cli.rs, fixtures/}
```

**Organizing principle**: layered by capability, not technical role. Each
sibling crate owns one user-facing surface end to end. Anything two surfaces
need moves *down* into `adept` (this is why `text.rs` and `companion.rs`
exist), never sideways between siblings.

**Package name ≠ crate name.** Directory names and `use`-path crate names are
unchanged, but the cargo *package* names differ: `crates/adept` is package
`adept-core` (crate `adept`), `crates/adept_fmt` is `adept-fmt`, `crates/adept_agent`
is `adept-agent`, and `crates/adept_cli` is package `adept` (bin `adept`, no
lib). So `cargo test -p adept-core` builds the core crate, but `use adept::...`
in source is unaffected — `crates/adept/Cargo.toml` sets `[lib] name = "adept"`
explicitly. Anything after `-p` on the command line is the package name;
anything in a `use` statement is the crate name.

## 5. Core Architecture Principles

Violating one is a design change, not a style preference.

1. **The core crate is the only shared vocabulary.** When `adept_fmt` and
   `adept_agent` both need a behaviour, it moves into `adept` so the two cannot
   drift.

2. **Lint findings and hard failures are different types.** A `Diagnostic` is
   something wrong with an otherwise-parseable skill; an `AdeptError` is an I/O
   or parse failure. Discovery never aborts: `SkillSet` keeps `skills` and
   `errors` side by side, and `Linter::lint_set` converts errors into
   `SL001`/`SL002`/`SL003` so a broken skill still reports rather than
   vanishing.

3. **Static checks are offline and fast.** `check`, `fmt`, and eval-dataset
   grading never touch the network, and the 100-skills-under-1s criterion is
   gated in CI (§6).

4. **Everything expensive is constructed once.** BPE tables are cached
   process-wide in `token.rs`; the MCP server holds its `Linter` in a `static
   OnceLock`. This is why `Rule` requires `Send + Sync`.

5. **Pluggability lives behind traits at the two seams that matter**:
   `SkillParser` and `LlmClient`. Both exist because the spec named them. Do
   not add speculative traits elsewhere.

6. **Diagnostics carry precise, stable locations.** Frontmatter fields are
   line-annotated at parse time; `Skill::body_line_offset` translates
   body-relative lines back to file lines. Rule codes are never reused — a
   retired code (`SL202`) stays retired so old configs fail closed.

## 6. Build System & Toolchain

Cargo workspace, no wrapper scripts, no Makefile.

```bash
cargo build --workspace              # CI uses: --workspace --all-targets
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

cargo install --path crates/adept_cli
cargo run -q -p adept -- check <path>
cargo bench -p adept-core --bench lint_100_skills -- --quick
```

**CI** (`.github/workflows/ci.yml`, on push to `main` and all PRs) runs a `ci`
job on ubuntu-latest with those four commands in order, then a **performance smoke
test**: it runs the criterion bench with `--quick`, greps the
`lint_100_skills   time: [...]` line, converts the point estimate to
milliseconds, and fails above **500ms**. That is deliberately ~25× the observed
~20ms, so it catches an order-of-magnitude regression without flaking on noisy
runners. **1 second for 100 skills** is the project's original acceptance
criterion and remains the outer bound; 500ms is the CI gate. The benchmark itself asserts nothing — the gate is bash text-parsing
of criterion output, a known brittleness recorded in `docs/BACKLOG.md`.

`clippy -D warnings` is enforced with `--all-targets`, so **new code must be
clippy-clean including tests and benches**.

**Python distribution** (root `pyproject.toml`, `python/adept/`). A second,
independent build path packs the `crates/adept_cli` binary into a Python
wheel via maturin's `bindings = "bin"`, so
`uv tool install git+https://github.com/mathiasesn/adept` puts a real native
`adept` executable on `PATH` — no PyO3, no importable API. Not published to
PyPI (name squatted, PEP 541 request pending); installable only from the git
URL.

The wheel's version is `dynamic`: maturin resolves it from cargo metadata,
which expands `version.workspace = true` back to
`[workspace.package].version`, so that field stays the single source of truth
and release-plz needs no change. The CI job asserts it.

`python/adept/` is discovery-and-dispatch only, not a library surface:
`__init__.py` re-exports `find_adept_bin` and `AdeptNotFound`, `_find_adept.py`
resolves the installed binary from the installing distribution's own `RECORD`
(via `importlib.metadata`) rather than inferring a layout from directory
names, and `__main__.py` enables `python -m adept`. There is no fallback — an
install that produces no usable `RECORD` raises `AdeptNotFound` rather than
guessing a path, so a source checkout cannot resolve the binary by design.
The no-subprocess invariant in AGENTS.md is about the Rust
binary; `__main__.py` dispatches *to* that binary and so falls outside the
invariant's subject rather than carving out an exception to it. Its POSIX
path calls `os.execvp`, which replaces the process rather than spawning one,
and only the Windows branch — where no exec-and-replace equivalent exists —
uses `subprocess.run`. That branch is a near-verbatim port of ruff's and is
never exercised by CI, a coverage gap accepted deliberately when the
Linux-only job was chosen (see `docs/BACKLOG.md`).

A second CI job, `python-packaging` (`.github/workflows/ci.yml`,
ubuntu-only), builds the wheel with `uv`, installs it, and asserts the
distribution version matches cargo's and that `adept --version` and
`python -m adept --version` agree. It then runs `python/tests` under pytest —
a second test runner that `cargo test --workspace` does not reach, pinning
the RECORD-based script-entry selection in `_find_adept.py`.

**Release pipeline** (`.github/workflows/release.yml`, `release-plz.toml`).
release-plz runs on push to `main` as two independent jobs, `release` and
`release-pr`, that run in parallel and do not depend on each other's outcome:
`release` publishes what is already on `main`, while `release-pr` prepares
what comes next. The `release` job publishes to crates.io — any crate whose
`[workspace.package].version` isn't already on the registry, in dependency
order — and pushes a `{{ package }}-v{{ version }}` tag per published crate.
The `release-pr` job keeps a rolling PR open carrying Conventional-Commits-derived
version bumps and changelog; merging that PR lands the bumped versions on
`main`, which the next push's `release` job turns into an actual publish. All
four packages share one `version_group` in `release-plz.toml`, so they bump
and publish in lockstep rather than independently. The `release` job keys off
crates.io registry state, not PR identity — see AGENTS.md's commit
conventions for why that means never hand-editing the version. Requires the
`CARGO_REGISTRY_TOKEN` repo secret, which is scoped to the `release` job
alone — the only job with publish rights, and the only one that builds, so it
alone carries the toolchain and cache steps. `release-pr` is the only job with
PR-write. `concurrency` stays workflow-level so it covers both jobs at once and
prevents two pushes from racing the publish or the PR update. The one step both
need, checkout, is a YAML anchor defined in `release` and aliased in
`release-pr`. `release.yml` owns the literal permissions, guard, and
concurrency expressions; nothing machine-checks a copy of them here.

The `release` job currently carries `dry_run: true`, so it logs what it would
publish and publishes nothing. Nothing has been released yet. Going live means
**deleting** the `dry_run` line, not setting it to `false` — that value is a
silent no-op, for the reason spelled out in `docs/BACKLOG.md`'s pre-publish
checklist, which also lists the conditions under which the deletion is safe.
`crates/adept/tests/workspace_metadata.rs` fails the build if the workflow ever
sets `dry_run: false`.

The `release-pr` job uses a second secret, `RELEASE_PLZ_TOKEN`. GitHub never
fires workflow triggers for events caused by the default `GITHUB_TOKEN` (loop
prevention), so a release PR opened with it would arrive with no CI checks at
all — and that PR is the one whose merge publishes to crates.io. A
fine-grained PAT with `contents: write` + `pull_requests: write` makes CI run
on it normally, so it is a hard prerequisite: configure the secret before the
first merge to `main`. There is no fallback and no fail-fast: the `release` job
succeeds on the plain `GITHUB_TOKEN` regardless, so the first push to `main`
can publish all four crates irreversibly while only `release-pr` goes red for
lacking the PAT.

## 7. Configuration

Precedence: **CLI flag > config file value > built-in default**.

`adept.toml` is discovered by walking *up* from the target path
(`config.rs::discover_config_file`). `--config <path>` forces a file and skips
discovery; a missing explicit `--config` is a hard error (exit 2), while a
missing discovered file silently falls back to defaults.

```toml
[lint]                        # deserializes into adept::LintConfig
disabled = ["SL206"]
description_min_tokens = 6
description_max_tokens = 75
body_max_tokens = 1500
tokenizer = "o200k_base"      # or "cl100k_base"

[fmt]                         # deserializes into adept_fmt::FmtConfig
line-width = 100

[eval]                        # EvalFileConfig, CLI-local (renamed from [score])
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"
capture_dir = ".adept-capture"  # off by default; gitignore it

# [fix] (FixFileConfig) and [create] (CreateFileConfig) take those same four keys, plus:
#   max_rounds = 2    both; falls back to adept_agent::DEFAULT_MAX_ROUNDS
#   eval_cases = 10   [create] only; falls back to create::DEFAULT_EVAL_CASES, no CLI flag
```

**Key casing is inconsistent across sections — verify against the struct, not
this table.** `[lint]` is `snake_case` (serde default); `[fmt]` is `kebab-case`
(`rename_all`); `[eval]`/`[fix]`/`[create]` carry **no** `rename_all`, so their
keys are plain Rust field names (`base_url`, not `base-url`).

Those three sections **never fall back to one another** — the only thing they
share is the `ADEPT_*` env vars below, and `capture_dir` follows the same
independence (pinned by `config.rs::capture_dir_sections_do_not_cross_fall_back`).
They are structurally near-identical, and `docs/BACKLOG.md` records why they
were not collapsed into a `#[serde(flatten)]`ed shared `LlmFileConfig`: the
refactor touches all three at once. Their independence as *TOML sections* is a
spec requirement and must survive any such refactor.

**A stale `[score]` section is a hard error**, not a silently-ignored table:
`config.rs::contains_legacy_score_section` checks for it explicitly (rather
than relying on `deny_unknown_fields`, which the config structs deliberately
lack — see `docs/BACKLOG.md`) and exits `2` naming the fix, because silently
ignoring it would quietly drop the user's `model`/`capture_dir`.

**Capture directory resolution** (`config.rs::resolve_capture_dir`, used by
`eval` and `fix`). Standard precedence, but the two layers anchor a *relative*
path differently:

| Source | Relative path resolves against |
|---|---|
| `--capture-dir` | the process CWD (passed through untouched) |
| `[eval]`/`[fix]` `capture_dir` | the directory containing the `adept.toml` that supplied it |

The config anchor comes from `AdeptConfig::origin_dir`, which is
`#[serde(skip)]` — stamped by the loader, never deserialized, so a hostile
`adept.toml` cannot redirect writes by declaring it. With no value from either
layer, capture is off.

**Environment variables** (`ADEPT_LOG` applies everywhere; the rest are
LLM-backed commands only, resolved in `adept_agent::LlmConfig::resolve`):

| Var | Flag | Purpose |
|---|---|---|
| `ADEPT_MODEL` | `--model` | Required by the `triggering`/`token-bloat`/`overlap` analyses, which exit 2 without it; grading alone needs no model. Over MCP its absence fails those analyses per-call rather than hiding the tool — §12. |
| `ADEPT_BASE_URL` | `--base-url` | Defaults to `https://api.openai.com/v1`. Rejects an embedded credential — see below. |
| `ADEPT_API_KEY` | *(none)* | Bearer token. Never accepted as a flag. Held as a `RedactedString` — §15. |
| `ADEPT_LOG` | `-v`/`-vv`/`-vvv` | `EnvFilter` syntax (e.g. `adept_agent::llm::client=trace`). Overrides the `-v` count wholesale. Adept's own namespace, not `RUST_LOG`. |

**`ADEPT_BASE_URL` credential rejection.** `LlmConfig::resolve` parses the
resolved value with `reqwest::Url` and returns `ConfigError::CredentialsInBaseUrl`
if `username()` is non-empty or `password()` is `Some` — a `base_url` carrying
credentials is rejected at resolution, never accepted and scrubbed later. A
value that fails to parse at all is left alone and still fails downstream at
`RequestBuilder::build`; the check targets userinfo only.

Every caller of `resolve()` decides what "an LLM is configured" means through
one shared predicate, `adept_cli::config::llm_available`: it maps `Ok(_)` and
`Err(ConfigError::CredentialsInBaseUrl)` both to `true` (the user did point at
an endpoint; only `MissingModel` means "no model") and is exhaustive, so a
future `ConfigError` variant is a compile error here rather than a silent
fallthrough. Four call sites share it:

- `resolve_llm_client` (`adept_cli::config`) — used by the `eval`/`fix` CLI
  commands. On `CredentialsInBaseUrl` it prints credential-specific guidance
  naming `api_key` instead of the generic model-guidance text, and exits `2`.
- `probe_model_available` (`adept_cli::commands::eval`) — the CLI's
  triggering/token-bloat/overlap precondition check. Also exits `2` on
  `CredentialsInBaseUrl`, via the same path as `resolve_llm_client`.
- The `create_skill`/`generate_evals` tool-advertisement gate
  (`adept_cli::commands::mcp`) — calls `llm_available` to decide
  whether to list those tools in `tools/list`. A `CredentialsInBaseUrl` counts
  as configured, so the tools stay **advertised** rather than vanishing
  indistinguishably from "no model configured"; the actual credential error
  then surfaces per-call when the tool is invoked.
- `eval_skill`'s model-availability check (`adept_cli::commands::mcp`) — same
  treatment: `CredentialsInBaseUrl` selects the LLM-backed
  analyses as available, and the real `resolve()` failure is reported as a
  structured per-call tool error naming the problem.

MCP has no exit code, so unlike the two CLI paths above, both MCP consumers
surface `CredentialsInBaseUrl` as a JSON-RPC tool-call error, not a process
exit — see §12.

No compile-time feature flags. `LintConfig`'s numeric thresholds each carry a
doc comment justifying the number — **keep that rationale attached if you
change a default.**

## 8. Command Structure & Exit Codes

`main.rs` parses with clap, resolves config from the *first* target path,
dispatches to `commands::<name>::run`, and calls `std::process::exit` with the
returned code. Command functions return `i32` — they never exit or panic.

Global flags: `--config <PATH>`, `--no-color`, `-q/--quiet`, `-v/--verbose`
(repeatable). `--quiet` and `-v` are independent — `--quiet` trims *stdout*
results, `-v` adds *stderr* diagnostics, so `-q -vv` is meaningful. Color is on
only when `!no_color && stdout().is_terminal()`, computed once in `run()`.

**Exit code contract** — public API, do not change:

| Code | Meaning |
|---|---|
| `0` | Clean (no diagnostics; nothing would be reformatted; or `--exit-zero`) |
| `1` | Diagnostics found (`check`) / files would be reformatted (`fmt --check`, `--diff`) |
| `2` | Usage or I/O error: bad path, unreadable file, bad config, unresolvable model, runtime failure |

Per command:

- **`check`**: `--format human|json`, `--select`/`--ignore` (comma-separated or repeated; accept a code `SL201` or a kebab name `description-too-short`), `--statistics`, `--exit-zero`, `--tokenizer`. `--select` is implemented as "disable everything not named" on top of `LintConfig::disabled` — see `apply_select_ignore`.
- **`fmt`**: `--check` (diff + exit 1), `--diff` (diff, exit 0), `--line-width`. Writes are **atomic**: temp file `.{name}.adept-tmp` in the same directory, `sync_all`, then `rename`. A failed format never clobbers the original.
- **`eval`**: `path` (a `SKILL.md` or a skill directory), `--format`, `--model`/`--base-url` (resolved against `[eval]`), `--num-prompts`/`--seed`/`--judge-samples` (triggering), `--tokenizer`, `--capture-dir`, `--results <PATH>`, `--evals <PATH>` (default `evals/evals.jsonl` relative to the skill dir), and `--select`/`--ignore` over `triggering`, `token-bloat`, `overlap`, `evals`. `evals` runs iff `--results` is passed; the other three iff a model is configured. Explicitly `--select`ing an analysis with a missing precondition is exit `2` naming what's missing; nothing available at all is exit `2`. Unselected analyses are `null` in JSON, not empty. Builds its own `tokio` runtime, but only when an LLM-backed analysis is selected — `--select evals` constructs no `LlmClient`.
- **`fix`**: **preview by default** — computes and prints a `FixReport` (summary, or unified diff via `--diff`) without touching disk. `--write` applies via `write_all_transactionally`; `--check` exits `1` if any skill has pending changes, printing the same diff (matching `fmt --check`). `--select`/`--ignore` restrict which diagnostics are attempted; `--max-rounds` bounds the fix/re-lint loop (default `DEFAULT_MAX_ROUNDS = 2`). Builds its own runtime.
- **`create`**: brief from `--from-file`, non-TTY stdin, or an interactive multi-line prompt, in that precedence order; exit `2` if none. `--out <dir>` (default cwd), `--name`, `--write`/`-w`, `--overwrite` (opt-in to writing into a directory that already has a `SKILL.md`), `--max-rounds`, `--model`/`--base-url`/`--tokenizer` (resolved against `[create]`), `--capture-dir`, `--format`. Runs generate → screen → repair (gate: zero Error, zero Warning; Info passes), then generates `evals/evals.jsonl`. A clean candidate exits `0`; one that exhausts `max_rounds` with findings still writes/prints the best candidate and exits `1`. `2` covers usage/I/O errors, an unparseable LLM response, and refusing to clobber an existing skill directory.
- **`mcp`**: no flags; reads stdin until EOF.
- **`eval` and `fix` additionally take `--capture-dir`** (§15), resolved before any request is issued so a bad directory is exit 2, not a silent skip.

**Output**: diagnostics and reports to stdout; every error message to stderr
prefixed `adept: error: `. `--quiet` suppresses only summary/progress lines,
never diagnostics.

## 9. Core Library Surface (`adept`)

Everything public is re-exported from `lib.rs`; the only public submodules are
`reporting`, `text`, `markdown`, and `evals`. Adding a type means adding it to
that re-export list.

**`Skill`** holds `path`, `frontmatter`, `body`, `body_line_offset`, and the
complete unmodified `source`. Retaining both `body` and `source` costs ~2× file
bytes — deliberate, since `fmt` compares against `source` byte-for-byte.

**`SkillParser`** is the format seam. `parse` has a default body that reads the
file and delegates to `parse_str`, so implementors only write `parse_str`.
`AnthropicSkillParser` splits on `\n` and strips a trailing `\r` per line, so
LF and CRLF are handled uniformly; frontmatter is `---`-delimited, required
`name`/`description`, optional `license`, everything else preserved in
`Frontmatter::extra` (a `BTreeMap`, so `fmt` gets deterministic ordering free).

*Known limitation*: `SKILL_FILE_NAME` is a private `const` in `skillset.rs`,
not part of `SkillParser`, so a parser for a `skill.yaml`/`AGENT.md` format can
never be handed a file — the seam is incomplete. See `docs/BACKLOG.md`.

**`SkillSet::discover`** accepts a `SKILL.md`, a skill directory, or a tree. It
skips hidden directories and `target`/`node_modules`, but never excludes the
root itself even if the root's own name matches.

**`TokenCounter`** holds a `&'static CoreBPE`. `token.rs::load_bpe` caches each
encoding in a `OnceLock<Result<CoreBPE, String>>` — the `Result` is cached too,
so a load failure is reported to every caller instead of retried forever.
`tiktoken-rs`'s `*_singleton()` helpers panic on failure; the hand-rolled cache
exists to preserve the `Result` API. **Never construct BPE tables elsewhere.**

**`text.rs`** owns the definition of a "word" (lowercased, split on
non-alphanumeric) and `jaccard`. `word_bag` and `words` share one tokenizer so
set-based and order-based callers cannot diverge.

**`markdown`** is the shared lexer and the only markdown-aware code in the
workspace, exposing two views over one document: `parse_document` builds the
span-free `Block`/`Inline` AST the formatter re-prints, while `headings` /
`link_destinations` / `inline_code_spans` return `Located<T>` values (1-based
body line) for the `SL1xx` rules, which need positions but no tree. Both go
through `markdown::parser()`, the **single** `pulldown-cmark` `Parser`
construction site — `grep -rn "Parser::new" crates/` must keep yielding exactly
one hit. **Do not construct a `Parser` anywhere else**: an `Options` flag
enabled for one view and not the other would let the linter and formatter drift
back into disagreeing about what a heading or link is. Fence matching, info
strings, indented code, nested brackets, and reference links all come from the
parser, so a markdown-aware rule never needs its own line scan.

**`adept::evals`** publishes the eval-dataset schema *and an offline grader*
(`validate`, `parse_results_jsonl`, `grade(cases, results) ->
EvalBenchmarkReport`). `grade` is deterministic and never spawns a subprocess,
so **adept still never *executes* a dataset** — it grades results a harness
already produced. `docs/EVALS.md` is the contract, machine-checked against the
code the same way `docs/RULES.md` is; the assertion vocabulary and the
`results.jsonl` shape live there, not here.

**`adept::is_eval_dataset(skill_dir, path)`** is applied at both `SL303` and
the token-bloat view, so a generated dataset is never counted as skill content
by either. Its matching semantics and its current dormancy are documented with
the rule, in `docs/RULES.md` (SL303).

## 10. Formatter and Evaluation Surfaces

### `adept_fmt`

Pipeline: `Skill` → canonical frontmatter string →
`adept::markdown::parse_document(&skill.body)` → `markdown::print_document` →
output. The formatter owns only the printer; the AST and its builder live in
the core crate. Exactly one blank line separates the closing `---` from the
body.

Frontmatter order is fixed: `name`, `description`, `license` (if present), then
extras alphabetically. YAML quoting is minimal-but-correct, hand-emitted.

Nesting of block quotes, lists, and footnote definitions is bounded by
`MAX_NESTING_DEPTH = 100`; anything deeper becomes `Block::Raw` and is passed
through verbatim rather than recursed into — a stack-overflow guard against
adversarial input. Keep it.

Prefer `format_skill`/`check_skill` (already-parsed `Skill`) over
`format_str`/`check_str` (re-parses). The CLI always has a `Skill` in hand.

Documented limitations, each visible to users as an unexpected diff:
reference-style link *definitions* are inlined at each use; Setext headings
always become ATX; tight-list preservation is partial; text escaping covers a
conservative subset. `HeadingStyle` and `StrongMarker` are single-variant
placeholders kept so config files can express intent without a future breaking
change — do not "simplify" them away.

### `adept_agent::eval`

Everything is `async` and reached through `&dyn LlmClient`. The crate **never
creates a tokio runtime**; callers drive it. `eval_skill` is the entry point
for the three LLM-backed analyses:

1. **Triggering accuracy** (`triggering.rs`) — the LLM generates N candidate
   prompts (half positive, half negative, `DEFAULT_NUM_PROMPTS = 10`), then a
   judge model sees *only the name and description* (never the body, mirroring
   real tool selection) and predicts whether it would trigger. Reports
   precision/recall/F1. `seed` and `judge_samples` (majority vote) damp judge
   variance.
2. **Token bloat** (`tokens.rs`) — description/body/companion counts via
   `TokenCounter`, plus LLM trimming suggestions. Companion discovery is
   `pub use adept::discover_companion_files` — the same walk `SL303` uses,
   re-exported rather than reimplemented.
3. **Overlap** (`overlap.rs`) — a cheap offline `jaccard` shortlist at
   `DEFAULT_SIMILARITY_THRESHOLD = 0.25` over name+description, then LLM
   adjudication of only the shortlisted pairs, filtered to pairs involving the
   scored skill.

The fourth analysis, **eval-dataset grading**, needs no LLM: the CLI calls
`adept::evals::grade` directly (§9) and folds the `EvalBenchmarkReport` into
`EvalReport::evals`. This is what makes `--select evals` touch no network.

Construct options via **`EvalOptions::for_model(model, tokenizer)`**, not a
struct literal — the model name has to reach both `EvalOptions::model` and
`TriggeringOptions::model`, and that constructor is the one place the wiring
lives, so the CLI and MCP tool cannot drift on defaults.

Prompt templates live in `adept_agent::eval::prompts` and are stamped into
every report as `PROMPT_VERSION`. **Its value is deliberately still
`"adept_score-prompts-v1"`** — unmoved by the crate merge so old and new
reports compare identically. Bump it only when wording changes in a way that
could shift scores. It is distinct from `adept_agent::prompts`'s per-surface
`PROMPT_VERSION`s (fix/create).

**Deliberate divergence**: the overlap shortlist uses name+description at 0.25
(tuned for recall — it only shortlists); `SL402` uses description-only at 0.6
(tuned for precision — it emits a diagnostic). Both call `adept::text::jaccard`.
`check` and `eval` can therefore reach different conclusions about the same
pair. This is intended.

**`EvalReport`** carries all four analyses as independent optional fields — an
unselected analysis is absent (`null` in JSON), distinguishable from one that
ran and found nothing. Two `EvalError` types exist and must not be confused:
`adept::evals::EvalError` (dataset/results parse failures) and
`adept_agent::eval::EvalError` (transport/JSON failures).

## 11. Rule Engine & Extension Points

`crates/adept/src/rules/mod.rs` is the file to read before touching rules.

**Three traits.** `Rule` carries identity — `code()`, `name()`,
`default_severity()` — and requires `Send + Sync`. Rules are stateless unit
structs, so this costs nothing and is what lets a `Registry` (hence a `Linter`)
live in a `static`. `SkillRule: Rule` checks one `Skill`; `SetRule: Rule`
checks a whole `SkillSet`. Both receive `&LintConfig` and a shared
`&TokenCounter`.

**Adding a rule — all four steps are required:**

1. Add the unit struct and `check` impl to the taxonomy file matching its code
   (`SL00x` → `rules/frontmatter.rs`, `SL1xx` → `structure.rs`, `SL2xx` →
   `description.rs`, `SL3xx` → `tokens.rs`, `SL4xx` → `cross.rs`).
2. Declare identity with the macro, on one line next to the struct:
   `impl_rule!(MyRule, "SL107", "my-rule", Warning);`. Never hand-write the
   `impl Rule` block.
3. Register it in `Registry::new`, in the matching `vec![]`, in code order.
4. Document it in `docs/RULES.md`. **`docs_test.rs` fails the build if a
   registered code is undocumented** — a hard gate, not a nicety.

Then add a fixture under `crates/adept/tests/fixtures/rules/` and an insta
snapshot (§13).

**Opting a rule into LLM fixing.** `Rule::fix_kind()` defaults to
`FixKind::None`. A rule opts in by passing `Llm` plus a region to the macro:
`impl_rule!(MyRule, "SL107", "my-rule", Warning, Llm, Description);` — region is
`Description` or `Body` (`SL301`/`SL206` → `Description`, `SL302` → `Body`).
This is **metadata only**: `adept` never fixes anything; the tag just makes the
rule visible to `adept_agent::fixable()`, and the region tells `adept_agent`
which batched request to route the diagnostic into. `adept_agent` hard-codes no
rule-code list, so tagging a new rule is immediately sufficient to make `adept
fix` attempt it; the unit test `only_expected_rules_are_tagged_llm_fixable`
pins the current tagged set and each region so the two cannot drift.
`FixKind::Deterministic` exists for a future non-LLM autofixer, carries no
region, and nothing returns it today. Only `SkillRule` diagnostics are ever
attempted — `SetRule` (cross-skill) findings are reported but never
auto-rewritten.

**Severity and enablement are applied by the `Linter`, never by the rule.**
Rules build diagnostics with their `default_severity()`;
`LintConfig::apply_overrides` rewrites it afterwards. Both `disabled` and
`severity_overrides` accept either the code or the kebab-case name.

**Rule codes are permanent.** `SL202` is retired (it duplicated `SL301`
exactly) and never reused, so an old config naming it fails closed.

**A third rule flavour: `ParseErrorRule`.** A skill with malformed frontmatter
has no `Skill` to run `SkillRule`s against, so `SL001`/`SL002`/`SL003` are also
registered as `ParseErrorRule`s (`fn check(&self, path: &Path, err:
&AdeptError)`) in `Registry::parse_error_rules`. `SL001`/`SL002` are
dual-registered — once as a `SkillRule` (field present but empty) and once as a
`ParseErrorRule` (field missing entirely, so parsing failed); `SL003` exists
only as a `ParseErrorRule`. `Linter::lint_set` runs them over `SkillSet::errors`
through the same `is_enabled` + `apply_overrides` dispatch, so
`--ignore`/`disabled`/`severity_overrides` work by code or name exactly as
elsewhere. `Registry::meta_iter` dedupes by code so dual registration doesn't
produce duplicate `RuleMeta` entries.

**Output ordering** is `(path, line, column, code)`, implemented once in the
public `adept::sort_diagnostics`. It is the user-visible ordering contract —
call it, never re-implement the comparator.

## 12. MCP Server Contract

`crates/adept_cli/src/commands/mcp.rs`. Hand-rolled JSON-RPC 2.0 over
newline-delimited stdio, protocol version `2024-11-05`. The `rmcp` SDK was
deliberately not used: a direct implementation keeps both the dependency
footprint and the risk of an SDK writing to stdout at zero.

Five tools. `check_skill`, `format_skill`, and `eval_skill` are advertised
unconditionally; `create_skill` and `generate_evals` only when
`adept_cli::config::llm_available` reports a model configured (§7) — which
also holds true for a `ConfigError::CredentialsInBaseUrl`, so a leaked
credential keeps both tools listed rather than hiding them; invoking either
then fails per-call with a credential-specific message instead of exiting a
process (MCP has no exit code).
**The latter two are preview-only and never touch the filesystem** — they
return the generated skill/dataset as data, mirroring why capture is CLI-only
(§15): an MCP client must not be able to make the server write to arbitrary
paths. Writing stays a CLI capability. **`eval_skill` is read-only** — it never
writes either.

**The hard invariant: stdout carries only JSON-RPC response messages.**
`serve()` is the only function that writes to stdout. `handle_message` is pure
with respect to I/O, which is what lets tests drive it without spawning the
binary. Any `println!` added below `serve` breaks every MCP client silently.
Use `eprintln!`.

- The `Linter` is built once into a `static OnceLock<Result<Linter, String>>`.
  `Linter::new` loads the tiktoken BPE tables, which costs far more than the
  lint itself; rebuilding per call is not acceptable.
- **`check_skill` uses `LintConfig::default()` and does not discover
  `adept.toml`.** Intended: the CLI stays the only config-aware entry point, so
  an MCP client's results can't silently shift depending on what config file
  happens to sit near a path it passes in. Pinned by a test that raises
  `description_min_tokens` in a sibling `adept.toml` and asserts the default
  threshold still applies.
- `eval_skill` **opts out of the advertisement gate** the other LLM tools apply,
  because grading needs no model — gating it would hide a tool that works with
  no `ADEPT_MODEL` set. Preconditions are enforced per-analysis instead,
  exactly as the CLI's `--select`/`--ignore`: an explicitly selected analysis
  with a missing precondition is a structured tool error naming what's missing,
  never a silent skip. It takes `results` as an inline JSON array — not a path,
  since an MCP client may not share a filesystem with the server — plus
  optional `evals`/`select`/`ignore`. `results` alongside raw `content` (no
  `path`) grades `contains` only and reports `file_exists`/`file_contains` as
  *skipped*, naming the missing directory; that is not an error. LLM-backed
  analyses are bounded by a 30s timeout; grading needs none.
- `format_skill`'s `line_width` is validated to `20..=500`; `create_skill`'s
  `max_rounds` to `1..=10`; both tools' `eval_cases` to `1..=50`. Out-of-range
  values are rejected with a structured tool error, **never clamped** — an MCP
  client talks to this server over public JSON-RPC with no other gate on LLM
  spend, so every numeric argument driving LLM calls needs an explicit bound.
- Notifications (no `id`, e.g. `notifications/initialized`) return `None` and
  produce no output line.

**Overlap detection over MCP uses real siblings.** `overlap_skillset`
discovers them via an optional `directory` argument, or `adept::sibling_root(path)`
when a real on-disk `path` is given, mirroring the CLI. Passing just `[skill]`
makes the analysis inert — it can only find overlaps against skills it sees.

## 13. Testing & Snapshot Conventions

336 tests across 19 suites (16 unit/integration + 3 doc-test), a few seconds
for a full workspace run. Unit tests live beside the code (`#[cfg(test)] mod
tests`); integration tests in `tests/`.

**Rule tests are snapshot tests.** Each rule gets a fixture directory under
`crates/adept/tests/fixtures/rules/<sl_code>_<slug>/` containing a `SKILL.md`
that triggers exactly that rule, plus an `insta` snapshot. A `cross_clean`
fixture asserts a well-formed pair produces nothing. Review snapshot diffs — an
accepted snapshot is an accepted behaviour change.

**Formatter tests** are fixture-in / snapshot-out
(`crates/adept_fmt/tests/fixtures/*.md`), one file per construct family, plus
`proptest_idempotency.rs` asserting `fmt(fmt(x)) == fmt(x)`. The proptest
currently excludes tables, fences, HTML, and emphasis, and the broad
idempotency loop is gated on `reflow_prose: false` — **prose reflow is the
least-covered high-risk path in the repo.**

**No test may perform network I/O.** `adept_agent` tests use `MockLlmClient`
(`with_texts` seeds a FIFO queue of scripted responses). `adept_cli`'s eval test
drives `run_with_client`, a `#[cfg(test)]` seam taking `&dyn LlmClient`.
Eval-dataset grading needs no client and is tested with no model configured,
pinning the claim that `--select evals` never constructs an `LlmClient`.

**CLI tests** use `assert_cmd` + `predicates` against
`tests/fixtures/{clean-skill,defective-skill}`. `tests/tracing.rs` guards §15:
it drives the real binary under `-vvv` and `ADEPT_LOG=trace` and asserts MCP
stdout stays byte-identical pure JSON-RPC, that `check`/`fmt --check` output is
unaffected by the logging layer (writing nothing to stderr by default), and
that an uncreatable `--capture-dir` exits 2.

**`docs_test.rs`** asserts every registered rule code appears in
`docs/RULES.md` and every assertion kind in `docs/EVALS.md`.

**Benchmark**: `benches/lint_100_skills.rs`, criterion, `harness = false`,
~20ms. §6 covers the CI gate.

Not covered: no fixture exercises a real skills corpus. The "runs clean-ish on
anthropics/skills" acceptance criterion is verified manually; a vendored
mini-corpus would make it a test.

## 14. Known Divergences & Deferrals

`docs/BACKLOG.md` has the full list. Two worth knowing before you write code:

**`SL104`'s heuristic filters are not lexing.** The URL-scheme, glob, `~`,
`@scope/name`, and template-placeholder filters in `rules/structure.rs` are
genuine domain judgements about what a repo-relative path looks like, with one
consumer. They sit *above* the shared lexer (§9) and should stay hand-written —
do not push them into `adept::markdown`.

**Performance work is deliberately not done**, since `check` runs well inside
its 1s target (§6). None of these are bugs:

- Discovery and the per-skill lint loop are sequential and embarrassingly
  parallel — `rayon` would cut wall time by ~Ncores.
- `SL402`/`SL403` are each O(n²) pairwise Jaccard: fine at 100 skills, 500k
  pairs at 1000.
- Words are `String`-allocated per word; hashing to `u64` would remove that.
- `Skill` retains both `source` and `body`, costing ~2× file bytes.

## 15. Observability, Capture & Secret Handling

Two independent layers. Both exist to make an LLM run reproducible after the
fact; neither is on by default, and neither changes a byte of output when off.

### Layer 1 — `tracing` to stderr

`adept_cli/src/logging.rs::init` installs the **one** global subscriber, from
`main`, before dispatch — every subcommand including `mcp`. Level comes from the
global `-v` count (`0` off, `1` info, `2` debug, `3+` trace), scoped to adept's
own crate targets so `-vvv` does not drown the user in `hyper`/`reqwest`
internals. `ADEPT_LOG` replaces that filter wholesale.

**The writer is `std::io::stderr`, explicitly, with `.with_ansi(false)`.** This
is the load-bearing line in the file: `tracing_subscriber::fmt()` defaults to
**stdout**, and `adept mcp` speaks JSON-RPC there, so a subscriber that lands on
stdout breaks every MCP client silently — no error, no diagnostic, just a client
that stops parsing. Do not drop `.with_writer`, and do not add a second
subscriber anywhere.

**Libraries emit, they never subscribe.** `llm::client::send_once` — the single
funnel for `eval` and `fix` — emits the serialized request body and raw response
text at `DEBUG`/`TRACE` *before parsing*, so a body that fails
`parse_chat_response` is still on the wire in the log. No library crate calls
`try_init`.

Default level is `off`: with no `-v` and no `ADEPT_LOG`, stdout *and* stderr are
identical to a build with no logging at all. Pinned by test.

### Layer 2 — `--capture-dir`

`llm::CaptureSink` writes verbatim per-call artifacts as
`<capture-dir>/<timestamp>/{run_metadata.json, call_NNNN/{request.json,
response.json, call_metadata.json}}`. Bodies are written **at the moment of
receipt, before parsing, never truncated**, and non-2xx responses are captured
too — an error body is exactly the evidence a live-endpoint validation needs.
Each invocation gets its own timestamped folder, so a capture dir is only
appended to; re-running never overwrites.

Opt-in via `OpenAiCompatClient::with_capture(Arc<CaptureSink>)`. `eval`/`fix`
resolve it **before** any request is issued, so an uncreatable directory is a
usage error (exit 2) rather than a silent skip. `run_metadata.json` is
self-describing by design: adept version, `PROMPT_VERSION`, subcommand,
resolved options, the *source* of each resolved value (flag / `adept.toml` /
env / default), timestamps, target path, and the exit code stamped by
`finalize`.

**Capture is CLI-only and never reachable from MCP.** `--capture-dir` /
`capture_dir` exist on `eval` and `fix` alone; MCP gets stderr tracing only.
This keeps capture off the public JSON-RPC contract. Capture artifacts contain
full prompts and full model output — steer users to a gitignored path.

### Secret handling

The API key is wrapped in `llm::RedactedString`, and that newtype is **adopted
at the `ResolvedLlmConfig::api_key` field**, not merely defined. Its `Debug`
*and* `Display` impls both render `****`, so `{:?}` on a resolved config — the
realistic leak, since a tracing event or panic message will happily format a
whole struct — cannot expose it. Never add a field, accessor, or log line that
renders the key by another route, and never derive `Serialize` onto a struct
holding one.

The invariant: **once configuration is resolved, the key exists only inside a
`RedactedString`, and `expose_secret()` is the sole way back out.** The
deliberate carve-out is the *input* side — `LlmConfig::api_key` is a plain
`String` in a `#[derive(Debug)]` struct, because it is assembled from flags and
config files before anything can be redacted. `LlmConfig::resolve` is the single
place the wrap happens, confining the unredacted type to pre-resolution
plumbing, which is never logged or captured. Do not widen that carve-out.

Three rules follow and are not negotiable: the `Authorization` header is
**omitted entirely** from captured request metadata (not masked, not present —
`request_header_map` strips it from the real outgoing headers);
`run_metadata.json` records only `api_key_present: bool`; and every request and
response body passes through `OpenAiCompatClient::scrub` before being logged or
written, replacing any verbatim occurrence of the resolved key with `****`. The
scrub is a defensive backstop, not the primary defence — no body carries the key
today — and is deliberately an exact-substring match rather than a
secret-shaped-string heuristic. Unit tests assert a configured key appears in no
tracing output and no capture artifact, including when smuggled into a prompt.

## 16. Invariants

- **Dependency direction is one-way** (§2). Nothing in the library stack may
  depend on `adept_agent`; only `adept_cli` does. Do not add a sideways
  dependency between siblings.
- **`check`, `fmt`, and eval-dataset grading never touch the network.** The
  `triggering`, `token-bloat`, and `overlap` analyses do, and only when a model
  is configured; `adept eval --select evals` needs no API key.
- **MCP stdout carries only JSON-RPC.** All logging goes to stderr;
  `handle_message` stays I/O-pure. `create_skill`/`generate_evals` are
  preview-only and never write to disk; `eval_skill` is read-only and always
  advertised regardless of whether a model is configured.
- **adept spawns no subprocess, ever.** Not even the `command` eval assertion
  `create` can generate — adept defines and validates that vocabulary but never
  executes it; `grade` judges a `command` assertion only from a harness-supplied
  exit code.
- **The tracing writer is stderr, never stdout.** Only `main` installs a
  subscriber; libraries emit and never subscribe. See §15.
- **The API key never leaves `RedactedString`.** `Debug` and `Display` both
  render `****`. `Authorization` is omitted entirely from captured metadata;
  `run_metadata.json` carries only `api_key_present: bool`.
- **A credential-bearing `base_url` is rejected at config resolution, not
  scrubbed at each egress.** `LlmConfig::resolve` returns
  `ConfigError::CredentialsInBaseUrl` when the resolved `base_url` parses with
  non-empty `username()` or a `password()`, closing the three egresses a
  userinfo-carrying URL previously reached — `LlmError::Request`'s `Display`
  (via reqwest), the matching `tracing` event, and `RunMetadata.base_url` on
  disk — by construction rather than per-site sanitization. `ResolvedLlmConfig`
  and the `LlmError::Request`/`MalformedResponse` variants are
  `#[non_exhaustive]`, so `resolve()` is the only way to build one from outside
  the crate — a struct literal can't bypass the check. All four consumers
  (`resolve_llm_client`, `probe_model_available`, and the two MCP call sites in
  `commands/mcp.rs`) route the decision through the shared
  `adept_cli::config::llm_available` predicate; the CLI paths exit `2`, the MCP
  paths surface it as a per-call tool error (no exit code). See §7, §12, and
  `docs/BACKLOG.md`.
- **Exit codes are a public contract**: `0` clean, `1` findings, `2` usage/I/O
  error.
- **Rule codes are permanent and never reused.** Retired codes stay retired and
  documented.
- **Every registered rule must appear in `docs/RULES.md`**, and every assertion
  kind in `docs/EVALS.md` — `docs_test.rs` enforces both.
- **Use `impl_rule!` for a rule's identity, register it in `Registry::new`, and
  let the `Linter` apply severity.** Rules never resolve their own enablement.
- **Construct `EvalOptions` via `for_model`**, and bump `PROMPT_VERSION` in
  `adept_agent::eval::prompts` when prompt wording changes meaningfully.
- **Never load BPE tables outside `token.rs::load_bpe`**; build expensive
  objects once (`static OnceLock`), which is why `Rule: Send + Sync`.
- **One parser-construction site**: `adept::markdown::parser()`. `grep -rn
  "Parser::new" crates/` must yield exactly one hit.
- **`fmt` writes atomically** (temp file + rename) and is idempotent. Both are
  tested; keep them.
- **`cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
  must pass**, tests and benches included.
