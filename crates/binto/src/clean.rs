use anyhow::{Context, Result};

use crate::installer::download::cache_dir;
use crate::output::{print_info, print_success};

/// `binto clean`: delete the download cache at `~/.cache/binto`. Installs already clean their own
/// temp files, but interrupted/failed runs can leave partial downloads and `*-extract` dirs
/// behind — this is the manual sweep. The cache is fully regenerable, so there's no prompt.
pub fn cmd_clean() -> Result<()> {
    let dir = cache_dir();

    if !dir.exists() {
        print_info("Cache is already empty.");
        return Ok(());
    }

    let freed = dir_size(&dir);

    std::fs::remove_dir_all(&dir)
        .with_context(|| format!("failed to remove cache dir {}", dir.display()))?;

    print_success(&format!("Cleared cache ({}).", human_size(freed)));
    Ok(())
}

/// Total size in bytes of all files under `dir`. Best-effort: unreadable entries are skipped.
fn dir_size(dir: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Format a byte count as a short human-readable string (e.g. `12.3 MiB`).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn formats_bytes_without_decimals() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1023), "1023 B");
    }

    #[test]
    fn formats_larger_units_with_one_decimal() {
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(1024 * 1024), "1.0 MiB");
        assert_eq!(human_size(5 * 1024 * 1024 + 512 * 1024), "5.5 MiB");
        assert_eq!(human_size(1024u64.pow(3)), "1.0 GiB");
    }
}
