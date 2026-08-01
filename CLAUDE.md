# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required reading

`docs/ARCHI.md` is the architecture source of truth — read it before any non-trivial change, and update it when a crate boundary, dependency, config mechanism, or hard invariant moves. It defers to two other docs in their own domains: `docs/RULES.md` (per-rule reference `SL001`–`SL403`, machine-checked against the registry), `docs/BACKLOG.md` (known gaps and deliberate deferrals — check here before "discovering" a bug).

## Commands

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --all-targets -- -D warnings   # enforced, tests and benches included
cargo fmt --all -- --check

cargo test -p adept rules::                  # one crate / filtered tests
cargo test -p adept_fmt --test format_tests  # a single suite
cargo insta review                           # review pending snapshot changes

cargo run -q -p adept_cli -- check <path>
cargo bench -p adept --bench lint_100_skills -- --quick
```

CI runs the four workspace commands above, then a perf smoke test that parses the criterion `lint_100_skills` line and fails above **500ms** (observed ~23ms; 1s is the acceptance criterion, 500ms is the gate).

## Architecture

Virtual cargo workspace, four crates, one binary (`adept`):

- `adept` — core: `Skill`, `SkillParser`, `SkillSet`, `Diagnostic`, `AdeptError`, `TokenCounter`, the rule engine, `markdown` (the shared pulldown-cmark lexer), and `evals` (the eval-dataset schema plus the offline `grade` function).
- `adept_fmt` — formatter: canonical frontmatter + Markdown reflow. Idempotent, atomic writes.
- `adept_agent` — LLM-assisted agent capabilities; top-of-stack, composes `adept` and `adept_fmt` (canonicalization). Houses its own LLM transport at `adept_agent::llm` (`LlmClient`, `OpenAiCompatClient`, `MockLlmClient`, `CaptureSink`, `LlmConfig`) and the four `adept eval` analyses at `adept_agent::eval` (`triggering`, `tokens`, `overlap`, `report`), both re-exported at the crate root. `fix` (autofix for `FixKind::Llm` diagnostics) is its own submodule; `candidate`/`diff`/`prompts`/`writer`/`gate` are crate-level machinery shared with `create` (generate → screen → repair → generate-evals skill authoring).
- `adept_cli` — clap CLI + `adept.toml` config + hand-rolled MCP stdio server.

Dependency direction: `adept_fmt` depends only on `adept`. `adept_agent` is top-of-stack and may compose `adept` and `adept_fmt`. Nothing in the library stack may depend on `adept_agent`; only `adept_cli` does.

Config precedence is **CLI flag > `adept.toml` > built-in default**; `adept.toml` is discovered by walking up from the target path. `[fix]`, `[eval]` and `[create]` are three independent sections (no fallback between any of them); the `ADEPT_MODEL` / `ADEPT_BASE_URL` / `ADEPT_API_KEY` env vars are the only thing they share. A config file containing a stale `[score]` section (the pre-rename name) is a hard error naming `[eval]`, not a silently-ignored table.

## Invariants (violating one is a design change, not a style choice)

- **`check`, `fmt`, and eval-dataset grading never touch the network.** The `triggering`, `token-bloat`, and `overlap` analyses do, and they run only when a model is configured; `adept eval --select evals` makes no network call and requires no API key. No test may perform network I/O — use `adept_agent::MockLlmClient`.
- **Exit codes are a public contract**: `0` clean, `1` findings, `2` usage/I/O error.
- **MCP stdout carries only JSON-RPC.** All logging goes to stderr; a stray `println!` in `commands/mcp.rs` breaks every client silently. `handle_message` stays I/O-pure. The MCP `create_skill`/`generate_evals` tools are preview-only and never write to disk; `eval_skill` is read-only (also never writes) and, unlike those two, is always advertised regardless of whether a model is configured.
- **adept spawns no subprocess, ever.** Not `check`/`fmt`/`eval`/`fix`, not `create` (the `command` eval assertion is defined and validated, never executed). Eval-dataset grading judges a `command` assertion only from a harness-supplied exit code.
- **Rule codes are permanent and never reused.** `SL202` is retired and stays retired so old configs fail closed.
- **One parser-construction site.** All markdown goes through `adept::markdown::parser()`, the sole caller of `Parser::new_ext` (`crates/adept/src/markdown/mod.rs`); `grep -rn "Parser::new" crates/` must keep yielding exactly one hit, or the linter and formatter drift on what a heading is.
- **Never load BPE tables outside `token.rs::load_bpe`** (caches the `Result` too). Expensive objects are built once in `static OnceLock`, which is why `Rule: Send + Sync`.
- **`fmt` writes atomically** (temp file + rename) and is idempotent; both are tested.
- `Diagnostic` = something wrong with a parseable skill; `AdeptError` = I/O/parse failure. Discovery never aborts — errors become `SL001`/`SL002`/`SL003` diagnostics.
- Sort output via `adept::sort_diagnostics` — never re-implement the `(path, line, column, code)` comparator.
- Construct `EvalOptions` via `EvalOptions::for_model`, and bump `PROMPT_VERSION` in `adept_agent::eval::prompts` when template wording could shift scores.

## Adding a rule (all four steps required)

1. Add the unit struct + `check` impl to the taxonomy file matching its code: `SL00x` → `rules/frontmatter.rs`, `SL1xx` → `structure.rs`, `SL2xx` → `description.rs`, `SL3xx` → `tokens.rs`, `SL4xx` → `cross.rs`.
2. `impl_rule!(MyRule, "SL107", "my-rule", Warning);` on one line next to the struct — never hand-write the `impl Rule` block. The optional trailing argument sets `fix_kind`, which defaults to `FixKind::None`:
   - `, Deterministic` — fixable without an LLM.
   - `, Llm, Description` or `, Llm, Body` — opts the rule into `adept fix`; the second ident is the `FixRegion` (those two variants are the only ones) telling `adept_agent` which part of the skill to rewrite.

   This is metadata only: `adept_agent` hard-codes no rule list, and a unit test pins the tagged set.
3. Register it in `Registry::new`, in the matching `vec![]`, in code order.
4. Document it in `docs/RULES.md` — `crates/adept/tests/docs_test.rs` fails the build otherwise.

Then add a fixture under `crates/adept/tests/fixtures/rules/<sl_code>_<slug>/` and an insta snapshot. Severity and enablement are applied by the `Linter`, never by the rule.

## Testing conventions

Rule tests and formatter tests are fixture-in / insta-snapshot-out; an accepted snapshot is an accepted behaviour change, so read the diff. Formatter fixtures are one file per construct family, plus `proptest_idempotency.rs`. Note prose reflow is the least-covered high-risk path — the broad idempotency loop is gated on `reflow_prose: false`. CLI tests use `assert_cmd`/`predicates` against `crates/adept_cli/tests/fixtures/{clean,defective}-skill`.

Dependency versions live once in the root `[workspace.dependencies]`; members reference them with `{ workspace = true }`.

Everything below the `<!-- rtk-instructions -->` marker is generated by `rtk init` — edit it there, not here.

<!-- rtk-instructions v2 -->
# RTK (Rust Token Killer) - Token-Optimized Commands

## Golden Rule

**Always prefix commands with `rtk`**. If RTK has a dedicated filter, it uses it. If not, it passes through unchanged. This means RTK is always safe to use.

**Important**: Even in command chains with `&&`, use `rtk`:
```bash
# ❌ Wrong
git add . && git commit -m "msg" && git push

