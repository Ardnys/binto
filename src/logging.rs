//! Tracing setup: a styled terminal layer (preserves ghr's UX, → stderr) plus an always-on
//! rotating file log under the data dir, with download progress driven by `tracing-indicatif`
//! spans so log lines never clobber the bars.
//!
//! Verbosity: `-v`/`-vv`/`-q` control the *terminal* level; `GHR_LOG` controls the *file* level
//! (`GHR_LOG=off` disables the file). The file defaults to `debug` so a failed install is always
//! diagnosable after the fact.

use std::io::{self, Write};
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_indicatif::IndicatifLayer;
use tracing_indicatif::filter::IndicatifFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

use crate::ui_format::UiFormat;

/// Wraps a writer to strip ANSI escape codes. The terminal-facing status lines pre-render their
/// color with `console::style`, so that color is baked into the event message; the file log must
/// not contain it. `with_ansi(false)` only stops the *formatter* from adding color — it doesn't
/// touch color already inside a field — so we strip it here on the way to the file.
struct StripAnsi<W>(W);

impl<W: Write> Write for StripAnsi<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The fmt layer writes one fully-formatted event per call, so a buffer never splits an
        // escape sequence.
        let text = String::from_utf8_lossy(buf);
        self.0
            .write_all(console::strip_ansi_codes(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

struct StripAnsiMakeWriter<M>(M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for StripAnsiMakeWriter<M> {
    type Writer = StripAnsi<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        StripAnsi(self.0.make_writer())
    }
}

/// The ghr log directory (`~/.local/share/ghr/logs`). Lives in the data dir (not the cache) so
/// `ghr clean` doesn't wipe it.
pub fn log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("ghr/logs")
}

/// Map the `-v`/`-q` flags to an `EnvFilter` for the terminal layer (scoped to the `ghr` crate
/// so dependency spans don't leak in).
fn terminal_filter(verbose: u8, quiet: bool) -> EnvFilter {
    let level = if quiet && verbose == 0 {
        "warn"
    } else {
        match verbose {
            0 => "info",
            1 => "debug",
            _ => "trace",
        }
    };
    EnvFilter::new(format!("ghr={level}"))
}

/// The file layer's filter: `GHR_LOG` if set, else `debug` (→ `trace` at `-vv`). Returns `None`
/// when `GHR_LOG=off`, which disables the file log entirely.
fn file_filter(verbose: u8) -> Option<EnvFilter> {
    match std::env::var("GHR_LOG") {
        Ok(v) if v.eq_ignore_ascii_case("off") => None,
        Ok(v) if !v.trim().is_empty() => Some(EnvFilter::new(v)),
        _ => {
            let level = if verbose >= 2 { "trace" } else { "debug" };
            Some(EnvFilter::new(format!("ghr={level}")))
        }
    }
}

/// Build the rotating file layer (daily, keep 7) plus its non-blocking worker guard. Returns
/// `None` if `GHR_LOG=off` or the appender can't be created — logging must never take down a
/// real command, so a file failure degrades to terminal-only.
fn file_layer<S>(verbose: u8) -> Option<(Box<dyn Layer<S> + Send + Sync>, WorkerGuard)>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let filter = file_filter(verbose)?;

    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("warning: could not create log dir {}: {e}", dir.display());
        return None;
    }

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("ghr")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&dir)
        .map_err(|e| eprintln!("warning: could not open log file in {}: {e}", dir.display()))
        .ok()?;

    let (writer, guard) = tracing_appender::non_blocking(appender);
    let layer = fmt::layer()
        .with_writer(StripAnsiMakeWriter(writer))
        .with_ansi(false)
        // The status lines pre-render color with `console::style`, so ANSI ends up *inside* the
        // message. By default tracing-subscriber escapes that into literal `\x1b[..` text (an
        // anti-injection guard), which `StripAnsi` then can't remove. Disable the escaping so
        // the bytes stay real ANSI — `StripAnsi` strips them on the way to the file.
        .with_ansi_sanitization(false)
        .with_target(true)
        .with_filter(filter)
        .boxed();

    Some((layer, guard))
}

/// Initialize the global subscriber. The returned [`WorkerGuard`] (when present) must be held
/// for the lifetime of the process — dropping it early truncates the file log — so `main` keeps
/// it in a binding that lives until exit.
pub fn init(verbose: u8, quiet: bool) -> Option<WorkerGuard> {
    let indicatif_layer = IndicatifLayer::new();
    let stderr_writer = indicatif_layer.get_stderr_writer();

    let terminal_layer = fmt::layer()
        .event_format(UiFormat)
        .with_writer(stderr_writer)
        .with_filter(terminal_filter(verbose, quiet));

    let (file_layer, guard) = match file_layer(verbose) {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(file_layer)
        .with(terminal_layer)
        // Only spans carrying an `indicatif.pb_show` field render a bar (download spans);
        // everything else is just structured context.
        .with(indicatif_layer.with_filter(IndicatifFilter::new(false)))
        .init();

    guard
}
