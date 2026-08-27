//! Fx-style console logging for modrun framework events.
//!
//! Framework events use target `modrun`. Call [`init`] or [`try_init`] from
//! examples and local binaries. Production services should install their own
//! subscriber (JSON, etc.); events still emit and are cheap no-ops without one.
//!
//! ```no_run
//! modrun::logging::init();
//! ```

use std::fmt;
use std::io::IsTerminal;

use tracing::Event;
use tracing::field::{Field, Visit};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Install a minimal tracing subscriber for modrun framework events.
///
/// Respects `RUST_LOG` when set; otherwise defaults to `modrun=info`.
///
/// This is for examples and local `main`. If a subscriber is already installed,
/// the call is a **no-op** (it does not panic). Prefer [`try_init`] when you
/// need to know whether this helper won. Production processes should set up
/// their own subscriber and skip this function.
///
/// ANSI colors are enabled only when stderr is a terminal.
pub fn init() {
    let _ = try_init();
}

/// Like [`init`], returning `true` when this process's global subscriber was
/// installed by this call.
#[must_use]
pub fn try_init() -> bool {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("modrun=info"));
    try_init_with_filter(filter)
}

/// Install a minimal tracing subscriber with an explicit filter.
///
/// No-op when a subscriber is already installed. See [`init`].
/// ANSI colors are enabled only when stderr is a terminal.
pub fn init_with_filter(filter: EnvFilter) {
    let _ = try_init_with_filter(filter);
}

/// Like [`init_with_filter`], returning `true` when this call installed the
/// global subscriber.
#[must_use]
pub fn try_init_with_filter(filter: EnvFilter) -> bool {
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stderr().is_terminal())
        .event_format(MessageOnly)
        .try_init()
        .is_ok()
}

/// Print only the preformatted fx-style `message` so structured fields stay
/// available to JSON subscribers without cluttering the local console.
#[derive(Clone, Debug, Default)]
struct MessageOnly;

struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" && self.message.is_none() {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }
}

impl<S, N> FormatEvent<S, N> for MessageOnly
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = MessageVisitor { message: None };
        event.record(&mut visitor);
        let Some(message) = visitor.message else {
            return Ok(());
        };
        let message = strip_debug_quotes(&message);
        writeln!(writer, "{message}")
    }
}

fn strip_debug_quotes(message: &str) -> &str {
    let bytes = message.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        &message[1..message.len() - 1]
    } else {
        message
    }
}
