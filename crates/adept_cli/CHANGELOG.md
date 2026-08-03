# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/mathiasesn/adept/compare/adept-v0.2.0...adept-v0.3.0) - 2026-08-03

### Fixed

- [**breaking**] stop credentials leaking through LlmError and base_url ([#47](https://github.com/mathiasesn/adept/pull/47))

## [0.2.0](https://github.com/mathiasesn/adept/compare/adept-v0.1.1...adept-v0.2.0) - 2026-08-03

### Added

- [**breaking**] distribute the adept CLI as a Python package ([#45](https://github.com/mathiasesn/adept/pull/45))

## [0.1.1](https://github.com/mathiasesn/adept/compare/adept-v0.1.0...adept-v0.1.1) - 2026-08-02

### Other

- release v0.1.0 ([#39](https://github.com/mathiasesn/adept/pull/39))

## [0.1.0](https://github.com/mathiasesn/adept/releases/tag/adept-v0.1.0) - 2026-08-02

### Added

- [**breaking**] rename packages for crates.io and automate releases ([#35](https://github.com/mathiasesn/adept/pull/35))

### Other

- add status badges to README ([#41](https://github.com/mathiasesn/adept/pull/41))
- adopt the adept logo ([#40](https://github.com/mathiasesn/adept/pull/40))
- tighten README, remove docs/MVP.md, drop dangling spec citations
- correct pass_rate wording, pin MCP config behaviour
- deduplicate eval orchestration, generalize MCP timeout helper
- Fix review findings in the eval surface
- Update docs for the eval unification
- Replace adept score with a unified adept eval command
- Dissolve adept_score into adept_agent as llm/ and eval/
- Simplify the create surface after review
- Correct documentation drift found by review
- Fix review findings in the create surface
- Document the create surface and the evals contract
- Add preview-only create_skill and generate_evals MCP tools
- Add the adept create CLI surface
- Rename adept_fix to adept_agent and split out the fix module
- Simplify the tracing and capture code
- Address review findings on the tracing and capture layer
- Add tracing regression tests and document the capture layer
- Wire tracing and capture into the adept CLI
- Simplify adept fix: push fix regions onto the rule trait
- Address review: conservation guard soundness and docs
- Add `adept fix` subcommand wiring adept_fix into the CLI
- Add FixKind rule tagging for LLM-fixable rules
- Unify score/mcp sibling-root discovery behind adept::sibling_root
- Give MCP score_skill real overlap detection via sibling discovery
- dedup shared logic, cache tokenizer tables, cut boilerplate
- Fix review findings across all four crates
- Add adept CLI with check, fmt, score, and MCP subcommands
- Scaffold cargo workspace and adept core crate
