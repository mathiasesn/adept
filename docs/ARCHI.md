# adept Architecture Documentation

> Generated: 2026-07-22 · Commit: `485672f` · Version: `0.1.0` (workspace-wide, unreleased)
> Re-read this file at the start of any session touching this codebase. Update it when the architecture changes (new major dependency, restructured layer, changed convention).

## 1. How to Read This Document

This is the architecture source of truth for `adept`. It is written for AI coding agents first and human contributors second: claims are concrete and traceable to real files, conventions are stated as rules to follow rather than described.

It is **not** a substitute for three documents that already exist and stay authoritative in their own domains:

| Document | Owns |
|---|---|
| `docs/MVP.md` | The originating spec: goals, non-goals, constraints, acceptance criteria. Do not re-litigate scope without reading it. |
| `docs/RULES.md` | The per-rule reference (`SL001`–`SL403`): what each rule flags and how to fix it. Machine-checked against the registry. |
| `docs/BACKLOG.md` | Known gaps and deliberate deferrals. Check here before "discovering" a bug. |

Sections 2–7 are universal; 8–10 cover the CLI and library surfaces; 11–14 cover the rule engine, MCP contract, testing conventions, and known divergences. Section 15 is the short list of things not to violate.

**Update this file when**: a crate is added or a crate boundary moves, a new dependency with architectural weight lands, the rule-registration or config-precedence mechanism changes, or a hard invariant in section 15 changes.

## 2. Overview

`adept` is an extremely fast linter and formatter for **Agent Skills** — the folder-of-instructions pattern of a `SKILL.md` file (YAML frontmatter + Markdown body) plus optional companion files sitting beside it.

It exists because skills are proliferating with no tooling to check their quality. The defects it targets are the ones that make a skill fail to trigger, over-trigger, or bloat an agent's context: vague or overlong descriptions, malformed frontmatter, token-budget overruns, broken file references, and near-duplicate skills competing for the same requests.

The shape is modelled on ruff: stable rule codes, `path:line:col: CODE message` diagnostics, fix suggestions, CI-friendly exit codes, and a single static binary. Four surfaces ship from one binary:

- **`adept check`** — static, offline lint. No network, ever.
- **`adept fmt`** — prettier-style formatting: canonical frontmatter plus full Markdown reflow. Idempotent, atomic writes.
- **`adept score`** — LLM-assisted scoring against any OpenAI-compatible endpoint: triggering accuracy, token bloat, cross-skill overlap.
- **`adept mcp`** — a JSON-RPC 2.0 stdio MCP server exposing `check_skill`, `format_skill`, and (conditionally) `score_skill`.

Architecturally it is a **cargo virtual workspace of three libraries and one binary**, with a strict dependency direction: everything depends on the core crate, nothing depends on the CLI.

```
adept_cli  (bin: `adept`)
  ├── adept_fmt ──┐
  ├── adept_score ┤
  └── adept ◄─────┘   (core: data model, parser, diagnostics, rule engine, tokenizer)
```

`adept_fmt` and `adept_score` do not know about each other. Only `adept_cli` composes them.

## 3. Technology Stack

**Language**: Rust, edition 2021, `rust-version = "1.85"`. `rust-toolchain.toml` pins channel `stable` with `clippy` and `rustfmt` components — do not hardcode a different toolchain in CI or scripts.

Versions are declared once in the root `[workspace.dependencies]` and referenced by member crates as `{ workspace = true }`. **Add new shared dependencies to the workspace table, not to a member's `[dependencies]` with an inline version.**

| Crate | Version | Used for |
|---|---|---|
| `serde` / `serde_yaml` / `serde_json` | 1 / 0.9 / 1 | Frontmatter YAML, diagnostic and report JSON, JSON-RPC, LLM payloads |
| `thiserror` | 1 | All library error enums (`AdeptError`, `FmtError`, `ScoreError`, `ConfigLoadError`) |
| `walkdir` | 2 | Skill discovery tree walk |
| `pulldown-cmark` | 0.12 | CommonMark event stream behind `adept::markdown` (lint rules + formatter AST) |
| `tiktoken-rs` | 0.12 | Token counting (`o200k_base` default, `cl100k_base` selectable) |
| `clap` | 4 (derive) | CLI argument parsing |
| `toml` | 0.8 | `adept.toml` config parsing (CLI only) |
| `reqwest` | 0.12 (json) | OpenAI-compatible HTTP client (`adept_score` only) |
| `tokio` | 1 (full) | Async runtime for `score`; the CLI builds it, `adept_score` never does |
| `async-trait` | 0.1 | `LlmClient` trait object |
| `owo-colors` | 4 | Terminal color in diagnostic rendering |
| `similar` | 2 | Unified diff for `fmt --check` / `--diff` |
| `insta` / `criterion` / `proptest` / `assert_cmd` / `predicates` / `tempfile` | dev-only | Snapshots, benchmarks, property tests, CLI integration tests |

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
    companion.rs           `discover_companion_files` — shared by SL303 and adept_score
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

