# Unify markdown parsing between linter and formatter

## Problem / Why

`crates/adept/src/rules/structure.rs` hand-rolls a line scanner for ATX headings,
fence tracking, and `](...)` link extraction — the fence-toggle loop appears three
times within that one file (`headings`, `BrokenFileReference::check`, and inline in
`extract_link_targets`'s caller). Meanwhile `crates/adept_fmt/src/markdown/build.rs`
already has a `pulldown-cmark`-backed AST with correct fence, indented-code,
nested-bracket and reference-link handling.

Consequences today:

- `SL102`/`SL103`/`SL104` and `adept fmt` hold two different definitions of
  "heading" and "link". The formatter can rewrite a link the linter cannot see.
- The line scanner is wrong in known ways the AST is not: it toggles `in_fence` on
  any line starting with ` ``` `/`~~~` (so it mismatches fence characters and
  ignores fence length and info strings), it never recognises indented code blocks,
  it cannot see setext headings, and `extract_link_targets` does naive `](` →
  first-`)` scanning that breaks on nested parentheses in a destination.
- Every new markdown-aware rule copy-pastes fence tracking again.

## Goals

- One markdown *lexer* — a single `pulldown-cmark` parser construction and event
  interpretation — underlying both the `SL1xx` rules and `adept fmt`, so the two
  can no longer disagree about what a heading or a link is.
- `SL102`, `SL103`, `SL104` produce their diagnostics from that shared lexer rather
  than from a bespoke line scan.
- Diagnostics keep accurate `line` values (currently `skill.body_line_offset + …`),
  so user-facing output does not regress.
- `structure.rs`'s duplicated fence/heading/link lexing is deleted, not merely
  wrapped.
- Setext headings become visible to `SL102`/`SL103` (accepted as a bugfix — see
  below), and a new `SL105 setext-heading` (Info) flags them as a style issue.

## Non-goals

- Changing SL104's **heuristic filters** (`is_intended_file_reference`,
  `looks_like_explicit_path`, `KNOWN_EXTENSIONS`). Per the backlog these are
  genuine domain judgements about what a repo-relative path looks like and must
  stay. Only the lexing beneath them is in scope.
- Resolving the 7 residual SL104 false positives (zip-internal paths, template
  filenames). Separate backlog item; if unification happens to change that count,
  record it, don't chase it.
- Any formatter output change. `adept fmt` behaviour is to remain byte-identical.
- Fixing the documented formatter limitations (reference-link definitions, setext
  → ATX, tight lists, escaping).
- Performance work (`rayon`, O(n²) Jaccard, `Skill` memory).

## Constraints

- Rust workspace, edition 2021, `rust-version = 1.85`, four crates.
- `adept_fmt`, `adept_score`, `adept_cli` all depend on `adept`; `adept` depends on
  none of them. Any shared module must not invert that.
- `adept` core currently has **no** `pulldown-cmark` dependency. `adept_fmt` has it
  at workspace version `0.12`.
- `adept check` runs ~21ms against a 1s target; the CI perf gate gives ~23ms against
  a 500ms threshold. Swapping a line scan for a full AST parse must stay well inside
  that.
- `build.rs` has a `MAX_NESTING_DEPTH` stack-overflow guard with a
  `collect_raw_until_end` fallback; whatever moves must preserve it.

## Proposed approach

**Layout: a module inside `adept` core**, not a new `adept_md` crate. `adept_fmt`
already depends on `adept`, so there is no cycle to dodge, and a fifth crate for
~400 lines buys compile-time isolation nothing needs. This adds `pulldown-cmark` to
`adept`'s dependencies — accepted deliberately: `adept` is the lint engine and
linting is inherently markdown-aware, and `adept_fmt` already pulls the same
workspace version. Extracting a module into a crate later is mechanical if the dep
becomes a real problem.

**Positions: a separate span-carrying query API**, with the formatter's AST left
span-free and untouched. What must be shared to fix the linter/formatter
disagreement is the *lexer* — the `pulldown-cmark` `Options` set and the event
interpretation — not the `Block` struct. So the new module exposes two views over
one parser construction:

- `parse_document(src) -> Vec<Block>` — moved from `adept_fmt`, unchanged, still
  span-free. The formatter re-prints whole documents and has no use for spans.
- A positioned query API built on `Parser::into_offset_iter()`, e.g.
  `headings(src) -> Vec<Located<Heading>>` and
  `link_destinations(src) -> Vec<Located<String>>`, where `Located<T>` carries a
  1-based line within the body. `Heading` carries `level`, `text`, and the
  ATX-vs-setext distinction (`pulldown_cmark` does not expose this directly; derive
  it by inspecting the first byte of the heading's source range, which
  `into_offset_iter` provides) — `SL105` below needs it.

Rejected: putting a span field (required or `Option`) on `Block`/`Inline`. It would
touch every construction site in `build.rs` and every match in `print.rs` to produce
a field the formatter can only ignore — a large, risky edit against a hard
constraint that formatter output stay byte-identical. An `Option<Range>` is worse
still, since a rule that silently reports line 0 from a `None` is a nastier failure
than a compile error.

The cost accepted here is two traversal functions rather than one. That is
tolerable because the part that must not drift — parser construction with its
feature flags, and fence/indented-code handling, which comes free from
`pulldown-cmark` — lives in one place beneath both views. Drift in the thin view
layer cannot reintroduce the class of bug being fixed.

**Setext headings: accepted as a bugfix, plus a new rule.** The old scanner counts
`#` characters and so cannot see setext headings at all, while `adept fmt` sees them
and rewrites them to ATX. A file titled `Title\n=====` therefore gets an `SL102
missing-h1` warning today that is simply wrong. After unification `pulldown-cmark`
reports setext headings, so:

- `SL102` stops false-positiving on setext-titled files.
- `SL103` sees the true heading sequence, and may newly report level skips it
  previously missed because part of the chain was invisible.
- A new **`SL105` `setext-heading` (Info)** flags setext headings as a style issue,
  since `adept fmt` will rewrite them to ATX anyway. This makes linter and formatter
  agree on *style*, not merely on lexing.

`Info` is the right severity: it is a style preference the formatter already
resolves automatically, so it should not fail CI by default.

Consumers: `SL102`/`SL103` use `headings()` (level + text + line);  `SL104` uses
`link_destinations()` plus inline-code spans, with fenced and indented code excluded
by the parser rather than a manual toggle. SL104's existing filter functions
(`is_intended_file_reference`, `looks_like_explicit_path`) then apply unchanged to
those destinations.

## Acceptance criteria

- `crates/adept/src/rules/structure.rs` contains no fence-tracking loop, no
  `#`-counting heading scan, and no `](` byte scan.
- Both views (`parse_document` and the positioned query API) live in the new core
  module and share one parser-construction site; `adept_fmt` calls into it rather
  than constructing its own `Parser`. `grep -rn "Parser::new" crates/` yields
  exactly one hit.
- `cargo test --workspace` passes; formatter insta snapshots and the reflow
  proptest are unchanged.
- `adept_fmt`'s snapshot and proptest suites pass with no snapshot accepted — the
  formatter's own AST and printer are not modified by this task.
- SL104 finding count on the anthropics/skills corpus does not increase above the
  current 7. This is a **manual** step: the corpus is not vendored (see
  `docs/ARCHI.md:336`), so it means cloning `anthropics/skills` and running `check`
  before and after, recording both counts in the PR description.
- `cargo bench`/the CI perf gate stays under its threshold.
- `docs/RULES.md` gains a `### SL105 \`setext-heading\` (Info)` entry —
  machine-enforced by `crates/adept/tests/docs_test.rs`, which asserts every
  registered rule is documented by code *and* by name.
- `docs/ARCHI.md` is updated: the "Markdown parsing is implemented twice" section
  (~line 342) and its "do not copy-paste the fence tracker a fourth time" warning
  must go, replaced by a pointer to the shared module. The `SL1xx` taxonomy line
  (~line 93) and the rule-authoring steps (~line 290) stay accurate for `SL105`.
- `docs/BACKLOG.md` drops the resolved item.
- New regression tests in `crates/adept/tests/rules.rs` cover each case the old
  scanner mis-lexed, since these are precisely the behaviours the refactor claims
  to fix and are otherwise untested:
  - mismatched fence characters (` ``` ` opened, `~~~` "closed")
  - fenced blocks whose info string contains a `#`
  - indented (4-space) code blocks containing heading-like or link-like text
  - a link destination containing nested parentheses
  - setext headings for `SL102`, `SL103`, and `SL105`

## Open questions

_(none)_

## Risks

- **Silent lint behaviour change.** A correct parser sees headings and links the
  old scanner missed (setext headings, links inside emphasis, indented code
  correctly excluded). Some of these are fixes; all are user-visible changes to
  `adept check` output and need to be deliberate, not incidental. Setext is
  now decided explicitly above; the remaining cases (links inside emphasis, links
  the old `](`-scan mis-terminated on nested parens) should be enumerated in the
  PR description with a before/after count, not just asserted to be improvements.
- **`SL103` may get noisier.** Seeing previously-invisible headings can surface
  level skips in files that were quiet before. This is correct behaviour, but it
  means the change is not purely additive for existing users.
- **Two views can drift.** Mitigated by construction (shared parser setup), but a
  future feature flag added to one view and not the other would reintroduce
  disagreement. Worth a comment at the parser-construction site saying so.
- **Dependency creep into core.** Accepted above, but noted: `adept_score` and
  `adept_cli` now transitively pull a markdown parser they do not use.
- **Perf.** Full AST parse per skill vs. a single line pass. Expected to be fine at
  21ms, but it is a real constant-factor increase on the hot path.
