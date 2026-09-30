//! User-facing message helpers. These are thin façades over `tracing`: each emits an event on
//! the `binto::ui` target so it renders with binto's styled prefix on the terminal (see
//! `ui_format`) *and* lands in the file log. Machine-readable output (`--json`, the `list`
//! table) is written directly to stdout by its callers, not through here.

use crate::error::BintoError;

/// Emit an error event, with a context-specific hint for the well-known `BintoError` variants.
pub fn print_error(err: &anyhow::Error) {
    if let Some(binto_err) = err.downcast_ref::<BintoError>() {
        tracing::error!(target: "binto::ui", "{binto_err}");
        let hint = match binto_err {
            BintoError::RateLimitExceeded { .. } => Some(
                "Set GITHUB_TOKEN or add github_token to ~/.config/binto/config.toml to increase the rate limit.",
            ),
            BintoError::NoCompatibleAssets { .. } => {
                Some("Try `binto install` with `--prerelease` or check the release page manually.")
            }
            BintoError::ChecksumMismatch { .. } => {
                Some("Downloaded file has been removed. The release asset may be corrupted.")
            }
            _ => None,
        };
        if let Some(hint) = hint {
            tracing::info!(target: "binto::ui", "  hint: {hint}");
        }
    } else {
        tracing::error!(target: "binto::ui", "{err:#}");
    }
}

pub fn print_success(msg: &str) {
    tracing::info!(target: "binto::ui", kind = "success", "{msg}");
}

pub fn print_warning(msg: &str) {
    tracing::warn!(target: "binto::ui", "{msg}");
}

pub fn print_info(msg: &str) {
    tracing::info!(target: "binto::ui", "{msg}");
}

/// Print a pre-styled line verbatim (no `info:`/`✓` prefix) — for the indented, colored status
/// lines in `update`/`check`/the stale banner. Still captured in the file log.
pub fn print_status(msg: &str) {
    tracing::info!(target: "binto::ui", kind = "status", "{msg}");
}
