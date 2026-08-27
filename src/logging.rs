//! Fx-style console logging for modrun framework events.
//!
//! Framework events use target `modrun`. Call [`init`] once at startup to print
//! lines like `[modrun] PROVIDE ...` without timestamps or tracing metadata.
//!
//! ```no_run
//! modrun::logging::init();
//! ```

use tracing_subscriber::EnvFilter;

/// Install a minimal tracing subscriber for modrun framework events.
///
/// Respects `RUST_LOG` when set; otherwise defaults to `modrun=info`.
pub fn init() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("modrun=info"));
    init_with_filter(filter);
}

/// Install a minimal tracing subscriber with an explicit filter.
pub fn init_with_filter(filter: EnvFilter) {
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_ansi(true)
        .init();
}
