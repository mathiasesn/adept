//! The process-wide `tracing` subscriber for the `adept` binary.
//!
//! Two hard rules, both load-bearing:
//!
//! 1. **The writer is stderr, never stdout.** `tracing_subscriber::fmt()`
//!    defaults to stdout, and `adept mcp` speaks JSON-RPC on stdout — a
//!    single log line there breaks every MCP client silently. Every
//!    subcommand, `mcp` included, gets the same stderr-routed subscriber.
//! 2. **Only `main` installs a subscriber.** The library crates emit events
//!    but never initialise a global collector.
//!
//! Level selection follows the `ty` shape: repeated `-v` flags pick a
//! default level (off / info / debug / trace), and the `ADEPT_LOG`
//! environment variable overrides them wholesale using `EnvFilter`
//! directive syntax (e.g. `ADEPT_LOG=adept_score::client=trace`). Adept
//! uses its own `ADEPT_*` namespace rather than `RUST_LOG`.
//!
//! With no `-v` and no `ADEPT_LOG` the filter is `off`, so stdout *and*
//! stderr are byte-identical to a build with no logging at all.

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::EnvFilter;

/// The environment variable that overrides the `-v` derived level, using
/// full `EnvFilter` directive syntax.
pub const ENV_LOG: &str = "ADEPT_LOG";

/// Install the global stderr subscriber for this process.
///
/// `verbosity` is the `-v` occurrence count: `0` off, `1` info, `2` debug,
/// `3`+ trace. A non-empty [`ENV_LOG`] value replaces that entirely.
///
/// Called once from `main` before subcommand dispatch. Installation
/// failures (only possible if a subscriber is already set) are ignored:
/// logging is diagnostics, never a reason to fail a run.
pub fn init(verbosity: u8) {
    let filter = match std::env::var(ENV_LOG) {
        Ok(directives) if !directives.trim().is_empty() => EnvFilter::new(directives),
        _ => default_filter(verbosity),
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        // Explicit: the default is stdout, which MCP owns.
        .with_writer(std::io::stderr)
        // stderr may be a pipe or a log file; never emit escape codes.
        .with_ansi(false)
        .with_target(true)
        .try_init();
}

/// The filter used when [`ENV_LOG`] is unset, scoped to adept's own crates
/// so `-vvv` doesn't drown the user in `hyper`/`reqwest` internals.
fn default_filter(verbosity: u8) -> EnvFilter {
    let level = match verbosity {
        0 => return EnvFilter::default().add_directive(LevelFilter::OFF.into()),
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    EnvFilter::new(format!(
        "adept={level},adept_cli={level},adept_fmt={level},adept_score={level},adept_agent={level}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbosity_zero_is_off() {
        assert_eq!(default_filter(0).max_level_hint(), Some(LevelFilter::OFF));
    }

    #[test]
    fn verbosity_maps_to_increasing_levels() {
        assert_eq!(default_filter(1).max_level_hint(), Some(LevelFilter::INFO));
        assert_eq!(default_filter(2).max_level_hint(), Some(LevelFilter::DEBUG));
        assert_eq!(default_filter(3).max_level_hint(), Some(LevelFilter::TRACE));
        assert_eq!(default_filter(9).max_level_hint(), Some(LevelFilter::TRACE));
    }
}
