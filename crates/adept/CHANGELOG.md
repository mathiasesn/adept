# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/mathiasesn/adept/compare/adept-core-v0.1.0...adept-core-v0.1.1) - 2026-08-02

### Other

- release v0.1.0 ([#39](https://github.com/mathiasesn/adept/pull/39))

## [0.1.0](https://github.com/mathiasesn/adept/releases/tag/adept-core-v0.1.0) - 2026-08-02

### Added

- [**breaking**] rename packages for crates.io and automate releases ([#35](https://github.com/mathiasesn/adept/pull/35))

### Fixed

- route parse errors through the ordinary rule pipeline

### Other

- add status badges to README ([#41](https://github.com/mathiasesn/adept/pull/41))
- adopt the adept logo ([#40](https://github.com/mathiasesn/adept/pull/40))
- simplify docs_test — shared loader, aggregate failure reports
- normalise BACKLOG citation style, wrap to 80, pin both with tests
- tighten README, remove docs/MVP.md, drop dangling spec citations
- address cleanup review findings
- correct pass_rate wording, pin MCP config behaviour
- deduplicate the SL4xx pairwise scan and SL205's word split
- deduplicate eval orchestration, generalize MCP timeout helper
- Fix review findings in the eval surface
- Update docs for the eval unification
- Replace adept score with a unified adept eval command
- Add an offline eval-dataset grader to adept::evals
- Simplify the create surface after review
- Correct documentation drift found by review
- Fix review findings in the create surface
- Document the create surface and the evals contract
- Add the eval-dataset schema and the evals/ exemption predicate
- Rename adept_fix to adept_agent and split out the fix module
- Make rule snapshots location-independent
- Remove accidentally committed insta pending snapshots
- Add tracing regression tests and document the capture layer
- Add tracing and raw LLM payload capture to adept_score
- Simplify adept fix: push fix regions onto the rule trait
- Address review: conservation guard soundness and docs
- Add FixKind rule tagging for LLM-fixable rules
- Implement exemption for OOXML archive-internal paths in SL104 rule
- Simplify has_creation_intent to reuse shared text::words tokenizer
- Address SL104 review: exclude modification verbs, document RULES.md
- Exempt skill-authored paths from SL104 broken-file-reference
- Relocate is_license_file to companion.rs, simplify SL303 guard
- Exempt bundled license files from SL303 companion-file-bloat
- Unify score/mcp sibling-root discovery behind adept::sibling_root
- Snapshot the linter's output over the vendored corpus
- Vendor an Apache-2.0 skills corpus fixture
- share a positioned-event iterator, collapse the fmt shim
- Fix SL105 false negative on hash-prefixed setext headings
- Add regression tests for the markdown-lexing bugs
- Rewrite SL1xx rules on the shared parser, add SL105
- Move markdown AST into core, add positioned query API
- dedup shared logic, cache tokenizer tables, cut boilerplate
- Fix review findings across all four crates
- Add adept CLI with check, fmt, score, and MCP subcommands
- Add lint rule engine and initial SL0/1/2/3/4xx rule set
- Scaffold cargo workspace and adept core crate
