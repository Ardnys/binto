//! Hard filters: the constraints that are not negotiable.
//!
//! An asset either *can* be installed on this host or it cannot. Nothing here expresses a
//! preference — a foreign architecture is not a weak signal to be outweighed, it is
//! disqualifying. Preferences live in [`super::rank`].

use tracing::debug;

use crate::github::types::Asset;
use crate::matcher::facts::{self, ArchFact, AssetKind, Format, LibcFact, OsFact};

/// Why an asset cannot be installed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Built for another operating system, per the carried marker.
    ForeignOs(&'static str),
    /// Built for another architecture, per the carried canonical name.
    ForeignArch(&'static str),
    /// A checksum or signature file, not an artifact.
    Sidecar(&'static str),
    Sbom,
    SourceArchive,
    /// A distro package; binto installs into user-land, not a package database.
    Package(&'static str),
    /// A compressed archive `installer::extract` cannot open — selecting it would
    /// download successfully and then install a compressed blob as the binary.
    UnsupportedArchive(&'static str),
    /// An editor extension, ecosystem package, or document shipped in the same release.
    NotABinary(&'static str),
}

impl RejectReason {
    pub fn label(self) -> &'static str {
        match self {
            RejectReason::ForeignOs(_) => "foreign_os",
            RejectReason::ForeignArch(_) => "foreign_arch",
            RejectReason::Sidecar(_) => "sidecar",
            RejectReason::Sbom => "sbom",
            RejectReason::SourceArchive => "source_archive",
            RejectReason::Package(_) => "package",
            RejectReason::UnsupportedArchive(_) => "unsupported_archive",
            RejectReason::NotABinary(_) => "not_a_binary",
        }
    }

    /// The marker that gave the asset away, for the trace.
    pub fn marker(self) -> &'static str {
        match self {
            RejectReason::ForeignOs(m)
            | RejectReason::ForeignArch(m)
            | RejectReason::Sidecar(m)
            | RejectReason::Package(m)
            | RejectReason::UnsupportedArchive(m)
            | RejectReason::NotABinary(m) => m,
            RejectReason::Sbom => "sbom.json",
            RejectReason::SourceArchive => "source code",
        }
    }
}

/// An asset that cleared every hard filter, carrying the facts ranking needs.
///
/// The fields are flat and total: by construction this asset is installable, so there is
/// no `Option` to unwrap and no inapplicable variant for [`super::rank`] to consider.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub asset: Asset,
    pub os: OsFact,
    pub arch: ArchFact,
    pub libc: LibcFact,
    pub format: Format,
}

/// Split `assets` into what can be installed on `host_arch` and what cannot.
///
/// `host_arch` is a raw machine name (`uname -m` or `--arch`); it is canonicalised here.
/// Assets naming no architecture are kept — plenty of releases ship one unlabelled binary
/// — and are demoted by tier instead.
pub fn apply_hard_filters(
    assets: Vec<Asset>,
    host_arch: &str,
) -> (Vec<Candidate>, Vec<(Asset, RejectReason)>) {
    let host = facts::canonical_arch(host_arch);
    let mut kept = Vec::new();
    let mut rejected = Vec::new();

    for asset in assets {
        let f = facts::parse(&asset.name);

        // The packaging decides whether the asset is an artifact at all; only then does
        // the host get a say. `Ok` carries the format forward so the candidate below
        // needs no unwrap.
        let verdict: Result<Format, RejectReason> = match f.kind {
            AssetKind::Sidecar(marker) => Err(RejectReason::Sidecar(marker)),
            AssetKind::Sbom => Err(RejectReason::Sbom),
            AssetKind::SourceArchive => Err(RejectReason::SourceArchive),
            AssetKind::Package(ext) => Err(RejectReason::Package(ext)),
            AssetKind::UnsupportedArchive(ext) => Err(RejectReason::UnsupportedArchive(ext)),
            AssetKind::NotABinary(ext) => Err(RejectReason::NotABinary(ext)),
            AssetKind::Installable(format) => match (f.os, f.arch) {
                (OsFact::Foreign(marker), _) => Err(RejectReason::ForeignOs(marker)),
                (_, ArchFact::Named(arch)) if arch != host => Err(RejectReason::ForeignArch(arch)),
                _ => Ok(format),
            },
        };

        match verdict {
            Ok(format) => kept.push(Candidate {
                asset,
                os: f.os,
                arch: f.arch,
                libc: f.libc,
                format,
            }),
            Err(reason) => {
                debug!(
                    asset = %asset.name,
                    reason = reason.label(),
                    marker = reason.marker(),
                    "asset rejected"
                );
                rejected.push((asset, reason));
            }
        }
    }

    (kept, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 1024,
            content_type: "application/octet-stream".to_string(),
        }
    }

    fn kept_names(assets: Vec<Asset>, host_arch: &str) -> Vec<String> {
        apply_hard_filters(assets, host_arch)
            .0
            .into_iter()
            .map(|c| c.asset.name)
            .collect()
    }

    #[test]
    fn removes_checksum_files() {
        let names = kept_names(
            vec![
                asset("tool_linux_amd64.tar.gz"),
                asset("tool_linux_amd64.tar.gz.sha256"),
                asset("checksums.txt"),
                asset("tool_checksums.txt"),
                asset("SHA256SUMS"),
            ],
            "x86_64",
        );
        assert_eq!(names, vec!["tool_linux_amd64.tar.gz"]);
    }

    #[test]
    fn removes_windows_darwin_assets() {
        let names = kept_names(
            vec![
                asset("tool_linux_amd64.tar.gz"),
                asset("tool_windows_amd64.zip"),
                asset("tool_darwin_arm64.tar.gz"),
                asset("tool_macos_arm64.tar.gz"),
                asset("tool_win64.exe"),
            ],
            "x86_64",
        );
        assert_eq!(names, vec!["tool_linux_amd64.tar.gz"]);
    }

    /// Architecture is a hard constraint, not a heavy penalty to be outscored.
    #[test]
    fn removes_foreign_architectures() {
        let names = kept_names(
            vec![
                asset("tool-x86_64-unknown-linux-gnu.tar.gz"),
                asset("tool-aarch64-unknown-linux-gnu.tar.gz"),
                asset("tool-riscv64-unknown-linux-gnu.tar.gz"),
                asset("tool-s390x-unknown-linux-gnu.tar.gz"),
                asset("tool-ppc64le-unknown-linux-gnu.tar.gz"),
                asset("tool-loongarch64-unknown-linux-gnu.tar.gz"),
                asset("tool-armv7l-unknown-linux-gnueabihf.tar.gz"),
                asset("tool-i686-unknown-linux-gnu.tar.gz"),
            ],
            "x86_64",
        );
        assert_eq!(names, vec!["tool-x86_64-unknown-linux-gnu.tar.gz"]);
    }

    #[test]
    fn keeps_assets_that_name_no_architecture() {
        let names = kept_names(vec![asset("tool-linux.tar.gz"), asset("tool")], "x86_64");
        assert_eq!(names, vec!["tool-linux.tar.gz", "tool"]);
    }

    #[test]
    fn removes_packages_and_unopenable_archives() {
        let (kept, rejected) = apply_hard_filters(
            vec![
                asset("tool_amd64.deb"),
                asset("tool_x86_64.rpm"),
                asset("tool-x86_64-linux.tar.zst"),
                asset("tool-x86_64-linux.gz"),
                asset("tool-x86_64-linux.tar.gz"),
            ],
            "x86_64",
        );
        assert_eq!(
            kept.into_iter().map(|c| c.asset.name).collect::<Vec<_>>(),
            vec!["tool-x86_64-linux.tar.gz"]
        );
        assert_eq!(
            rejected.iter().map(|(_, r)| r.label()).collect::<Vec<_>>(),
            vec![
                "package",
                "package",
                "unsupported_archive",
                "unsupported_archive"
            ]
        );
    }

    #[test]
    fn aarch64_host_keeps_arm64_and_drops_amd64() {
        let names = kept_names(
            vec![
                asset("tool-linux-amd64.tar.gz"),
                asset("tool-linux-arm64.tar.gz"),
            ],
            "aarch64",
        );
        assert_eq!(names, vec!["tool-linux-arm64.tar.gz"]);
    }

    #[test]
    fn candidates_carry_the_facts_ranking_needs() {
        let (kept, _) = apply_hard_filters(
            vec![asset("ripgrep-14.1.0-x86_64-unknown-linux-musl.tar.gz")],
            "x86_64",
        );
        let c = &kept[0];
        assert_eq!(c.os, OsFact::Linux);
        assert_eq!(c.arch, ArchFact::Named("x86_64"));
        assert_eq!(c.libc, LibcFact::Musl);
        assert_eq!(c.format, Format::Tar);
    }
}
