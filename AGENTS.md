# AGENTS.md

Guidance for AI coding agents working in this repository. `AGENTS.md` is the tool-agnostic convention read natively by Claude Code, Codex CLI, GitHub Copilot, Cursor, Zed and others; `CLAUDE.md` is a symlink to this file, kept so Claude Code's native path still resolves.

## What adept is

A linter, formatter, and LLM-assisted authoring tool for agent skills (`SKILL.md` files). One binary, six subcommands:

| Command | Does | Network |
|---|---|---|
| `check` | Lints skills against the `SL*` rule registry | never |
| `fmt` | Rewrites skills into canonical form, in place | never |
| `eval` | Scores triggering, token bloat, overlap, and eval datasets | yes, except `--select evals` |
| `fix` | Rewrites skills to clear `FixKind::Llm` diagnostics | yes |
| `create` | Generates a new skill + synthetic eval dataset from a brief | yes |
| `mcp` | Serves the above over MCP on stdio | depends on tool |

## Required reading

`docs/ARCHI.md` is the architecture source of truth — read it before any non-trivial change, and update it when a crate boundary, dependency, config mechanism, or hard invariant moves. It defers to three docs in their own domains:

- `docs/RULES.md` — per-rule reference `SL001`–`SL403`, machine-checked against the registry.
- `docs/EVALS.md` — eval-dataset schema (`evals.jsonl` / `results.jsonl`) and grading semantics.
- `docs/BACKLOG.md` — known gaps and deliberate deferrals. Check here before "discovering" a bug.

## Commands

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --all-targets -- -D warnings   # enforced, tests and benches included
cargo fmt --all -- --check

cargo test -p adept-core rules::             # one crate / filtered tests
cargo test -p adept-fmt --test format_tests  # a single suite
cargo insta review                           # review pending snapshot changes