crates/adept_score/        LLM SCORING — depends on adept; async throughout
  src/
    lib.rs                 ScoreOptions, ScoreError, `score_skill` (the single entry point)
    client.rs              LlmClient trait, OpenAiCompatClient, LlmConfig/ResolvedLlmConfig
    mock.rs                MockLlmClient — the only client tests may use
    prompts.rs             All prompt templates + PROMPT_VERSION
    triggering.rs          Prompt generation, judging, precision/recall/F1
    tokens.rs              Token-bloat analysis + LLM trimming suggestions
    overlap.rs             Offline Jaccard shortlist → LLM adjudication of shortlisted pairs
    report.rs              ScoreReport + human renderer

crates/adept_cli/          BINARY `adept` — composes all three libraries
  src/
    main.rs                Dispatch; owns process exit codes
    cli.rs                 clap derive structs; TokenizerArg mirror enum
    config.rs              adept.toml discovery (walk up) + parsing
    commands/{check,fmt,score,mcp}.rs
  tests/{cli.rs, fixtures/}
```

**Organizing principle**: layered by capability, not by technical role. Each sibling crate owns one user-facing surface end to end and depends on the core crate for the shared data model. Anything two surfaces need moves *down* into `adept` (this is why `text.rs` and `companion.rs` exist), never sideways between siblings.

## 5. Core Architecture Principles

These are the principles the code actually follows. Violating one is a design change, not a style preference.

1. **The core crate is the only shared vocabulary.** `Skill`, `Diagnostic`, `AdeptError`, `TokenCounter` all live in `adept`. When `adept_fmt` and `adept_score` both need a behaviour, it moves into `adept` so the two cannot drift. `text.rs` and `companion.rs` were both extracted for exactly this reason and their module docs say so.

2. **Lint findings and hard failures are different types.** A `Diagnostic` is something wrong with an otherwise-parseable skill. An `AdeptError` is an I/O or parse failure. Discovery never aborts on a bad skill: `SkillSet` keeps `skills` and `errors` side by side, and `Linter::lint_set` converts the errors into `SL001`/`SL002`/`SL003` diagnostics so a broken skill still reports rather than vanishing.

3. **Static checks are offline and fast.** `check` and `fmt` must never touch the network. Only `adept score` (and the MCP `score_skill` tool) does I/O beyond the filesystem. The perf criterion — 100 skills in under 1s, currently ~20ms — is gated in CI.

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

[score]                       # ScoreFileConfig, CLI-local
model = "gpt-4o-mini"
base-url = "https://api.openai.com/v1"
tokenizer = "o200k_base"
```

`[lint]` uses `snake_case` keys (serde default on `LintConfig`); `[fmt]` and `[score]` use `kebab-case` (`#[serde(rename_all = "kebab-case")]`). This inconsistency is real — match the existing casing of the table you are editing.

**Environment variables** (scoring only, resolved in `adept_score::LlmConfig::resolve`):

| Var | Flag | Purpose |
|---|---|---|
| `ADEPT_MODEL` | `--model` | Model identifier. Required — without it `score` exits 2 and the MCP `score_skill` tool is not advertised. |
| `ADEPT_BASE_URL` | `--base-url` | Defaults to `https://api.openai.com/v1` |
| `ADEPT_API_KEY` | *(none)* | Bearer token, if the endpoint requires one. Never accepted as a flag. |

There are no compile-time feature flags. `LintConfig`'s numeric thresholds each carry a doc comment justifying the specific number — **keep that rationale attached if you change a default.**

## 8. Command Structure & Exit Codes

