//! The terminal side of [`installer::reporter`](crate::installer::reporter): download bars
//! drawn on the shared [`output::progress`] and prompts through dialoguer.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use crate::installer::reporter::{Prompter, Reporter};
use crate::installer::run::{Installed, Outcome};
use crate::output::{self, print_info, print_status, print_success, print_warning};

/// Draws one byte-progress bar per running download, largest on top, in the style of uv's
/// `ProgressReporter`. Bars live on [`output::progress`], so log lines print above them.
#[derive(Default)]
pub struct DownloadReporter {
    state: Mutex<BarState>,
}

#[derive(Default)]
struct BarState {
    bars: HashMap<usize, ProgressBar>,
    /// `(size, id)` of the running bars in display order, largest first.
    order: Vec<(u64, usize)>,
    next_id: usize,
    /// The longest name shown so far; every bar pads its label to it so the bars line up.
    max_len: usize,
}

impl BarState {
    fn style(&self) -> ProgressStyle {
        ProgressStyle::with_template(&format!(
            "{{msg:{}.bold}}  {{bar:40.green/black.dim}}  {{binary_bytes:>9}} / {{binary_total_bytes:<9}}  {{bytes_per_sec:>11}}",
            self.max_len
        ))
        .expect("progress template is valid")
        .progress_chars("━──")
    }

    /// Widen every bar's label when a longer name arrives.
    fn widen_to(&mut self, len: usize) {
        if len <= self.max_len {
            return;
        }
        self.max_len = len;
        let style = self.style();
        for bar in self.bars.values() {
            bar.set_style(style.clone());
        }
    }
}

impl Reporter for DownloadReporter {
    fn on_download_start(&self, name: &str, size: Option<u64>) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.widen_to(name.len());

        let bar = match size {
            Some(len) => ProgressBar::new(len),
            None => ProgressBar::no_length(),
        };
        // Unknown sizes sort last.
        let size = size.unwrap_or(0);
        let position = state.order.partition_point(|&(s, _)| s >= size);
        let bar = output::progress().insert(position, bar);
        bar.set_style(state.style());
        bar.set_message(name.to_string());

        state.next_id += 1;
        let id = state.next_id;
        state.order.insert(position, (size, id));
        state.bars.insert(id, bar);
        id
    }

    fn on_download_progress(&self, id: usize, bytes: u64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A response can still yield bytes after its download was reported complete.
        if let Some(bar) = state.bars.get(&id) {
            bar.inc(bytes);
        }
    }

    fn on_download_complete(&self, id: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.order.retain(|&(_, i)| i != id);
        if let Some(bar) = state.bars.remove(&id) {
            bar.finish_and_clear();
            output::progress().remove(&bar);
        }
    }
}

/// Asks the person at the terminal.
pub struct TermPrompter;

impl Prompter for TermPrompter {
    fn interactive(&self) -> bool {
        true
    }

    fn select(&self, prompt: &str, items: &[String]) -> Result<usize> {
        Ok(output::progress().suspend(|| {
            dialoguer::Select::new()
                .with_prompt(prompt)
                .items(items)
                .default(0)
                .interact()
        })?)
    }

    fn fuzzy_select(&self, prompt: &str, items: &[String]) -> Result<usize> {
        Ok(output::progress().suspend(|| {
            dialoguer::FuzzySelect::new()
                .with_prompt(prompt)
                .items(items)
                .default(0)
                .interact()
        })?)
    }

    fn confirm(&self, prompt: &str, default: bool) -> Result<bool> {
        Ok(output::progress().suspend(|| {
            dialoguer::Confirm::new()
                .with_prompt(prompt)
                .default(default)
                .interact()
        })?)
    }
}

/// Answers for `--yes`: the first (top-ranked) item of every pick, and yes to every confirm.
pub struct AssumeYes;

impl Prompter for AssumeYes {
    fn interactive(&self) -> bool {
        false
    }

    fn select(&self, _prompt: &str, _items: &[String]) -> Result<usize> {
        Ok(0)
    }

    fn fuzzy_select(&self, _prompt: &str, _items: &[String]) -> Result<usize> {
        Ok(0)
    }

    fn confirm(&self, _prompt: &str, _default: bool) -> Result<bool> {
        Ok(true)
    }
}

/// The prompter for a command's `--yes` flag.
pub fn prompter(assume_yes: bool) -> &'static dyn Prompter {
    if assume_yes {
        &AssumeYes
    } else {
        &TermPrompter
    }
}

/// Print what happened to each tool, then hand back the installed ones — or an error if any
/// tool failed. A lone tool's failure is returned as it is, so `main` prints it with its hint.
pub fn report(outcomes: Vec<Outcome>) -> Result<Vec<Installed>> {
    let lone = outcomes.len() == 1;
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Installed(done) => {
                print_installed(&done);
                installed.push(*done);
            }
            Outcome::Failed { name, error } if lone => failed.push((name, error)),
            Outcome::Failed { name, error } => {
                print_warning(&format!("{name}: {error:#}"));
                failed.push((name, error));
            }
            other => print_settled(&other),
        }
    }
    match failed.len() {
        0 => Ok(installed),
        1 if lone => Err(failed.swap_remove(0).1),
        n => {
            let names: Vec<&str> = failed.iter().map(|(name, _)| name.as_str()).collect();
            anyhow::bail!("{n} tool(s) failed: {}", names.join(", "))
        }
    }
}

fn print_installed(done: &Installed) {
    let entry = &done.result.tool_entry;
    match &done.replaced {
        Some(from) => print_success(&format!(
            "{}: {} → {}",
            entry.binary_name, from.installed_tag, entry.installed_tag
        )),
        None => print_success(&format!(
            "Installed {} {} → {}",
            entry.binary_name,
            entry.installed_tag,
            done.result.installed_path.display()
        )),
    }
}

/// A tool left as it was.
fn print_settled(outcome: &Outcome) {
    match outcome {
        Outcome::UpToDate { name, tag } => print_up_to_date(name, tag),
        Outcome::Pinned {
            name,
            installed,
            pin,
        } => print_status(&format!(
            "  {} {installed} (pinned {pin})",
            console::style(name).blue().bold()
        )),
        Outcome::Declined { name } => print_info(&format!("{name}: installation cancelled.")),
        Outcome::Installed(_) | Outcome::Failed { .. } => {}
    }
}

pub fn print_up_to_date(name: &str, tag: &str) {
    print_status(&format!(
        "  {} {tag} (up to date)",
        console::style(name).green().bold(),
    ));
}