cargo run -q -p adept -- check <path>
cargo bench -p adept-core --bench lint_100_skills -- --quick
```

CI runs the four workspace commands above, then a perf smoke test that parses the criterion `lint_100_skills` line and fails above **500ms** (observed ~23ms; 1s is the acceptance criterion, 500ms is the gate), then a separate `python-packaging` job (ubuntu-only) that builds the maturin wheel, installs it with `uv`, and asserts `adept --version` and `python -m adept --version` agree and match `[workspace.package].version`, then runs `python/tests` under pytest (the one test suite `cargo test --workspace` does not reach).

## Architecture

Virtual cargo workspace, four crates, one binary (`adept`):

- `adept` (package `adept-core`, directory `crates/adept`) — core: `Skill`, `SkillParser`, `SkillSet`, `Diagnostic`, `AdeptError`, `TokenCounter`, the rule engine, `markdown` (the shared pulldown-cmark lexer), and `evals` (the eval-dataset schema plus the offline `grade` function).
- `adept_fmt` (package `adept-fmt`) — formatter: canonical frontmatter + Markdown reflow. Idempotent, atomic writes.
- `adept_agent` (package `adept-agent`) — everything LLM-assisted. Submodules:
  - `llm` — the transport and its config resolution: `LlmClient`, `OpenAiCompatClient`, `MockLlmClient`, `CaptureSink`, `LlmConfig`, `ConfigError`/`llm_available`, `ScrubbedBody`.
  - `eval` — the four `adept eval` analyses: `triggering`, `tokens`, `overlap`, `report`.
  - `fix` — autofix for `FixKind::Llm` diagnostics.
  - `create` — skill authoring: generate → screen → repair → generate-evals.

  `llm` and `eval` are re-exported at the crate root. `candidate`/`diff`/`prompts`/`writer`/`gate` sit at crate level because `fix` and `create` share them.
- `adept_cli` (package `adept`, directory `crates/adept_cli`) — clap CLI + `adept.toml` config + hand-rolled MCP stdio server.

Dependency direction is strictly layered: `adept` ← `adept_fmt` ← `adept_agent` ← `adept_cli` (crate names; the equivalent package names are `adept-core` ← `adept-fmt` ← `adept-agent` ← `adept`). `adept_agent` may compose `adept` and `adept_fmt`; nothing in the library stack may depend on `adept_agent`, only `adept_cli` does.

Config precedence is **CLI flag > `adept.toml` > built-in default**; `adept.toml` is discovered by walking up from the target path. `[fix]`, `[eval]` and `[create]` are three independent sections (no fallback between any of them); the `ADEPT_MODEL` / `ADEPT_BASE_URL` / `ADEPT_API_KEY` env vars are the only thing they share. A config file containing a stale `[score]` section (the pre-rename name) is a hard error naming `[eval]`, not a silently-ignored table.

## Invariants (violating one is a design change, not a style choice)

- **`check`, `fmt`, and eval-dataset grading never touch the network.** The `triggering`, `token-bloat`, and `overlap` analyses do, and they run only when a model is configured; `adept eval --select evals` makes no network call and requires no API key. No test may perform network I/O — use `adept_agent::MockLlmClient`.
- **Exit codes are a public contract**: `0` clean, `1` findings, `2` usage/I/O error.
- **MCP stdout carries only JSON-RPC.** All logging goes to stderr; a stray `println!` in `commands/mcp.rs` breaks every client silently. `handle_message` stays I/O-pure. The MCP `create_skill`/`generate_evals` tools are preview-only and never write to disk; `eval_skill` is read-only (also never writes) and, unlike those two, is always advertised regardless of whether a model is configured.
- **The adept binary spawns no subprocess, ever.** Not `check`/`fmt`/`eval`/`fix`, not `create` (the `command` eval assertion is defined and validated, never executed). Eval-dataset grading judges a `command` assertion only from a harness-supplied exit code. The rule is about the Rust binary; the Python distribution shim that dispatches *to* it (`python/adept/__main__.py`) is out of its scope — see `docs/ARCHI.md` §6.
- **Rule codes are permanent and never reused.** `SL202` is retired and stays retired so old configs fail closed.
- **One parser-construction site.** All markdown goes through `adept::markdown::parser()`, the sole caller of `Parser::new_ext` (`crates/adept/src/markdown/mod.rs`); `grep -rn "Parser::new" crates/` must keep yielding exactly one hit, or the linter and formatter drift on what a heading is.
- **Never load BPE tables outside `token.rs::load_bpe`** (caches the `Result` too). Expensive objects are built once in `static OnceLock`, which is why `Rule: Send + Sync`.
- **`fmt` writes atomically** (temp file + rename) and is idempotent; both are tested.
- `Diagnostic` = something wrong with a parseable skill; `AdeptError` = I/O/parse failure. Discovery never aborts — errors become `SL001`/`SL002`/`SL003` diagnostics.
- Sort output via `adept::sort_diagnostics` — never re-implement the `(path, line, column, code)` comparator.
- Build the options structs via their `for_model` constructors (`EvalOptions`, `CreateOptions`, `FixOptions`) rather than struct literals, so defaults stay in one place. Bump `PROMPT_VERSION` in `adept_agent::eval::prompts` whenever template wording could shift scores.
- **A credential-bearing `base_url` is rejected at config resolution, not scrubbed at each egress.** `LlmConfig::resolve` returns `ConfigError::CredentialsInBaseUrl` when the resolved `base_url` parses with a non-empty username or any password, closing by construction the three egresses userinfo previously reached.

## Adding a rule (all five steps required)

1. Add the unit struct + `check` impl to the taxonomy file matching its code: `SL00x` → `rules/frontmatter.rs`, `SL1xx` → `structure.rs`, `SL2xx` → `description.rs`, `SL3xx` → `tokens.rs`, `SL4xx` → `cross.rs`.
2. `impl_rule!(MyRule, "SL107", "my-rule", Warning);` on one line next to the struct — never hand-write the `impl Rule` block. The optional trailing argument sets `fix_kind`, which defaults to `FixKind::None`:
   - `, Deterministic` — fixable without an LLM.
   - `, Llm, Description` or `, Llm, Body` — opts the rule into `adept fix`; the second ident is the `FixRegion` (those two variants are the only ones) telling `adept_agent` which part of the skill to rewrite.

   This is metadata only: `adept_agent` hard-codes no rule list, and a unit test pins the tagged set.
3. Register it in `Registry::new`, in the matching `vec![]`, in code order.
4. Document it in `docs/RULES.md` — `crates/adept/tests/docs_test.rs` fails the build otherwise.
5. Add a fixture under `crates/adept/tests/fixtures/rules/<sl_code>_<slug>/` and an insta snapshot.

The rule itself decides nothing about severity or enablement — the `Linter` applies both.

## Testing conventions

- Rule and formatter tests are fixture-in / insta-snapshot-out. **An accepted snapshot is an accepted behaviour change** — read the diff before `cargo insta review` accepts it.
- Formatter fixtures are one file per construct family, plus `proptest_idempotency.rs`. Prose reflow is the least-covered high-risk path: the broad idempotency loop is gated on `reflow_prose: false`, so changes there need their own targeted tests.
- CLI tests use `assert_cmd`/`predicates` against `crates/adept_cli/tests/fixtures/{clean,defective}-skill`.
- No test may perform network I/O — use `adept_agent::MockLlmClient`.

Dependency versions live once in the root `[workspace.dependencies]`; members reference them with `{ workspace = true }`.

## Commit conventions

Releases are automated via release-plz, driven by Conventional Commits. `feat:`/`fix:` bump as expected; `refactor:` does not bump by default, but refactors here have historically been breaking, so a breaking change needs `!` (e.g. `refactor!:`) or a `BREAKING CHANGE:` footer — no exceptions.

Never hand-edit `[workspace.package].version` and merge to `main`: the release job keys off whether that version already exists on crates.io, not off which PR changed it, so a manual bump fires a real publish outside the release PR. Version bumps come only from the release PR release-plz opens.

Everything below the `<!-- rtk-instructions -->` marker is generated by `rtk init` — edit it there, not here. `rtk init` targets `CLAUDE.md`, which is a symlink to this file, so regeneration lands here.

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
