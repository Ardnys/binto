//! Parse a release-asset filename into the typed facts every later stage reads.
//!
//! Parsing is host-independent on purpose: an asset is *described* (`arch: aarch64`),
//! never *judged* (`arch: foreign`). Applying the host, and deciding what that means, is
//! [`super::filter`]'s job. Nothing downstream re-inspects a filename.

use std::sync::LazyLock;

// -- token matching ------------------------------------------------------

/// Characters that delimit a term inside an asset name.
const SEPARATORS: &[u8] = b"-_. ";

fn is_sep(b: u8) -> bool {
    SEPARATORS.contains(&b)
}

/// True when `term` occurs in `name` delimited by a separator or a string edge.
///
/// Plain `contains` is not good enough here: it fires on `arm` inside `alarm` and on
/// `mac` inside `macchanger`. Callers must still try terms **longest-first** — `_` is a
/// separator, so `x86` is a genuine token of `x86_64` and only ordering keeps a 64-bit
/// asset from being read as 32-bit.
fn has_token(name: &str, term: &str) -> bool {
    let bytes = name.as_bytes();
    name.match_indices(term).any(|(start, _)| {
        let end = start + term.len();
        (start == 0 || is_sep(bytes[start - 1])) && (end == bytes.len() || is_sep(bytes[end]))
    })
}

/// The value of the first entry in `table` whose term is a token of `name`.
///
/// `table` must be sorted longest term first; [`sorted_longest_first`] does that.
fn find_token<T: Copy>(name: &str, table: &[(&'static str, T)]) -> Option<T> {
    table
        .iter()
        .find(|(term, _)| has_token(name, term))
        .map(|(_, value)| *value)
}

fn sorted_longest_first<T>(mut pairs: Vec<(&'static str, T)>) -> Vec<(&'static str, T)> {
    pairs.sort_by_key(|(term, _)| std::cmp::Reverse(term.len()));
    pairs
}

fn first_suffix<T: Copy>(name: &str, table: &[(&'static str, T)]) -> Option<T> {
    table
        .iter()
        .find(|(suffix, _)| name.ends_with(suffix))
        .map(|(_, value)| *value)
}

// -- architecture --------------------------------------------------------

// TODO: some repos have both x86_64 and amd64 in assets?
// TODO: there's also "baseline" builds. Default to "baseline" if stuck between these choices. Additional info:
// TODO: there's "default" or "dev" builds as well
// openscience-linux-x64-baseline.tar.gz VS openscience-linux-x64.tar.gz
// Apparently it's about microarchitectures in modern CPUs. Too specific to work on it for now.
//
/// Every architecture binto can recognise, and the spellings releases use for it.
///
/// Architectures we never run on are listed too, not out of completeness but because an
/// unlisted one parses as "no architecture stated" and stays a candidate — an `arm5`
/// build would otherwise be offered on an aarch64 host.
///
/// Word-size markers are deliberately absent; they live in [`BITNESS_TERMS`].
const ARCH_SYNONYMS: &[(&str, &[&str])] = &[
    ("x86_64", &["x86_64", "amd64", "x64", "amd_64"]),
    ("aarch64", &["aarch64", "arm64"]),
    ("armv7", &["armv7", "armv7l", "armhf", "arm"]),
    ("armv6", &["armv6", "armv6l", "arm6"]),
    ("armv5", &["armv5", "armv5l", "arm5"]),
    ("i686", &["i686", "i386", "x86", "386"]),
    ("riscv64", &["riscv64", "riscv64gc"]),
    ("ppc64le", &["ppc64le", "powerpc64le"]),
    ("ppc64", &["ppc64", "powerpc64"]),
    ("s390x", &["s390x"]),
    ("loongarch64", &["loongarch64", "loong64"]),
    ("mips64le", &["mips64le", "mips64el"]),
    ("mips64", &["mips64"]),
    ("mipsle", &["mipsle", "mipsel"]),
    ("mips", &["mips"]),
];

/// Word size, which is not an architecture: `64bit` says how wide the machine is, not
/// which machine it is. Kept out of [`ARCH_SYNONYMS`] because it maps to no single
/// canonical name — every 64-bit architecture answers to it.
///
/// Consulted only once [`ARCH_TERMS`] has found nothing, which is the whole rule:
/// `..._arm64_64bit.tar.gz` states both facts and the machine is the more specific one, so
/// it wins by never being asked to compete. Ordering inside the name is irrelevant, unlike
/// a single flattened table where `64bit` and `arm64` are the same length and declaration
/// order silently decides.
///
/// A lone `64bit` is read as `x86_64`: a publisher labelling by word size alone is
/// shipping for the desktop, and an aarch64 build that cared would have said so.
///
/// Longest term first, as [`find_token`] requires.
const BITNESS_TERMS: &[(&str, &str)] = &[
    ("64-bit", "x86_64"),
    ("32-bit", "32bit"),
    ("64bit", "x86_64"),
    ("32bit", "32bit"),
    ("32", "32bit"),
];

/// Every synonym flattened to `(term, canonical)`, longest term first so `x86_64` is
/// consumed before `x86` and `arm64` before `arm`.
static ARCH_TERMS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    sorted_longest_first(
        ARCH_SYNONYMS
            .iter()
            .flat_map(|(canonical, synonyms)| synonyms.iter().map(move |s| (*s, *canonical)))
            .collect(),
    )
});

/// The architecture an asset names, canonicalised. `Unspecified` is common and harmless:
/// plenty of releases ship a single binary with no arch marker at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchFact {
    Named(&'static str),
    Unspecified,
}

impl ArchFact {
    pub fn label(self) -> &'static str {
        match self {
            ArchFact::Named(canonical) => canonical,
            ArchFact::Unspecified => "unspecified",
        }
    }
}

/// Canonicalise a raw machine name (`uname -m`, or `--arch`). Falls back to `x86_64`.
pub fn canonical_arch(raw: &str) -> &'static str {
    let raw = raw.trim().to_lowercase();
    ARCH_SYNONYMS
        .iter()
        .find(|(_, synonyms)| synonyms.contains(&raw.as_str()))
        .map(|(canonical, _)| *canonical)
        // `uname` never says `64bit`, but a hand-written `--arch` might.
        .or_else(|| {
            BITNESS_TERMS
                .iter()
                .find(|(term, _)| *term == raw)
                .map(|(_, canonical)| *canonical)
        })
        .unwrap_or("x86_64")
}

pub fn detect_arch() -> String {
    std::process::Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_lowercase())
        .unwrap_or_else(|| std::env::consts::ARCH.to_lowercase())
}

