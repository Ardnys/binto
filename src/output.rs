//! User-facing message helpers. These are thin façades over `tracing`: each emits an event on
//! the `ghr::ui` target so it renders with ghr's styled prefix on the terminal (see
//! `ui_format`) *and* lands in the file log. Machine-readable output (`--json`, the `list`
//! table) is written directly to stdout by its callers, not through here.

use crate::error::GhrError;

/// Emit an error event, with a context-specific hint for the well-known `GhrError` variants.
pub fn print_error(err: &anyhow::Error) {
    if let Some(ghr_err) = err.downcast_ref::<GhrError>() {
        tracing::error!(target: "ghr::ui", "{ghr_err}");
        let hint = match ghr_err {
            GhrError::RateLimitExceeded { .. } => Some(
                "Set GITHUB_TOKEN or add github_token to ~/.config/ghr/config.toml to increase the rate limit.",
            ),
            GhrError::NoCompatibleAssets { .. } => {
                Some("Try `ghr install` with `--prerelease` or check the release page manually.")
            }
            GhrError::ChecksumMismatch { .. } => {
                Some("Downloaded file has been removed. The release asset may be corrupted.")
            }
            _ => None,
        };
        if let Some(hint) = hint {
            tracing::info!(target: "ghr::ui", "  hint: {hint}");
        }
    } else {
        tracing::error!(target: "ghr::ui", "{err:#}");
    }
}

pub fn print_success(msg: &str) {
    tracing::info!(target: "ghr::ui", kind = "success", "{msg}");
}

pub fn print_warning(msg: &str) {
    tracing::warn!(target: "ghr::ui", "{msg}");
}

pub fn print_info(msg: &str) {
    tracing::info!(target: "ghr::ui", "{msg}");
}

/// Print a pre-styled line verbatim (no `info:`/`✓` prefix) — for the indented, colored status
/// lines in `update`/`check`/the stale banner. Still captured in the file log.
pub fn print_status(msg: &str) {
    tracing::info!(target: "ghr::ui", kind = "status", "{msg}");
}
