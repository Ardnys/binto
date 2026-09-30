//! A cross-process advisory lock serializing binto's state/manifest read-modify-write sections.
//!
//! Without it, two concurrent `binto` processes both `load()` the old file, mutate their own
//! in-memory copy, and `save()` — the second to write clobbers the first's entry (a lost update),
//! and both racing on the shared `*.toml.tmp` temp file can corrupt it. The lock is held only for
//! the brief mutation (never across a download), so concurrent installs still run their slow work
//! in parallel and merely serialize their writes.

use std::fs::OpenOptions;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Path to the lock file (`~/.local/share/binto/.lock`). It carries no data — only its advisory
/// lock matters — and lives in the data dir so `binto clean` (which wipes the cache) leaves it be.
pub fn lock_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
        .join("binto/.lock")
}

/// RAII guard for the global binto mutation lock. The lock is released when this drops (and, as a
/// backstop, when the process exits).
pub struct LockGuard(std::fs::File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Block until this process holds the exclusive binto mutation lock.
///
/// Call this around a single state/manifest read-modify-write; never hold the returned guard
/// across slow work (network, downloads) or you'll serialize what should run in parallel. Do not
/// nest acquisitions on the same thread — a second exclusive `flock` from the same process blocks
/// on the first.
pub fn acquire() -> Result<LockGuard> {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("failed to open lock file {}", path.display()))?;
    file.lock()
        .with_context(|| format!("failed to acquire lock on {}", path.display()))?;
    Ok(LockGuard(file))
}