# ✅ Correct
rtk git add . && rtk git commit -m "msg" && rtk git push
```

## RTK Commands by Workflow

### Build & Compile (80-90% savings)
```bash
rtk cargo build         # Cargo build output
rtk cargo check         # Cargo check output
rtk cargo clippy        # Clippy warnings grouped by file (80%)
rtk tsc                 # TypeScript errors grouped by file/code (83%)
rtk lint                # ESLint/Biome violations grouped (84%)
rtk prettier --check    # Files needing format only (70%)
rtk next build          # Next.js build with route metrics (87%)
```

### Test (60-99% savings)
```bash
rtk cargo test          # Cargo test failures only (90%)
rtk go test             # Go test failures only (90%)
rtk jest                # Jest failures only (99.5%)
rtk vitest              # Vitest failures only (99.5%)
rtk playwright test     # Playwright failures only (94%)
rtk pytest              # Python test failures only (90%)
rtk rake test           # Ruby test failures only (90%)
rtk rspec               # RSpec test failures only (60%)
rtk test <cmd>          # Generic test wrapper - failures only
```

### Git (59-80% savings)
```bash
rtk git status          # Compact status
rtk git log             # Compact log (works with all git flags)
rtk git diff            # Compact diff (80%)
rtk git show            # Compact show (80%)
rtk git add             # Ultra-compact confirmations (59%)
rtk git commit          # Ultra-compact confirmations (59%)
rtk git push            # Ultra-compact confirmations
rtk git pull            # Ultra-compact confirmations
rtk git branch          # Compact branch list
rtk git fetch           # Compact fetch
rtk git stash           # Compact stash
rtk git worktree        # Compact worktree
```

Note: Git passthrough works for ALL subcommands, even those not explicitly listed.

### GitHub (26-87% savings)
```bash
rtk gh pr view <num>    # Compact PR view (87%)
rtk gh pr checks        # Compact PR checks (79%)
rtk gh run list         # Compact workflow runs (82%)
rtk gh issue list       # Compact issue list (80%)
rtk gh api              # Compact API responses (26%)
```

### JavaScript/TypeScript Tooling (70-90% savings)
```bash
rtk pnpm list           # Compact dependency tree (70%)
rtk pnpm outdated       # Compact outdated packages (80%)
rtk pnpm install        # Compact install output (90%)
rtk npm run <script>    # Compact npm script output
rtk npx <cmd>           # Compact npx command output
rtk prisma              # Prisma without ASCII art (88%)
```

### Files & Search (60-75% savings)
```bash
rtk ls <path>           # Tree format, compact (65%)
rtk read <file>         # Code reading with filtering (60%)
rtk grep <pattern>      # Search grouped by file (75%). Format flags (-c, -l, -L, -o, -Z) run raw.
rtk find <pattern>      # Find grouped by directory (70%)
```

### Analysis & Debug (70-90% savings)
```bash
rtk err <cmd>           # Filter errors only from any command
rtk log <file>          # Deduplicated logs with counts
rtk json <file>         # JSON structure without values
rtk deps                # Dependency overview
rtk env                 # Environment variables compact
rtk summary <cmd>       # Smart summary of command output
rtk diff                # Ultra-compact diffs
```

### Infrastructure (85% savings)
```bash
rtk docker ps           # Compact container list
rtk docker images       # Compact image list
rtk docker logs <c>     # Deduplicated logs
rtk kubectl get         # Compact resource list
rtk kubectl logs        # Deduplicated pod logs
```

### Network (65-70% savings)
```bash
rtk curl <url>          # Compact HTTP responses (70%)
rtk wget <url>          # Compact download output (65%)
```

### Meta Commands
```bash
rtk gain                # View token savings statistics
rtk gain --history      # View command history with savings
rtk discover            # Analyze Claude Code sessions for missed RTK usage
rtk proxy <cmd>         # Run command without filtering (for debugging)
rtk init                # Add RTK instructions to CLAUDE.md
rtk init --global       # Add RTK to ~/.claude/CLAUDE.md
```

## Token Savings Overview

| Category | Commands | Typical Savings |
|----------|----------|-----------------|
| Tests | vitest, playwright, cargo test | 90-99% |
| Build | next, tsc, lint, prettier | 70-87% |
| Git | status, log, diff, add, commit | 59-80% |
| GitHub | gh pr, gh run, gh issue | 26-87% |
| Package Managers | pnpm, npm, npx | 70-90% |
| Files | ls, read, grep, find | 60-75% |
| Infrastructure | docker, kubectl | 85% |
| Network | curl, wget | 65-70% |

Overall average: **60-90% token reduction** on common development operations.
<!-- /rtk-instructions -->