`main.rs` parses with clap, resolves config from the *first* target path, dispatches to `commands::<name>::run`, and calls `std::process::exit` with the returned code. Command functions return `i32` — they never exit or panic themselves.

Global flags: `--config <PATH>`, `--no-color`, `-q/--quiet`. Color is enabled only when `!no_color && stdout().is_terminal()`, computed once in `run()` and threaded down.

**Exit code contract** — this is a public API, do not change it:

| Code | Meaning |
|---|---|
| `0` | Clean (no diagnostics; or nothing would be reformatted; or `--exit-zero`) |
| `1` | Diagnostics found (`check`) / files would be reformatted (`fmt --check`, `fmt --diff`) |
| `2` | Usage or I/O error: bad path, unreadable file, bad config, unresolvable model, runtime failure |

Each command module re-declares its own `EXIT_*` consts. Conventions per command:

- **`check`**: `--format human|json`, `--select`/`--ignore` (comma-separated or repeated; accept either a code `SL201` or a kebab name `description-too-short`), `--statistics`, `--exit-zero`, `--tokenizer`. `--select` is implemented as "disable everything not named" on top of `LintConfig::disabled` — see `apply_select_ignore`.
- **`fmt`**: `--check` (diff + exit 1), `--diff` (diff, exit 0), `--line-width`. Writes are **atomic**: temp file `.{name}.adept-tmp` in the same directory, `sync_all`, then `rename`. A failed format never clobbers the original.
- **`score`**: builds its own `tokio::runtime::Runtime` and calls `block_on`. `adept_score` never creates a runtime.
- **`mcp`**: no flags; reads stdin until EOF.

**Output**: diagnostics and reports go to stdout; every error message goes to stderr prefixed `adept: error: `. The `--quiet` flag suppresses only summary/progress lines, never diagnostics.

## 9. Core Library Surface (`adept`)

Everything public is re-exported from `lib.rs`; there are no public submodules except `reporting`, `text`, and `markdown`. Adding a type means adding it to that re-export list.

**`Skill`** is the data model: `path`, `frontmatter`, `body`, `body_line_offset`, and the complete unmodified `source`. Both `body` and `source` are retained (roughly 2× file bytes) — deliberate, since `fmt` compares against `source` byte-for-byte; halving it is a recorded deferral.

**`SkillParser`** is the format seam. `parse` has a default body that reads the file and delegates to `parse_str`, so implementors only write `parse_str`. `AnthropicSkillParser` splits on `\n` and strips a trailing `\r` per line, so LF and CRLF are handled uniformly; frontmatter is `---`-delimited, required `name`/`description`, optional `license`, everything else preserved in `Frontmatter::extra` (a `BTreeMap`, so `fmt` gets deterministic alphabetical ordering for free).

**`SkillSet::discover`** accepts a `SKILL.md` file, a skill directory, or a tree. It skips hidden directories and `target`/`node_modules`, but never excludes the root itself even if the root's own name matches.

**`TokenCounter`** holds a `&'static CoreBPE`. `token.rs::load_bpe` caches each encoding in a `OnceLock<Result<CoreBPE, String>>` — the `Result` is cached too, so a load failure is reported to every caller instead of being retried forever. `tiktoken-rs` ships `*_singleton()` helpers but they panic on failure; the hand-rolled cache exists to preserve the `Result` API. **Never construct BPE tables outside `load_bpe`.**

**`text.rs`** owns the definition of a "word" (lowercased, split on non-alphanumeric) and `jaccard`. `word_bag` and `words` share one tokenizer so the set-based and order-based callers cannot diverge.

**`markdown`** is the shared Markdown lexer, and the only markdown-aware code in the workspace. It exposes two views over the same document: `parse_document` builds the span-free `Block`/`Inline` AST the formatter re-prints, while `headings` / `link_destinations` / `inline_code_spans` return `Located<T>` values (carrying a 1-based body line) for the `SL1xx` rules, which need positions but no tree. Both views go through `markdown::parser()`, the **single** `pulldown-cmark` `Parser` construction site in the workspace — `grep -rn "Parser::new" crates/` must keep yielding exactly one hit. **Do not construct a `Parser` anywhere else**: an `Options` flag enabled for one view and not the other would let the linter and the formatter drift back into disagreeing about what a heading or a link is. Fence matching, info strings, indented code, nested brackets and reference links all come from the parser, so a markdown-aware rule never needs its own line scan.

