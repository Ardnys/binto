use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The first of the listed `ArchiveKind` variants whose extension `name` ends with, checked
/// in the order given.
macro_rules! first_extension_of {
    ($name:expr, [$($kind:ident),+ $(,)?]) => {{
        let name: &str = $name;
        $(
            if name.ends_with(ArchiveKind::$kind.extension()) {
                return Some(ArchiveKind::$kind);
            }
        )+
        None
    }};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Tgz,
    TarXz,
    TarBz2,
    Zip,
}

impl ArchiveKind {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::TarGz => ".tar.gz",
            Self::Tgz => ".tgz",
            Self::TarXz => ".tar.xz",
            Self::TarBz2 => ".tar.bz2",
            Self::Zip => ".zip",
        }
    }

    /// `name` must be lowercase
    pub fn from_filename(name: &str) -> Option<Self> {
        // Checked in the order listed: a longer extension must come before any shorter one it
        // ends with (`.tar.gz` before a bare `.gz`).
        first_extension_of!(name, [TarGz, Tgz, TarXz, TarBz2, Zip])
    }
    pub fn label(self) -> &'static str {
        match self {
            ArchiveKind::TarGz | ArchiveKind::Tgz | ArchiveKind::TarXz | ArchiveKind::TarBz2 => {
                "tar"
            }
            ArchiveKind::Zip => "zip",
        }
    }
    pub fn extract(self, path: &Path, dest: &Path) -> Result<()> {
        match self {
            ArchiveKind::TarGz | ArchiveKind::Tgz => extract_tar_gz(path, dest),
            ArchiveKind::TarXz => extract_tar_xz(path, dest),
            ArchiveKind::TarBz2 => extract_tar_bz2(path, dest),
            ArchiveKind::Zip => extract_zip(path, dest),
        }
    }
}

#[tracing::instrument(skip(dest_dir), fields(archive = %archive_path.display()), err(level = "debug"))]
pub fn extract_archive(
    archive_kind: ArchiveKind,
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<()> {
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("failed to create {}", dest_dir.display()))?;

    archive_kind.extract(archive_path, dest_dir)
}

fn extract_tar_gz(path: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest)
        .with_context(|| format!("failed to unpack tar.gz to {}", dest.display()))
}

fn extract_tar_xz(path: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let xz = liblzma::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(xz);
    archive
        .unpack(dest)
        .with_context(|| format!("failed to unpack tar.xz to {}", dest.display()))
}

fn extract_tar_bz2(path: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let bz2 = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(bz2);
    archive
        .unpack(dest)
        .with_context(|| format!("failed to unpack tar.bz2 to {}", dest.display()))
}

fn extract_zip(path: &Path, dest: &Path) -> Result<()> {
    let file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("failed to read zip {}", path.display()))?;
    archive
        .extract(dest)
        .with_context(|| format!("failed to extract zip to {}", dest.display()))
}

const ELF_MAGIC: &[u8] = b"\x7fELF";

/// Locate the binary inside an extracted directory.
/// Priority:
/// 1. File named `binary_name` (exact match)
/// 2. Single ELF file with executable bit
/// 3. Multiple ELFs → return all for the caller to present a picker
#[tracing::instrument(skip(dir), fields(binary = %binary_name), err(level = "debug"))]
pub fn find_binary(dir: &Path, binary_name: &str) -> Result<BinarySearchResult> {
    let mut elf_files: Vec<PathBuf> = Vec::new();

    // Sorted, so the same archive always yields the same match and the same candidate order.
    for entry in walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e: Result<walkdir::DirEntry, _>| e.ok())
        .filter(|e: &walkdir::DirEntry| e.file_type().is_file())
    {
        let path: PathBuf = entry.path().to_path_buf();

        // Priority 1: exact name match
        if path.file_name().and_then(|n: &std::ffi::OsStr| n.to_str()) == Some(binary_name) {
            return Ok(BinarySearchResult::Found(path));
        }

        // Collect ELF candidates (executable bit may not be set in archives — check magic only)
        if is_elf(&path) {
            elf_files.push(path);
        }
    }

    match elf_files.len() {
        0 => Ok(BinarySearchResult::NotFound),
        1 => Ok(BinarySearchResult::Found(elf_files.remove(0))),
        _ => Ok(BinarySearchResult::Multiple(elf_files)),
    }
}

pub enum BinarySearchResult {
    Found(PathBuf),
    Multiple(Vec<PathBuf>),
    NotFound,
}

pub fn is_elf(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    use std::io::Read;
    f.read_exact(&mut magic)
        .map(|_| magic == *ELF_MAGIC)
        .unwrap_or(false)
}

pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal .tar.gz in memory containing one fake ELF binary.
    fn make_tar_gz_with_elf(binary_name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);

            let elf_bytes: &[u8] = b"\x7fELF fake binary content";
            let mut header = tar::Header::new_gnu();
            header.set_size(elf_bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            ar.append_data(&mut header, binary_name, elf_bytes).unwrap();
            ar.finish().unwrap();
        }
        buf
    }

    /// Build a minimal .tar.gz with an ELF that has NO executable bit set.
    fn make_tar_gz_elf_no_exec(binary_name: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);

            let elf_bytes: &[u8] = b"\x7fELF binary without exec bit";
            let mut header = tar::Header::new_gnu();
            header.set_size(elf_bytes.len() as u64);
            header.set_mode(0o644); // no exec bit
            header.set_cksum();
            ar.append_data(&mut header, binary_name, elf_bytes).unwrap();
            ar.finish().unwrap();
        }
        buf
    }

    #[test]
    fn extracts_tar_gz_and_finds_named_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("tool.tar.gz");
        let extract_dir = tmp.path().join("extract");
        let archive_kind = ArchiveKind::from_filename(archive_path.to_str().unwrap()).unwrap();

        std::fs::write(&archive_path, make_tar_gz_with_elf("mytool")).unwrap();
        extract_archive(archive_kind, &archive_path, &extract_dir).unwrap();

        let result = find_binary(&extract_dir, "mytool").unwrap();
        assert!(matches!(result, BinarySearchResult::Found(_)));
    }

    #[test]
    fn finds_elf_without_exec_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("tool.tar.gz");
        let extract_dir = tmp.path().join("extract");
        let archive_kind = ArchiveKind::from_filename(archive_path.to_str().unwrap()).unwrap();

        std::fs::write(&archive_path, make_tar_gz_elf_no_exec("mytool")).unwrap();
        extract_archive(archive_kind, &archive_path, &extract_dir).unwrap();

        // Must still find it even without executable bit
        let result = find_binary(&extract_dir, "mytool").unwrap();
        assert!(matches!(result, BinarySearchResult::Found(_)));
    }

    #[test]
    fn not_found_when_no_elf() {
        let tmp = tempfile::tempdir().unwrap();
        let archive_path = tmp.path().join("tool.tar.gz");
        let extract_dir = tmp.path().join("extract");
        let archive_kind = ArchiveKind::from_filename(archive_path.to_str().unwrap()).unwrap();

        // Write a tar.gz with a plain text file
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);
            let content = b"not an elf";
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            ar.append_data(&mut header, "readme.txt", content as &[u8])
                .unwrap();
            ar.finish().unwrap();
        }
        std::fs::write(&archive_path, buf).unwrap();
        extract_archive(archive_kind, &archive_path, &extract_dir).unwrap();

        let result = find_binary(&extract_dir, "mytool").unwrap();
        assert!(matches!(result, BinarySearchResult::NotFound));
    }
}
