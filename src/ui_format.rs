//! Custom terminal event formatter for the user-facing tracing layer.
//!
//! Reproduces ghr's original styled output (`✓` / `info:` / `warning:` / `error:`) from the
//! event's level plus an optional `kind` field, so routing the `output::print_*` helpers
//! through `tracing` keeps the exact terminal UX. Only the terminal layer uses this; the file
//! layer uses the default timestamped formatter.

use std::fmt;

use console::style;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Collects the `message` and `kind` fields off an event.
#[derive(Default)]
struct UiVisitor {
    message: String,
    kind: Option<String>,
}

impl Visit for UiVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "kind" => self.kind = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

/// Terminal formatter: a colored prefix derived from level (+ the `kind` field) followed by the
/// message. No timestamp or target — those live in the file log.
pub struct UiFormat;

impl<S, N> FormatEvent<S, N> for UiFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = UiVisitor::default();
        event.record(&mut visitor);

        let level = *event.metadata().level();

        // `kind = "status"` is pre-styled, prefix-free output (the indented/colored lines from
        // `update`/`check`): print the message verbatim so its look is unchanged.
        if level == Level::INFO && visitor.kind.as_deref() == Some("status") {
            return writeln!(writer, "{}", visitor.message);
        }

        let prefix = match (level, visitor.kind.as_deref()) {
            (Level::ERROR, _) => style("error:").red().bold().to_string(),
            (Level::WARN, _) => style("warning:").yellow().bold().to_string(),
            (Level::INFO, Some("success")) => style("✓").green().bold().to_string(),
            (Level::INFO, _) => style("info:").cyan().bold().to_string(),
            (Level::DEBUG, _) => style("debug:").dim().to_string(),
            (Level::TRACE, _) => style("trace:").dim().to_string(),
        };

        writeln!(writer, "{prefix} {}", visitor.message)
    }
}
