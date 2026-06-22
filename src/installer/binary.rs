use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)
        .with_context(|| format!("failed to chmod +x {}", path.display()))
}

/// Atomically install a binary from `src` to `dest_dir/binary_name`.
/// Uses write-to-temp + rename to avoid partial overwrites.
#[tracing::instrument(skip(src), fields(dest_dir = %dest_dir.display(), binary = %binary_name), err(level = "debug"))]
pub fn atomic_install(src: &Path, dest_dir: &Path, binary_name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dest_dir).map_err(|_| crate::error::GhrError::InstallDirMissing {
        path: dest_dir.to_path_buf(),
    })?;

    make_executable(src)?;

    let dest = dest_dir.join(binary_name);
    let tmp = dest_dir.join(format!(".{binary_name}.tmp"));

    std::fs::copy(src, &tmp).with_context(|| format!("failed to copy to {}", tmp.display()))?;

    make_executable(&tmp)?;

    std::fs::rename(&tmp, &dest)
        .with_context(|| format!("failed to rename to {}", dest.display()))?;

    Ok(dest)
}
