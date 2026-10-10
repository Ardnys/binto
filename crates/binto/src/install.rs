use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{self, Config};
use crate::installer::plan::{ReleaseSource, Target};
use crate::installer::run::{Ctx, Installed, run};
use crate::manifest::Manifest;
use crate::output::print_warning;
use crate::reporter::{self, DownloadReporter, report};
use crate::state::State;

/// `binto install <owner/repo> [-t <tag>]`. Pins to `tag` when given (and records the pin in
/// the manifest), otherwise shows the interactive release picker. Given a tag, an already
/// tracked tool is re-pinned in place.
pub async fn cmd_install(
    repo: &str,
    tag: Option<String>,
    alias: Option<String>,
    to: Option<PathBuf>,
    include_prerelease: bool,
    assume_yes: bool,
    config: &Config,
) -> Result<()> {
    // A `--to` override (with `~` expanded) wins over the configured install_dir. The chosen
    // dir is stored in the tool's install_path, so future `binto update`s reinstall there too.
    let install_dir = match &to {
        Some(p) => config::expand_tilde(p),
        None => config.install_dir.clone(),
    };

    let include_prerelease = include_prerelease || config.include_prereleases;
    let source = match tag.clone() {
        Some(t) => ReleaseSource::Tag(t),
        None if assume_yes => ReleaseSource::Latest { include_prerelease },
        None => ReleaseSource::Pick { include_prerelease },
    };
    let target = Target::for_install(
        &State::load()?,
        repo,
        source,
        alias.clone(),
        install_dir.clone(),
        &config.install_dir,
    )?;

    let reporter = DownloadReporter::default();
    let ctx = Ctx::new(config, &reporter, reporter::prompter(assume_yes))?;
    for done in report(run(vec![target], &ctx).await?)? {
        record_in_manifest(repo, &done, tag.as_deref(), alias.as_deref())?;
        warn_if_not_on_path(done.result.installed_path.parent().unwrap_or(&install_dir));
    }
    Ok(())
}

/// Keep the declarative manifest in sync, so `binto sync` can replay this elsewhere. A fresh
/// install records the tag (`None` clears a pin) and alias; a re-pin moves only the tag and
/// keeps the alias the row already has. Format-preserving: comments elsewhere are kept.
fn record_in_manifest(
    repo: &str,
    done: &Installed,
    tag: Option<&str>,
    alias: Option<&str>,
) -> Result<()> {
    if let Some(previous) = &done.replaced {
        // By the stem it was tracked under, which names its manifest row even if this
        // release spells its asset differently; on a plain one-binary row the manifest maps
        // it back to the row itself.
        Manifest::set_tag_and_save(repo, previous.binary(), tag)
    } else {
        Manifest::record_and_save(repo, done.result.variant.as_deref(), tag, alias)
    }
}

// BUG: it looks weird with "." as a path
fn warn_if_not_on_path(install_dir: &Path) {
    let Ok(path_var) = std::env::var("PATH") else {
        return;
    };
    if !path_var.split(':').any(|p| Path::new(p) == install_dir) {
        print_warning(&format!(
            "{} is not on your PATH. Add it: export PATH=\"{}:$PATH\"",
            install_dir.display(),
            install_dir.display()
        ));
    }
}