**Known limitation**: `SKILL_FILE_NAME` is a private `const` in `skillset.rs`, not part of `SkillParser`. A parser for a format using `skill.yaml` or `AGENT.md` can therefore never be handed a file — the pluggability seam is incomplete. See `docs/BACKLOG.md`.

## 10. Formatter and Scorer Surfaces

### `adept_fmt`

Pipeline: `Skill` → canonical frontmatter string → `adept::markdown::parse_document(&skill.body)` (the shared lexer's `Block`/`Inline` AST — see §9) → `markdown::print_document` → output. The formatter owns only the printer; the AST and its builder live in the core crate. Exactly one blank line separates the closing `---` from the body.

Frontmatter order is fixed: `name`, `description`, `license` (if present), then extras alphabetically. YAML quoting is minimal-but-correct, emitted by hand in `frontmatter.rs`.

Nesting of block quotes, lists, and footnote definitions is bounded by `adept::markdown::MAX_NESTING_DEPTH = 100`; anything deeper becomes `Block::Raw` and is passed through verbatim rather than recursed into. This is a stack-overflow guard against adversarial input — keep it.

Prefer `format_skill` / `check_skill` (already-parsed `Skill`) over `format_str` / `check_str` (re-parses) whenever a `Skill` is in hand. The CLI always has one.

Documented limitations, each visible to users as an unexpected diff: reference-style link *definitions* are inlined at each use; Setext headings always become ATX; tight-list preservation is partial; text escaping covers a conservative subset. `HeadingStyle` and `StrongMarker` are single-variant placeholders kept so config files can express intent without a future breaking change — do not "simplify" them away.

### `adept_score`

Everything is `async` and reached through `&dyn LlmClient`. The crate **never creates a tokio runtime**; callers drive it.

`score_skill` is the single entry point and runs three independent analyses per `ScoreOptions`:

1. **Triggering accuracy** (`triggering.rs`) — the LLM generates N candidate prompts (half positive, half negative, `DEFAULT_NUM_PROMPTS = 10`), then a judge model sees *only the skill's name and description* (never the body, mirroring real tool selection) and predicts whether it would trigger. Reports precision/recall/F1. `seed` and `judge_samples` (majority vote) exist to damp judge variance.
2. **Token bloat** (`tokens.rs`) — description/body/companion counts via `adept::TokenCounter`, plus LLM trimming suggestions. Companion discovery is `pub use adept::discover_companion_files` — the same walk `SL303` uses, re-exported rather than reimplemented.
3. **Overlap** (`overlap.rs`) — a cheap offline `jaccard` shortlist at `DEFAULT_SIMILARITY_THRESHOLD = 0.25` over name+description, then LLM adjudication of only the shortlisted pairs. Results are filtered to pairs involving the scored skill.

Construct options via **`ScoreOptions::for_model(model, tokenizer)`**, not a struct literal. The model name has to reach both `ScoreOptions::model` and `TriggeringOptions::model`; that constructor is the one place that wiring lives, so the CLI and MCP tool cannot drift on defaults.

All prompt templates live in `prompts.rs` and are stamped into every report as `PROMPT_VERSION`. **Bump `PROMPT_VERSION` whenever a template's wording changes in a way that could shift scores**, so old and new reports can be told apart.

**Deliberate divergence**: `adept_score`'s overlap shortlist uses name+description at 0.25 (tuned for recall — it only shortlists); `SL402` uses description-only at 0.6 (tuned for precision — it emits a diagnostic). Both call `adept::text::jaccard`. `check` and `score` can therefore reach different conclusions about the same pair; this is intended, not a bug.

## 11. Rule Engine & Extension Points

`crates/adept/src/rules/mod.rs` is the file to read before touching rules.

**Three traits.** `Rule` carries identity — `code()`, `name()`, `default_severity()` — and requires `Send + Sync`. Rules are stateless unit structs, so this costs nothing and is what allows a `Registry` (hence a `Linter`) to live in a `static`. `SkillRule: Rule` checks one `Skill`; `SetRule: Rule` checks a whole `SkillSet`. Both `check` methods receive `&LintConfig` and a shared `&TokenCounter`.

**Adding a rule** — all four steps are required:

1. Add the unit struct and its `check` impl to the taxonomy file matching its code: `SL00x` → `rules/frontmatter.rs`, `SL1xx` → `structure.rs`, `SL2xx` → `description.rs`, `SL3xx` → `tokens.rs`, `SL4xx` → `cross.rs`.
2. Declare its identity with the macro, on one line next to the struct: `impl_rule!(MyRule, "SL107", "my-rule", Warning);`. Do not hand-write the three-method `impl Rule` block; import `impl_rule` from `super`.
3. Register it in `Registry::new`, in the appropriate `vec![]` literal, in code order.
4. Document it in `docs/RULES.md`. **`crates/adept/tests/docs_test.rs` fails the build if a registered code is undocumented** — this is a hard gate, not a nicety.

Then add a fixture under `crates/adept/tests/fixtures/rules/` and an insta snapshot test (§13).

**Severity and enablement** are applied by the `Linter`, never by the rule. Rules build diagnostics with their `default_severity()`; `LintConfig::apply_overrides` rewrites it afterwards. Both `disabled` and `severity_overrides` accept either the code or the kebab-case name.

**Rule codes are permanent.** `SL202` is retired (it duplicated `SL301` exactly) and its code is never reused, so an old config naming it fails closed rather than silently acquiring a new meaning. `docs/RULES.md` documents retired codes explicitly.

**`SL003` is special.** A skill with malformed frontmatter has no `Skill` to run rules against, so `SL001`/`SL002`/`SL003` are synthesized by `parse_error_diagnostic`, a `match` over `AdeptError` in `Linter::lint_set`. Its metadata lives in the module-level `const PARSE_ERROR_META` so it is still listed, documented, and disableable like any other rule. The consequence: that `match` is closed over `AdeptError`, so a custom `SkillParser` has no way to contribute its own error codes. A third rule flavour (`ParseErrorRule`) is the recorded fix.

**Output ordering** is `(path, line, column, code)`, implemented once in the public `adept::sort_diagnostics`. It is the user-visible ordering contract — call it, never re-implement the comparator at a call site.

## 12. MCP Server Contract

`crates/adept_cli/src/commands/mcp.rs`. Hand-rolled JSON-RPC 2.0 over newline-delimited stdio, protocol version `2024-11-05`. The `rmcp` SDK was deliberately not used: the surface is three tools behind `initialize` / `tools/list` / `tools/call`, and a direct implementation keeps both the dependency footprint and the risk of an SDK writing to stdout on our behalf at zero.

**The hard invariant: stdout carries only JSON-RPC response messages. Everything else goes to stderr.** `serve()` is the only function that writes to stdout. `handle_message` is pure with respect to I/O — it never touches stdin, stdout, or stderr — which is what lets tests drive it directly without spawning the binary. Any `println!` added below `serve` breaks every MCP client silently. Use `eprintln!`.

Other contract points:

- The `Linter` is built once into a `static OnceLock<Result<Linter, String>>`. `Linter::new` loads the tiktoken BPE tables, which costs far more than the lint itself; rebuilding it per tool call is not acceptable.
- `score_skill` is **conditionally advertised**: it appears in `tools/list` only when an LLM backend can actually be resolved. Called without a resolvable config it returns a structured tool error (`isError: true`) rather than hanging or panicking. Requests are bounded by `SCORE_TIMEOUT` (30s).
- `format_skill`'s `line_width` argument is validated to `MIN_LINE_WIDTH..=MAX_LINE_WIDTH` (20..=500); out-of-range or zero values return a structured tool error instead of producing degenerate one-word-per-line output.
- Notifications (messages with no `id`, e.g. `notifications/initialized`) return `None` and produce no output line.

**Known gap**: `score_skill` passes `[skill]` as the skillset, so overlap detection over MCP is inert — a skill is only ever compared against itself. Fixing it needs a directory argument the tool schema does not yet express. Recorded in `docs/BACKLOG.md` as a pre-publish decision.

## 13. Testing & Snapshot Conventions

148 tests across 14 suites, ~3.5s for the full workspace run. Tests live beside the code (`#[cfg(test)] mod tests`) for unit-level behaviour and in `tests/` for integration.

**Rule tests are snapshot tests.** Each rule gets a fixture directory under `crates/adept/tests/fixtures/rules/<sl_code>_<slug>/` containing a `SKILL.md` that triggers exactly that rule, plus an `insta` snapshot under `crates/adept/tests/snapshots/rules__snapshot_<name>.snap`. There is also a `cross_clean` fixture asserting a well-formed pair produces nothing. Review snapshot diffs — an accepted snapshot is an accepted behaviour change.

**Formatter tests** are fixture-in / snapshot-out (`crates/adept_fmt/tests/fixtures/*.md` → `snapshots/format_tests__*.snap`), one file per construct family (headings, tables, nested lists, code fences, blockquotes, HTML blocks, long prose, links + inline code, already-formatted). Plus `proptest_idempotency.rs` asserting `fmt(fmt(x)) == fmt(x)`. Note the proptest currently excludes tables, fences, HTML, and emphasis, and the broad idempotency loop is gated on `reflow_prose: false` — **prose reflow is the least-covered high-risk path in the repo.**

**No test may perform network I/O.** `adept_score` tests use `MockLlmClient` (`with_texts` seeds a FIFO queue of scripted responses); the module doc states this as a rule. `adept_cli`'s scoring test drives `run_with_client`, which exists solely as a `#[cfg(test)]` seam taking `&dyn LlmClient`.

**CLI tests** use `assert_cmd` + `predicates` against `tests/fixtures/{clean-skill,defective-skill}`.

**`docs_test.rs`** asserts every registered rule code appears in `docs/RULES.md`. If you add a rule and skip the docs, CI fails.

**Benchmark**: `crates/adept/benches/lint_100_skills.rs`, criterion, `harness = false`. Currently ~20ms. See §6 for how CI gates it.

Not covered: no fixture exercises a real skills corpus. The "runs clean-ish on anthropics/skills" acceptance criterion is verified manually by cloning the repo and running `check` against it; a vendored mini-corpus would make it a test.

## 14. Known Divergences & Deferrals

Read `docs/BACKLOG.md` for the full list. The three worth knowing before you write code:

**`SL104`'s heuristic filters are not lexing.** The URL-scheme, glob, `~`, `@scope/name` and template-placeholder filters in `rules/structure.rs` are genuine domain judgements about what a repo-relative path looks like, with one consumer. They sit *above* the shared lexer (§9) and should stay hand-written — do not try to push them into `adept::markdown`.

**Stale comment in `adept_score/src/overlap.rs`.** Its module header (lines 4–8) still claims the similarity heuristic is "implemented locally and deliberately separate" from anything in the core crate. That is no longer true — the function bodies call `adept::text::word_bag` / `adept::text::jaccard`. The *thresholds and inputs* remain deliberately divergent (see §10); only the comment is out of date. Trust the code, and fix the comment when you are next in that file.

**Performance work is deliberately not done**, since `check` runs ~20ms against a 1s target: skill discovery and the per-skill lint loop are sequential and embarrassingly parallel (`rayon` would cut wall time by roughly Ncores); `SL402`/`SL403` are each O(n²) pairwise Jaccard, fine at 100 skills but 500k pairs at 1000; hashing words to `u64` would remove the per-word `String` allocation; `Skill` retaining both `source` and `body` costs ~2× file bytes. Do not treat any of these as bugs.

## 15. Summary & Key Architectural Decisions

The things not to violate:

- **Dependency direction is one-way.** `adept_fmt` and `adept_score` depend on `adept` and never on each other. Shared behaviour moves *down* into `adept`, never sideways.
- **`check` and `fmt` never touch the network.** Only `score` (and the MCP `score_skill` tool) does.
- **MCP stdout carries only JSON-RPC.** All logging goes to stderr; `handle_message` stays I/O-pure.
- **Exit codes are a public contract**: `0` clean, `1` findings, `2` usage/I/O error.
- **Rule codes are permanent and never reused.** Retired codes stay retired and documented.
- **Every registered rule must appear in `docs/RULES.md`** — `docs_test.rs` enforces it.
- **Use `impl_rule!` for a rule's identity, register it in `Registry::new`, and let the `Linter` apply severity.** Rules never resolve their own enablement or severity.
- **Construct `ScoreOptions` via `for_model`**, and bump `PROMPT_VERSION` when prompt wording changes meaningfully.
- **Never load BPE tables outside `token.rs::load_bpe`**; build expensive objects once (`static OnceLock`), which is why `Rule: Send + Sync`.
- **`fmt` writes atomically** (temp file + rename) and is idempotent. Both properties are tested; keep them.
- **`cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass**, tests and benches included.
