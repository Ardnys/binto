//! Parse a release-asset filename into the typed facts every later stage reads.
//!
//! Parsing is host-independent on purpose: an asset is *described* (`arch: aarch64`),
//! never *judged* (`arch: foreign`). Applying the host, and deciding what that means, is
//! [`super::filter`]'s job. Nothing downstream re-inspects a filename.

use std::ops::Range;
use std::sync::LazyLock;

// -- token matching ------------------------------------------------------

/// Characters that delimit a term inside an asset name.
const SEPARATORS: &[u8] = b"-_. ";

fn is_sep(b: u8) -> bool {
    SEPARATORS.contains(&b)
}

// TODO: i don't like the bytes here
/// Where `term` occurs in `name` delimited by a separator or a string edge, if it does.
///
/// Plain `contains` is not good enough here: it fires on `arm` inside `alarm` and on
/// `mac` inside `macchanger`. Callers must still try terms **longest-first** — `_` is a
/// separator, so `x86` is a genuine token of `x86_64` and only ordering keeps a 64-bit
/// asset from being read as 32-bit.
fn token_span(name: &str, term: &str) -> Option<Range<usize>> {
    if term.is_empty() {
        return None;
    }
    let bytes = name.as_bytes();
    name.match_indices(term)
        .map(|(start, _)| start..start + term.len())
        .find(|span| {
            (span.start == 0 || is_sep(bytes[span.start - 1]))
                && (span.end == bytes.len() || is_sep(bytes[span.end]))
        })
}