// -- operating system ----------------------------------------------------

// TODO: there's winx, dragonfly
const OS_FOREIGN_TERMS: &[&str] = &[
    "windows", "darwin", "macos", "osx", "win32", "win64", "freebsd", "netbsd", "mac", "openbsd",
    "solaris", "android",
];

// TODO: there are sometimes install.sh scripts in the releases. Should we run them?
// TODO: or we can just get the shell script and make it executable alongside binto binaries. maybe that works too in some cases
const OS_FOREIGN_EXTENSIONS: &[(&str, &str)] = &[
    (".exe", "exe"),
    (".msi", "msi"),
    (".dmg", "dmg"),
    (".pkg", "pkg"),
    (".apk", "apk"),
];

static OS_FOREIGN_SORTED: LazyLock<Vec<(&'static str, &'static str)>> =
    LazyLock::new(|| sorted_longest_first(OS_FOREIGN_TERMS.iter().map(|t| (*t, *t)).collect()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsFact {
    Linux,
    /// A non-Linux OS, carrying the marker that gave it away.
    Foreign(&'static str),
    Unspecified,
}

impl OsFact {
    pub fn label(self) -> &'static str {
        match self {
            OsFact::Linux => "linux",
            OsFact::Foreign(marker) => marker,
            OsFact::Unspecified => "unspecified",
        }
    }
}

// -- libc ----------------------------------------------------------------

/// `gnu` deliberately does not match inside `gnueabihf` under token rules, so every
/// embedded-ABI spelling is listed explicitly.
const LIBC_TERMS: &[(&str, LibcFact)] = &[
    ("gnu", LibcFact::Gnu),
    ("glibc", LibcFact::Gnu),
    ("gnueabi", LibcFact::Gnu),
    ("gnueabihf", LibcFact::Gnu),
    ("musl", LibcFact::Musl),
    ("musleabi", LibcFact::Musl),
    ("musleabihf", LibcFact::Musl),
    // A statically linked build has no libc dependency, which is what musl buys you.
    // TODO: "standalone" or "stand_alone" could also mean musl
    ("static", LibcFact::Musl),
];

static LIBC_SORTED: LazyLock<Vec<(&'static str, LibcFact)>> =
    LazyLock::new(|| sorted_longest_first(LIBC_TERMS.to_vec()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibcFact {
    Gnu,
    Musl,
    Unspecified,
}

impl LibcFact {
    pub fn label(self) -> &'static str {
        match self {
            LibcFact::Gnu => "gnu",
            LibcFact::Musl => "musl",
            LibcFact::Unspecified => "unspecified",
        }
    }
}

// -- packaging -----------------------------------------------------------

// TODO what shall we do with completions and man pages
// TODO: sometimes the exact same package has different archive extensions. In that scenario we can pick anything
// TODO: there's .gz, .tar.zst
/// Archive shapes `installer::extract::extract_archive` can actually open.
///
/// This table and that function must agree: an entry here that the extractor does not
/// handle means the matcher can confidently pick an asset the install then fails on.
const SUPPORTED_ARCHIVES: &[(&str, Format)] = &[
    (".tar.gz", Format::Tar),
    (".tgz", Format::Tar),
    (".tar.xz", Format::Tar),
    (".tar.bz2", Format::Tar),
    (".zip", Format::Zip),
    (".appimage", Format::AppImage),
];

/// Extensions that are plainly not a runnable binary. Releases ship editor extensions,
/// language-ecosystem packages, and documentation alongside the real artifact; without
/// this list they fall through to "raw binary" and compete for the install.
const NOT_A_BINARY_EXTENSIONS: &[(&str, &str)] = &[
    (".vsix", "vsix"),
    (".jar", "jar"),
    (".war", "war"),
    (".nupkg", "nupkg"),
    (".gem", "gem"),
    (".whl", "whl"),
    (".crate", "crate"),
    (".snap", "snap"),
    (".flatpak", "flatpak"),
    (".wasm", "wasm"),
    (".json", "json"),
    (".jsonl", "jsonl"),
    (".yaml", "yaml"),
    (".yml", "yml"),
    (".toml", "toml"),
    (".xml", "xml"),
    (".txt", "txt"),
    (".md", "md"),
    (".pdf", "pdf"),
    (".png", "png"),
    (".svg", "svg"),
    (".so", "so"),
    (".sqlite", "sqlite"),
    (".pdb", "pdb"),           // debug-symbol database
    (".d", "d"),               // debug symbols
    (".rb", "ruby"),           // some peeps put ruby scripts in releases
    (".asar", "asar"),         // electron app archive
    (".blockmap", "blockmap"), // electron related something
];

// TODO: when an archive is implemented, remove them from this list
/// Compressed shapes the extractor does *not* understand. Without this list they fall
/// through to "raw binary", get copied verbatim, and are installed as an executable that
/// is really a compressed blob.
const UNSUPPORTED_ARCHIVES: &[(&str, &str)] = &[
    (".tar.zst", "tar.zst"),
    (".tzst", "tzst"),
    (".tar.lz4", "tar.lz4"),
    (".tar.lzma", "tar.lzma"),
    (".7z", "7z"),
    (".gz", "gz"),
    (".xz", "xz"),
    (".bz2", "bz2"),
    (".zst", "zst"),
    (".lz4", "lz4"),
];

/// How an installable asset is packaged. Ranked by [`super::rank`]; every variant here is
/// something the installer can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Tar,
    Zip,
    /// No recognised extension — assumed to be the binary itself.
    Raw,
    AppImage,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Tar => "tar",
            Format::Zip => "zip",
            Format::Raw => "raw",
            Format::AppImage => "appimage",
        }
    }
}

// -- sidecars ------------------------------------------------------------

const CHECKSUM_EXTENSIONS: &[&str] = &[
    ".sha256",
    ".sha512",
    ".sha1",
    ".md5",
    ".sig",
    ".asc",
    ".minisig",
    ".b64",
    ".sigstore.json",
    ".sum",
    ".pub",
];

/// Digest names that appear as a whole-file manifest: `SHA256SUMS`, `md5sums.txt`.
const CHECKSUM_ALGOS: &[&str] = &[
    "md5", "sha1", "sha224", "sha256", "sha384", "sha512", "b2", "blake2", "blake2b", "blake3",
];

/// True for the digest manifest a release ships next to its binaries.
///
/// Matched by shape rather than enumerated: releases spell it `SHA256SUMS`,
/// `sha512sum.txt`, `MD5SUMS`, and a dozen other ways, and an incomplete list leaves the
/// manifest looking like an extensionless binary.
fn is_checksum_manifest(name: &str) -> bool {
    if name.contains("checksum") {
        return true;
    }
    // Only an exact whole-name match counts, so a released `b2sum` *binary* would be
    // named `b2sum-linux-amd64` and stay a candidate.
    let stem = name.strip_suffix(".txt").unwrap_or(name);
    let stem = stem.strip_suffix('s').unwrap_or(stem);
    stem.strip_suffix("sum")
        .is_some_and(|algo| CHECKSUM_ALGOS.contains(&algo))
}

// TODO: also parse sbom.json files — worth investigating which repos ship them.
const SBOM_SUFFIX: &str = "sbom.json";

/// What an asset *is*, before any question of whether we want it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// A candidate for installation, packaged the given way.
    Installable(Format),
    /// A checksum or signature file, carrying the marker that identified it.
    Sidecar(&'static str),
    Sbom,
    SourceArchive,
    /// A distro package (`.deb`/`.rpm`) — binto installs into user-land, not a package db.
    Package(&'static str),
    /// A compressed archive the installer cannot open, carrying its extension.
    UnsupportedArchive(&'static str),
    /// An editor extension, ecosystem package, or document — shipped in the same release
    /// as the binary, but not a thing to install on `$PATH`.
    NotABinary(&'static str),
}

/// Everything the matcher knows about an asset, derived from its name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetFacts {
    pub os: OsFact,
    pub arch: ArchFact,
    pub libc: LibcFact,
    pub kind: AssetKind,
}

/// Parse an asset name. Case-insensitive; the name is lowercased once here.
pub fn parse(name: &str) -> AssetFacts {
    let name = name.to_lowercase();

    AssetFacts {
        os: parse_os(&name),
        arch: parse_arch(&name),
        libc: find_token(&name, &LIBC_SORTED).unwrap_or(LibcFact::Unspecified),
        kind: parse_kind(&name),
    }
}

/// A named architecture if the asset states one, else whatever its word size implies.
fn parse_arch(name: &str) -> ArchFact {
    find_token(name, &ARCH_TERMS)
        .or_else(|| find_token(name, BITNESS_TERMS))
        .map(ArchFact::Named)
        .unwrap_or(ArchFact::Unspecified)
}

fn parse_os(name: &str) -> OsFact {
    // A foreign marker wins over `linux`: a name carrying both is not something we can
    // confidently install, and rejecting is the safe reading.
    if let Some(marker) = find_token(name, &OS_FOREIGN_SORTED) {
        return OsFact::Foreign(marker);
    }
    if let Some(ext) = first_suffix(name, OS_FOREIGN_EXTENSIONS) {
        return OsFact::Foreign(ext);
    }
    if has_token(name, "linux") {
        OsFact::Linux
    } else {
        OsFact::Unspecified
    }
}

fn parse_kind(name: &str) -> AssetKind {
    if name.contains("source code") || name.contains("source_code") || name.contains("source") {
        return AssetKind::SourceArchive;
    }
    if let Some(ext) = CHECKSUM_EXTENSIONS.iter().find(|ext| name.ends_with(*ext)) {
        return AssetKind::Sidecar(ext);
    }
    if is_checksum_manifest(name) {
        return AssetKind::Sidecar("checksum");
    }
    if name.ends_with(SBOM_SUFFIX) {
        return AssetKind::Sbom;
    }
    if name.ends_with(".deb") {
        return AssetKind::Package("deb");
    }
    if name.ends_with(".rpm") {
        return AssetKind::Package("rpm");
    }
    // Supported archives are checked before unsupported ones so `.tar.gz` is never read
    // as a bare `.gz`.
    if let Some(format) = first_suffix(name, SUPPORTED_ARCHIVES) {
        return AssetKind::Installable(format);
    }
    if let Some(ext) = first_suffix(name, UNSUPPORTED_ARCHIVES) {
        return AssetKind::UnsupportedArchive(ext);
    }
    if let Some(ext) = first_suffix(name, NOT_A_BINARY_EXTENSIONS) {
        return AssetKind::NotABinary(ext);
    }
    AssetKind::Installable(Format::Raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_need_boundaries() {
        assert!(has_token("tool-arm-linux", "arm"));
        assert!(has_token("arm-linux", "arm"));
        assert!(has_token("tool-linux-arm", "arm"));
        // The bug boundary matching exists to fix.
        assert!(!has_token("alarm-clock-linux-amd64", "arm"));
        assert!(!has_token("macchanger-linux-amd64", "mac"));
        assert!(!has_token("gnuplot-linux", "gnu"));
    }

    #[test]
    fn underscore_is_a_separator_so_arch_terms_must_be_tried_longest_first() {
        // `x86` really is a token of `x86_64`; only ordering saves us.
        assert!(has_token("tool-x86_64-linux", "x86"));
        assert_eq!(parse("tool-x86_64-linux").arch, ArchFact::Named("x86_64"));
        assert_eq!(parse("tool-arm64-linux").arch, ArchFact::Named("aarch64"));
        assert_eq!(parse("tool-x86-linux").arch, ArchFact::Named("i686"));
    }

    #[test]
    fn arch_synonyms_canonicalise() {
        for (name, expected) in [
            ("tool_linux_amd64.tar.gz", "x86_64"),
            ("tool-x86_64-unknown-linux-gnu.tar.gz", "x86_64"),
            ("tool-aarch64-unknown-linux-gnu.tar.gz", "aarch64"),
            ("gh_2.45.0_linux_386.tar.gz", "i686"),
            ("tool-riscv64-linux.tar.gz", "riscv64"),
            ("tool-powerpc64le-linux.tar.gz", "ppc64le"),
            ("arduino-cli_1.5.1_Linux_32bit.tar.gz", "32bit"),
            ("arduino-cli_1.5.1_Linux_64bit.tar.gz", "x86_64"),
            ("ascii-image-converter_Linux_arm64_64bit.tar.gz", "aarch64"),
            // The named architecture wins wherever the word size sits relative to it.
            ("tool_linux_64bit_armv7.tar.gz", "armv7"),
            ("tool_Linux_64-bit_ppc64le.tar.gz", "ppc64le"),
        ] {
            assert_eq!(parse(name).arch, ArchFact::Named(expected), "{name}");
        }
        assert_eq!(parse("tool-linux.tar.gz").arch, ArchFact::Unspecified);
    }

    #[test]
    fn embedded_abi_spellings_resolve_to_a_libc() {
        assert_eq!(
            parse("bat-arm-unknown-linux-gnueabihf.tar.gz").libc,
            LibcFact::Gnu
        );
        assert_eq!(
            parse("tool-armv7-linux-musleabi.tar.gz").libc,
            LibcFact::Musl
        );
        assert_eq!(parse("tool-x86_64-linux-gnu.tar.gz").libc, LibcFact::Gnu);
        assert_eq!(parse("tool-x86_64-linux-musl.tar.gz").libc, LibcFact::Musl);
        assert_eq!(parse("tool-linux-amd64-static").libc, LibcFact::Musl);
        assert_eq!(parse("tool-linux-amd64.tar.gz").libc, LibcFact::Unspecified);
    }

    #[test]
    fn foreign_os_is_detected_by_term_or_extension() {
        for (name, marker) in [
            ("tool_windows_amd64.zip", "windows"),
            ("tool-x86_64-apple-darwin.tar.gz", "darwin"),
            ("tool_macOS_amd64.zip", "macos"),
            ("tool_win64.exe", "win64"),
            ("tool-installer.msi", "msi"),
        ] {
            assert_eq!(parse(name).os, OsFact::Foreign(marker), "{name}");
        }
        assert_eq!(parse("tool_linux_amd64.tar.gz").os, OsFact::Linux);
        assert_eq!(parse("tool_amd64.tar.gz").os, OsFact::Unspecified);
    }

    #[test]
    fn sidecars_and_packages_are_not_installable() {
        for (name, expected) in [
            (
                "tool_linux_amd64.tar.gz.sha256",
                AssetKind::Sidecar(".sha256"),
            ),
            ("tool_linux_amd64.tar.gz.sig", AssetKind::Sidecar(".sig")),
            ("checksums.txt", AssetKind::Sidecar("checksum")),
            ("SHA256SUMS", AssetKind::Sidecar("checksum")),
            ("tool_checksums.txt", AssetKind::Sidecar("checksum")),
            ("tool.sbom.json", AssetKind::Sbom),
            ("tool_amd64.deb", AssetKind::Package("deb")),
            ("tool_x86_64.rpm", AssetKind::Package("rpm")),
            ("Source code (zip)", AssetKind::SourceArchive),
        ] {
            assert_eq!(parse(name).kind, expected, "{name}");
        }
    }

    /// Enumerating manifest names left `MD5SUMS` and `SHA1SUMS` looking like
    /// extensionless binaries, so a release shipping only those offered them as install
    /// candidates.
    #[test]
    fn checksum_manifests_are_matched_by_shape_not_by_list() {
        for name in [
            "sha256sums",
            "sha256sum",
            "sha256sums.txt",
            "sha512sums",
            "md5sums",
            "sha1sums",
            "blake3sums",
            "b2sums",
            "checksums",
        ] {
            assert_eq!(parse(name).kind, AssetKind::Sidecar("checksum"), "{name}");
        }
        // Caught by the digest extension rather than the manifest shape — a different
        // marker in the trace, the same rejection.
        assert_eq!(
            parse("checksums.sha256").kind,
            AssetKind::Sidecar(".sha256")
        );
        // A released binary that merely ends in `sum` is not a manifest.
        assert_eq!(
            parse("b2sum-linux-amd64").kind,
            AssetKind::Installable(Format::Raw)
        );
        assert_eq!(parse("tool-sums").kind, AssetKind::Installable(Format::Raw));
    }

    /// An unlisted architecture parses as "unstated" and stays a candidate, so the
    /// unrunnable ones have to be named.
    #[test]
    fn uncommon_architectures_are_recognised_rather_than_read_as_unstated() {
        for (name, expected) in [
            ("dstask-linux-arm5", "armv5"),
            ("tool-linux-armv6l", "armv6"),
            ("autorestic_1.8.3_linux_mips64le.bz2", "mips64le"),
            ("autorestic_1.8.3_linux_mipsle.bz2", "mipsle"),
            ("autorestic_1.8.3_linux_mips.bz2", "mips"),
            ("tool-linux-ppc64", "ppc64"),
            ("arduino-cli_1.5.1_Linux_32bit.tar.gz", "32bit"),
        ] {
            assert_eq!(parse(name).arch, ArchFact::Named(expected), "{name}");
        }
        // The longer spellings still win their prefixes.
        assert_eq!(parse("tool-linux-arm64").arch, ArchFact::Named("aarch64"));
        assert_eq!(parse("tool-linux-ppc64le").arch, ArchFact::Named("ppc64le"));
        assert_eq!(parse("tool-linux-mips64").arch, ArchFact::Named("mips64"));
    }

    #[test]
    fn formats_match_what_the_extractor_supports() {
        for (name, format) in [
            ("tool.tar.gz", Format::Tar),
            ("tool.tgz", Format::Tar),
            ("tool.tar.xz", Format::Tar),
            ("tool.tar.bz2", Format::Tar),
            ("tool.zip", Format::Zip),
            ("Tool-x86_64.AppImage", Format::AppImage),
            ("tool_linux_amd64", Format::Raw),
        ] {
            assert_eq!(parse(name).kind, AssetKind::Installable(format), "{name}");
        }
    }

    /// `extract_archive` copies anything it does not recognise verbatim, so ranking these
    /// as raw binaries installs a compressed blob as an executable.
    #[test]
    fn archives_the_extractor_cannot_open_are_their_own_kind() {
        for (name, ext) in [
            ("tool-x86_64-linux.tar.zst", "tar.zst"),
            ("tool-linux-amd64.gz", "gz"),
            ("tool-linux-amd64.7z", "7z"),
        ] {
            assert_eq!(
                parse(name).kind,
                AssetKind::UnsupportedArchive(ext),
                "{name}"
            );
        }
        // A supported tarball must never be read as a bare `.gz`.
        assert_eq!(
            parse("tool-x86_64-linux.tar.gz").kind,
            AssetKind::Installable(Format::Tar)
        );
    }

    /// Releases ship editor extensions and docs next to the binary; ranked as raw
    /// binaries they compete for — and can win — the install.
    #[test]
    fn artifacts_that_are_not_binaries_are_their_own_kind() {
        for (name, ext) in [
            ("tombi-vscode-1.2.0-linux-x64.vsix", "vsix"),
            ("tool-1.0.jar", "jar"),
            ("release-notes.md", "md"),
            ("tool-manifest.yaml", "yaml"),
            ("tool-linux-amd64.wasm", "wasm"),
        ] {
            assert_eq!(parse(name).kind, AssetKind::NotABinary(ext), "{name}");
        }
    }

    #[test]
    fn canonical_arch_normalises_uname_output() {
        assert_eq!(canonical_arch("x86_64"), "x86_64");
        assert_eq!(canonical_arch("AMD64"), "x86_64");
        assert_eq!(canonical_arch("aarch64"), "aarch64");
        assert_eq!(canonical_arch(" arm64 "), "aarch64");
        // Unknown machine names fall back rather than failing the install outright.
        assert_eq!(canonical_arch("sparc"), "x86_64");
    }
}
