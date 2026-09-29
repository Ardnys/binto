use binto_contract::RunRecord;
use color_eyre::eyre::{Context, ContextCompat};

use crate::app::App;
use crate::model::RepoView;

use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

pub mod app;
pub mod event;
pub mod export;
pub mod model;
pub mod ui;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    // Everything that can fail or print happens before the terminal is taken over: output
    // written after `ratatui::init` lands on the alternate screen, and an error there leaves
    // the terminal in raw mode.
    let dataset = parse_cli_args().wrap_err("Incorrect CLI arguments")?;
    let repos: Vec<RepoView> = parse_runner_results(&dataset)?
        .into_iter()
        .map(RepoView::from_record)
        .collect();

    let terminal = ratatui::init();
    let result = App::new(&dataset, repos).run(terminal);
    ratatui::restore();
    if let Ok(Some(message)) = &result {
        println!("{message}");
    }
    result.map(|_| ())
}

fn parse_cli_args() -> Option<PathBuf> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        println!("Usage: insite results.jsonl");
        println!("  Pass in the output of runner.");
        return None;
    }
    Some(PathBuf::from(&args[1]))
}

fn parse_runner_results(result_path: &Path) -> color_eyre::Result<Vec<RunRecord>> {
    let result_file = File::open(result_path).wrap_err("Failed to open results file")?;
    let mut results = Vec::new();
    for line in BufReader::new(result_file).lines() {
        let line = line.wrap_err("failed to read result line")?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str(&line).wrap_err("Failed to parse result line")?;
        results.push(record);
    }
    Ok(results)
}