/// The first entry in `table` whose term is a token of `name`, with where it matched.
///
/// `table` must be sorted longest term first; [`sorted_longest_first`] does that.
fn find_token_span<T: Copy>(name: &str, table: &[(&'static str, T)]) -> Option<(T, Range<usize>)> {
    table
        .iter()
        .find_map(|(term, value)| token_span(name, term).map(|span| (*value, span)))
}

fn sorted_longest_first<T>(mut pairs: Vec<(&'static str, T)>) -> Vec<(&'static str, T)> {
    pairs.sort_by_key(|(term, _)| std::cmp::Reverse(term.len()));
    pairs
}

/// Where the first matching suffix in `table` sits, if `name` ends with one.
fn suffix_span<T>(name: &str, table: &[(&'static str, T)]) -> Option<Range<usize>> {
    table
        .iter()
        .find(|(suffix, _)| name.ends_with(suffix))
        .map(|(suffix, _)| name.len() - suffix.len()..name.len())
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
/// Longest term first, as [`find_token_span`] requires.
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
// WARN: currently I hand sort this list
pub const LINUX_TERMS: &[&str] = &["unknown-linux", "linux"];

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

// -- asset names ---------------------------------------------------------

/// A fact read off an asset name, with every span that stating it consumed.
///
/// Both outputs come from one search, so what [`AssetName::facts`] reports and what
/// [`AssetName::stem`] removes can never disagree about which term matched.
struct Found<T> {
    fact: T,
    spans: Vec<Range<usize>>,
}

/// One asset's file name, ready to be read for facts or reduced to its stem.
///
/// Holds the name twice on purpose. Matching runs against `lower` so tables need only
/// list lowercase terms, and every span this module produces indexes `lower` — never
/// `raw`, whose bytes can shift under `to_lowercase` (`İ` is two bytes and lowercases to
/// three). `raw` is kept verbatim because GitHub download URLs are case-sensitive.
pub struct AssetName {
    raw: String,
    lower: String,
}

impl AssetName {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let lower = raw.to_lowercase();
        Self { raw, lower }
    }

    /// The name exactly as the release published it.
    #[allow(dead_code)]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Everything the matcher reads off this name.
    pub fn facts(&self) -> AssetFacts {
        AssetFacts {
            os: self.find_os().fact,
            arch: self.find_arch().fact,
            libc: self.find_libc().fact,
            kind: parse_kind(&self.lower),
        }
    }

    /// What is left of the name once every fact it states is cut out: `tool-server` from
    /// `tool-server-x86_64-unknown-linux-musl.tar.xz`. Lowercase, since that is what the
    /// spans index and what an installed filename should be anyway.
    ///
    /// `tag` is the release tag the asset shipped under. Without it the version survives
    /// into the stem (`gh_2.45.0`), because a version is only recognisable relative to the
    /// tag that produced it.
    pub fn stem(&self, tag: Option<&str>) -> String {
        let mut spans = Vec::new();
        spans.extend(self.find_arch().spans);
        spans.extend(self.find_os().spans);
        spans.extend(self.find_libc().spans);
        spans.extend(self.extension_span());
        spans.extend(tag.and_then(|tag| self.version_span(tag)));

        tidy_separators(&cut_spans(&self.lower, spans))
    }

    /// The named architecture wins the *fact* wherever the word size sits relative to it,
    /// but both spans are cut: `..._arm64_64bit.tar.gz` states one machine twice.
    fn find_arch(&self) -> Found<ArchFact> {
        let named = find_token_span(&self.lower, &ARCH_TERMS);
        let bits = find_token_span(&self.lower, BITNESS_TERMS);

        let fact = match named.as_ref().or(bits.as_ref()) {
            Some((canonical, _)) => ArchFact::Named(canonical),
            None => ArchFact::Unspecified,
        };
        let spans = named.into_iter().chain(bits).map(|(_, s)| s).collect();

        Found { fact, spans }
    }

    fn find_os(&self) -> Found<OsFact> {
        // A foreign marker wins over `linux`: a name carrying both is not something we can
        // confidently install, and rejecting is the safe reading.
        if let Some((marker, span)) = find_token_span(&self.lower, &OS_FOREIGN_SORTED) {
            return Found {
                fact: OsFact::Foreign(marker),
                spans: vec![span],
            };
        }
        if let Some(ext) = first_suffix(&self.lower, OS_FOREIGN_EXTENSIONS) {
            return Found {
                fact: OsFact::Foreign(ext),
                spans: suffix_span(&self.lower, OS_FOREIGN_EXTENSIONS)
                    .into_iter()
                    .collect(),
            };
        }
        // Hand-sorted longest-first, so `unknown-linux` is consumed before `linux`.
        match LINUX_TERMS
            .iter()
            .find_map(|term| token_span(&self.lower, term))
        {
            Some(span) => Found {
                fact: OsFact::Linux,
                spans: vec![span],
            },
            None => Found {
                fact: OsFact::Unspecified,
                spans: vec![],
            },
        }
    }

    fn find_libc(&self) -> Found<LibcFact> {
        match find_token_span(&self.lower, &LIBC_SORTED) {
            Some((fact, span)) => Found {
                fact,
                spans: vec![span],
            },
            None => Found {
                fact: LibcFact::Unspecified,
                spans: vec![],
            },
        }
    }

    /// Checked in the order [`parse_kind`] uses, so `.tar.gz` is never read as a bare `.gz`.
    fn extension_span(&self) -> Option<Range<usize>> {
        suffix_span(&self.lower, SUPPORTED_ARCHIVES)
            .or_else(|| suffix_span(&self.lower, UNSUPPORTED_ARCHIVES))
            .or_else(|| suffix_span(&self.lower, NOT_A_BINARY_EXTENSIONS))
            .or_else(|| suffix_span(&self.lower, OS_FOREIGN_EXTENSIONS))
    }

    /// Where this asset states its version, given the tag it shipped under.
    ///
    /// A release tags `v2.45.0` and names the asset `gh_2.45.0_...`, or the reverse, so
    /// both spellings are tried. The bare-substring fallback catches a version glued to
    /// its neighbours, which no token search would find.
    fn version_span(&self, tag: &str) -> Option<Range<usize>> {
        let tag = tag.trim().to_lowercase();
        let version = version_of_tag(&tag);
        let bare = version.trim_start_matches('v');
        if bare.is_empty() {
            return None;
        }
        let prefixed = format!("v{bare}");
        let candidates = [version, bare, prefixed.as_str()];

        candidates
            .iter()
            .find_map(|c| token_span(&self.lower, c))
            .or_else(|| {
                candidates
                    .iter()
                    .find_map(|c| self.lower.find(c).map(|start| start..start + c.len()))
            })
    }
}

/// The version portion of a release tag.
///
/// A repo shipping several products tags per product — `jql-v8.1.2`, `lutgen-studio-v0.4.0` —
/// and cutting the whole tag out of `jql-v8.1.2-x86_64-unknown-linux-musl.tar.gz` takes the
/// binary's name with it, leaving an empty stem. Only the part that looks like a version is
/// one: the earliest separator-delimited run beginning with an optional `v` and then a digit.
///
/// A tag naming no version at all (`nightly`) is returned whole, since the whole tag is then
/// the closest thing to a version the release has.
fn version_of_tag(tag: &str) -> &str {
    let bytes = tag.as_bytes();
    // Every start index is either 0 or preceded by an ASCII boundary byte, so it is always
    // a char boundary.
    (0..tag.len())
        .filter(|&i| i == 0 || is_tag_boundary(bytes[i - 1]))
        .find(|&i| {
            let rest = &tag[i..];
            rest.strip_prefix('v')
                .unwrap_or(rest)
                .starts_with(|c: char| c.is_ascii_digit())
        })
        .map(|i| &tag[i..])
        .unwrap_or(tag)
}

/// Where a version is allowed to begin inside a tag: after a scope or word separator.
///
/// Deliberately not [`is_sep`]. A `.` sits *inside* a version, so accepting it as a start
/// makes `cli/v2.2.1` match at the second dot and report `2.1`. A `/` is not a separator in
/// an asset name but is the usual scope marker in a monorepo tag.
fn is_tag_boundary(b: u8) -> bool {
    matches!(b, b'-' | b'_' | b'/' | b' ')
}

/// `s` with every span removed. Spans may overlap and arrive in any order — `x86_64` as an
/// architecture overlaps `64` as a word size — so they are sorted and merged as they are cut.
fn cut_spans(s: &str, mut spans: Vec<Range<usize>>) -> String {
    spans.sort_by_key(|span| span.start);

    let mut out = String::with_capacity(s.len());
    let mut cursor = 0;
    for span in spans {
        if span.start > cursor {
            out.push_str(&s[cursor..span.start]);
        }
        cursor = cursor.max(span.end);
    }
    out.push_str(&s[cursor..]);
    out
}

/// Tidy the separator runs a cut leaves behind: drop them at either edge, and collapse an
/// interior run to its first character so `tool-x86_64-linux-gnu-v3.tar.gz` reduces to
/// `tool-v3` rather than `tool---v3`.
///
/// A single interior separator is load-bearing — it is what makes `tool-cli` a different
/// binary from `tool` — so only runs are touched, and the surviving character keeps the
/// name's own style (`_` stays `_`).
fn tidy_separators(name: &str) -> String {
    let trimmed = name.trim_matches(|c: char| c.is_ascii() && is_sep(c as u8));

    let mut out = String::with_capacity(trimmed.len());
    let mut in_run = false;
    for c in trimmed.chars() {
        let sep = c.is_ascii() && is_sep(c as u8);
        if !(sep && in_run) {
            out.push(c);
        }
        in_run = sep;
    }
    out
}

fn parse_kind(name: &str) -> AssetKind {
    // Token-delimited, not `contains`: `resource-manager` and `opensource-cli` are
    // binaries, and a substring test rejects both as source archives.
    if token_span(name, "source").is_some() {
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

    fn has_token(name: &str, term: &str) -> bool {
        token_span(name, term).is_some()
    }

    fn parse(name: &str) -> AssetFacts {
        AssetName::new(name).facts()
    }

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

    fn stem_of(name: &str) -> String {
        AssetName::new(name).stem(None)
    }

    #[test]
    fn a_stem_is_the_name_with_every_stated_fact_cut_out() {
        for (name, expected) in [
            ("tool_linux_amd64.tar.gz", "tool"),
            ("tool-cli_linux_amd64.tar.gz", "tool-cli"),
            ("tool-x86_64-unknown-linux-gnu.tar.gz", "tool"),
            (
                "tool-server-x86_64-unknown-linux-musl.tar.xz",
                "tool-server",
            ),
            (
                "ascii-image-converter_Linux_arm64_64bit.tar.gz",
                "ascii-image-converter",
            ),
            ("bat-arm-unknown-linux-gnueabihf.tar.gz", "bat"),
            ("tool-linux-amd64-static", "tool"),
            // Nothing stated but the OS: the rest of the name is all stem.
            ("tool-linux.tar.gz", "tool"),
        ] {
            assert_eq!(stem_of(name), expected, "{name}");
        }
    }

    /// The stem is what tells `tool-cli` and `tool-server` apart, so separators *inside*
    /// it have to survive — only the runs a cut leaves at the edges are trimmed.
    #[test]
    fn separators_inside_a_stem_survive() {
        assert_eq!(stem_of("tool-cli_linux_amd64.tar.gz"), "tool-cli");
        assert_eq!(
            stem_of("tool-server-x86_64-unknown-linux-musl.tar.xz"),
            "tool-server"
        );
        // A fact leading the name leaves the separator run in front, not just behind.
        assert_eq!(stem_of("x86_64-linux-tool.tar.gz"), "tool");
    }

    /// Without the tag a version is unrecognisable, so it stays; with the tag it goes,
    /// whichever side of the `v` each spelling falls on.
    #[test]
    fn a_version_is_only_removable_against_its_release_tag() {
        let gh = AssetName::new("gh_2.45.0_linux_386.tar.gz");
        assert_eq!(gh.stem(None), "gh_2.45.0");
        assert_eq!(gh.stem(Some("v2.45.0")), "gh");
        assert_eq!(gh.stem(Some("2.45.0")), "gh");

        let arduino = AssetName::new("arduino-cli_1.5.1_Linux_64bit.tar.gz");
        assert_eq!(arduino.stem(None), "arduino-cli_1.5.1");
        assert_eq!(arduino.stem(Some("1.5.1")), "arduino-cli");

        // The asset carries the `v`, the tag does not.
        let tool = AssetName::new("tool-v1.2.3-linux-amd64.tar.gz");
        assert_eq!(tool.stem(Some("1.2.3")), "tool");
        assert_eq!(tool.stem(Some("v1.2.3")), "tool");

        // A tag that appears nowhere in the name leaves the stem alone.
        assert_eq!(stem_of("tool_linux_amd64.tar.gz"), "tool");
        assert_eq!(
            AssetName::new("tool_linux_amd64.tar.gz").stem(Some("v9.9.9")),
            "tool"
        );
    }

    /// The removers this replaced searched with a plain `contains`, so `arm` inside
    /// `alarm` cut a hole in the stem.
    #[test]
    fn a_stem_cut_respects_token_boundaries() {
        assert_eq!(stem_of("alarm-clock-linux-amd64.tar.gz"), "alarm-clock");
        assert_eq!(stem_of("macchanger-linux-amd64.tar.gz"), "macchanger");
        assert_eq!(stem_of("gnuplot-linux-amd64.tar.gz"), "gnuplot");
    }

    /// A substring test read `resource` and `opensource` as source archives and rejected
    /// the binary outright.
    #[test]
    fn source_is_matched_as_a_token_not_a_substring() {
        for name in [
            "resource-manager_linux_amd64.tar.gz",
            "opensource-cli_linux_amd64.tar.gz",
            "datasource_linux_amd64.tar.gz",
        ] {
            assert_eq!(
                parse(name).kind,
                AssetKind::Installable(Format::Tar),
                "{name}"
            );
        }
        // The genuine article, however it is spelled.
        for name in ["Source code (zip)", "tool_source_code.tar.gz"] {
            assert_eq!(parse(name).kind, AssetKind::SourceArchive, "{name}");
        }
    }

    /// A cut in the middle of a name leaves the separators from both sides touching, and
    /// a stem is a filename — `tool---v3` is not one.
    #[test]
    fn interior_separator_runs_collapse_to_one() {
        assert_eq!(stem_of("tool-x86_64-linux-gnu-v3.tar.gz"), "tool-v3");
        assert_eq!(stem_of("tool_linux_amd64_extras.tar.gz"), "tool_extras");
        // A version's own dots are single separators and survive untouched.
        assert_eq!(stem_of("gh_2.45.0_linux_386.tar.gz"), "gh_2.45.0");
    }

    /// `arm64` and `64bit` overlap in `..._arm64_64bit...`; cutting both must not
    /// double-cut the shared bytes or panic on the reversed span.
    #[test]
    fn overlapping_facts_are_cut_once() {
        assert_eq!(
            stem_of("ascii-image-converter_Linux_arm64_64bit.tar.gz"),
            "ascii-image-converter"
        );
        assert_eq!(stem_of("tool_linux_64bit_armv7.tar.gz"), "tool");
        assert_eq!(stem_of("tool_Linux_64-bit_ppc64le.tar.gz"), "tool");
    }

    /// Spans index the lowercased name, and `raw` is never sliced — a name whose case
    /// folding changes its byte length would otherwise slice mid-character and panic.
    #[test]
    fn a_stem_is_lowercase_and_leaves_the_raw_name_alone() {
        let name = AssetName::new("Tool-CLI_Linux_AMD64.tar.gz");
        assert_eq!(name.stem(None), "tool-cli");
        assert_eq!(name.raw(), "Tool-CLI_Linux_AMD64.tar.gz");

        // `İ` is two bytes and lowercases to three.
        let turkish = AssetName::new("İtool_linux_amd64.tar.gz");
        assert_eq!(turkish.raw(), "İtool_linux_amd64.tar.gz");
        assert!(turkish.stem(None).ends_with("tool"));
    }

    /// A repo that tags per product puts the product's name in the tag, so cutting the tag
    /// whole took the binary's name with it. Every one of these came back empty from a real
    /// harness run before the version was separated from the tag.
    #[test]
    fn a_name_prefixed_tag_gives_up_only_its_version() {
        for (name, tag, expected) in [
            (
                "jql-v8.1.2-x86_64-unknown-linux-musl.tar.gz",
                "jql-v8.1.2",
                "jql",
            ),
            (
                "iwe-v0.8.0-x86_64-unknown-linux-gnu.tar.gz",
                "iwe-v0.8.0",
                "iwe",
            ),
            (
                "lutgen-studio-v0.4.0-x86_64-unknown-linux-gnu",
                "lutgen-studio-v0.4.0",
                "lutgen-studio",
            ),
            // Scoped with `/` rather than `-`.
            ("plandex_2.2.1_linux_amd64.tar.gz", "cli/v2.2.1", "plandex"),
            ("dnote_0.16.0_linux_amd64.tar.gz", "cli-v0.16.0", "dnote"),
        ] {
            assert_eq!(AssetName::new(name).stem(Some(tag)), expected, "{tag}");
        }
    }

    #[test]
    fn a_tag_is_split_at_the_version_it_names() {
        assert_eq!(version_of_tag("jql-v8.1.2"), "v8.1.2");
        assert_eq!(version_of_tag("lutgen-studio-v0.4.0"), "v0.4.0");
        assert_eq!(version_of_tag("ipinfo-3.3.2"), "3.3.2");
        // A monorepo scopes its tag with `/`, which no asset name uses as a separator.
        assert_eq!(version_of_tag("cli/v2.2.1"), "v2.2.1");
        // A `.` is interior to a version, never the start of one: accepting it here read
        // `cli/v2.2.1` as `2.1` and left `plandex_2` behind.
        assert_eq!(version_of_tag("2.2.1"), "2.2.1");
        // Plain tags are already nothing but a version.
        assert_eq!(version_of_tag("v2.45.0"), "v2.45.0");
        assert_eq!(version_of_tag("1.5.1"), "1.5.1");
        // A prerelease suffix belongs to the version, not to the name.
        assert_eq!(version_of_tag("app-v1.2.3-beta.1"), "v1.2.3-beta.1");
        // Nothing version-shaped: the whole tag is the best a release has to offer.
        assert_eq!(version_of_tag("nightly"), "nightly");
    }

    /// Splitting the tag must not make version removal greedier — a tag absent from the
    /// name still cuts nothing.
    #[test]
    fn a_tag_that_is_not_in_the_name_cuts_nothing() {
        let tool = AssetName::new("tool_linux_amd64.tar.gz");
        assert_eq!(tool.stem(Some("v9.9.9")), "tool");
        assert_eq!(tool.stem(Some("other-v9.9.9")), "tool");
        assert_eq!(tool.stem(Some("nightly")), "tool");
    }
}
