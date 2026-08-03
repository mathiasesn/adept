# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/mathiasesn/adept/compare/adept-fmt-v0.1.1...adept-fmt-v0.2.0) - 2026-08-03

### Added

- [**breaking**] distribute the adept CLI as a Python package ([#45](https://github.com/mathiasesn/adept/pull/45))

## [0.1.1](https://github.com/mathiasesn/adept/compare/adept-fmt-v0.1.0...adept-fmt-v0.1.1) - 2026-08-02

### Other

- release v0.1.0 ([#39](https://github.com/mathiasesn/adept/pull/39))

## [0.1.0](https://github.com/mathiasesn/adept/releases/tag/adept-fmt-v0.1.0) - 2026-08-02

### Added

- [**breaking**] rename packages for crates.io and automate releases ([#35](https://github.com/mathiasesn/adept/pull/35))

### Other

- add status badges to README ([#41](https://github.com/mathiasesn/adept/pull/41))
- adopt the adept logo ([#40](https://github.com/mathiasesn/adept/pull/40))
- tighten README, remove docs/MVP.md, drop dangling spec citations
- Update docs for the eval unification
- Correct documentation drift found by review
- Document the create surface and the evals contract
- Rename adept_fix to adept_agent and split out the fix module
- Add tracing regression tests and document the capture layer
- Simplify adept fix: push fix regions onto the rule trait
- Address review: conservation guard soundness and docs
- Tidy reflow guard and its tests
- Cover tilde code fences in reflow guard; firm up its tests
- Extend reflow guard to thematic breaks and setext underlines
- Harden reflow against leaning-toothpick line starts
- Dedup the reflow repro tests behind a shared helper
- Collapse build_tokens dispatch, drop the unreachable arm
- Guard reflow idempotency over the corpus, fix escape-split words
- share a positioned-event iterator, collapse the fmt shim
- Document SL105 and the unified markdown module
- Move markdown AST into core, add positioned query API
- dedup shared logic, cache tokenizer tables, cut boilerplate
- Fix review findings across all four crates
- Add adept CLI with check, fmt, score, and MCP subcommands
- Add adept_fmt markdown and frontmatter formatter
- Scaffold cargo workspace and adept core crate
