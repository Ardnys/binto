use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::event::{AppEvent, Event, EventHandler};
use crate::export;
use crate::model::{Outcome, RepoView};
use crate::ui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{DefaultTerminal, widgets::TableState};

/// Which outcomes the repo list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Only(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Repos,
    Detail,
}

/// What happens once an export succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterExport {
    Stay,
    /// The dialog was opened by quitting with marks that were never exported.
    Quit,
}

#[derive(Debug)]
pub struct ExportDialog {
    pub input: String,
    pub after: AfterExport,
    /// Set by Enter on a path that already exists; a second Enter overwrites it.
    pub confirm_overwrite: bool,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub events: EventHandler,
    /// The results file, for the header.
    pub source: String,
    pub repos: Vec<RepoView>,
    /// Indices into `repos` that pass the current filter, multi-stem toggle, and search.
    pub visible: Vec<usize>,
    /// Selection within `visible`.
    pub table: TableState,
    pub filter: Filter,
    pub multi_stem_only: bool,
    pub query: String,
    pub searching: bool,
    pub focus: Focus,
    /// Clamped to the content height when rendered.
    pub detail_scroll: u16,
    /// Per [`Outcome::index`].
    pub counts: [usize; 4],
    /// Indices into `repos` marked for export. Survives filtering and searching.
    pub marked: BTreeSet<usize>,
    /// Marks changed since the last successful export.
    pub unsaved_marks: bool,
    pub export: Option<ExportDialog>,
    /// A transient footer message, and when it was set.
    pub status: Option<(String, Instant)>,
    /// Printed once the terminal is restored, so an export made on the way out is confirmed.
    pub exit_message: Option<String>,
    default_export: String,
}

impl App {
    pub fn new(source: &Path, repos: Vec<RepoView>) -> Self {
        let mut counts = [0; 4];
        for repo in &repos {
            counts[repo.outcome.index()] += 1;
        }
        let stem = source
            .file_stem()
            .map_or("results".into(), |s| s.to_string_lossy());

        let mut app = Self {
            running: true,
            events: EventHandler::new(),
            source: source.display().to_string(),
            repos,
            visible: Vec::new(),
            table: TableState::default(),
            filter: Filter::All,
            multi_stem_only: false,
            query: String::new(),
            searching: false,
            focus: Focus::Repos,
            detail_scroll: 0,
            counts,
            marked: BTreeSet::new(),
            unsaved_marks: false,
            export: None,
            status: None,
            exit_message: None,
            default_export: format!("marked-{stem}.jsonl"),
        };
        app.refilter();
        app
    }

    /// Run the application's main loop. Returns a message to print after the terminal is
    /// restored, if there is one.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<Option<String>> {
        while self.running {
            terminal.draw(|frame| ui::render(frame, &mut self))?;
            self.handle_events()?;
        }
        Ok(self.exit_message)
    }

    pub fn handle_events(&mut self) -> color_eyre::Result<()> {
        match self.events.next()? {
            Event::Tick => {}
            Event::Crossterm(crossterm::event::Event::Key(key))
                if key.kind == crossterm::event::KeyEventKind::Press =>
            {
                self.handle_key_event(key);
            }
            Event::Crossterm(_) => {}
            Event::App(event) => self.apply(event),
        }
        Ok(())
    }

    /// Translate a key into an [`AppEvent`]. Nothing here touches state directly.
    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // The one way out that never asks, whatever is open.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.events.send(AppEvent::ForceQuit);
        }

        if self.export.is_some() {
            let event = match key.code {
                KeyCode::Enter => AppEvent::ExportConfirm,
                KeyCode::Esc => AppEvent::ExportCancel,
                KeyCode::Backspace => AppEvent::ExportPop,
                KeyCode::Char(c) => AppEvent::ExportPush(c),
                _ => return,
            };
            return self.events.send(event);
        }

        if self.searching {
            let event = match key.code {
                KeyCode::Enter => AppEvent::SearchEnd { keep: true },
                KeyCode::Esc => AppEvent::SearchEnd { keep: false },
                KeyCode::Backspace => AppEvent::SearchPop,
                KeyCode::Char(c) => AppEvent::SearchPush(c),
                _ => return,
            };
            return self.events.send(event);
        }

        let event = match key.code {
            KeyCode::Char('q') => AppEvent::Quit,
            // Esc backs out of an applied search before it quits.
            KeyCode::Esc if !self.query.is_empty() => AppEvent::SearchEnd { keep: false },
            KeyCode::Esc => AppEvent::Quit,

            KeyCode::Char('j') | KeyCode::Down => AppEvent::Move(1),
            KeyCode::Char('k') | KeyCode::Up => AppEvent::Move(-1),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppEvent::Move(10)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppEvent::Move(-10)
            }
            KeyCode::PageDown => AppEvent::Move(10),
            KeyCode::PageUp => AppEvent::Move(-10),
            KeyCode::Char('g') | KeyCode::Home => AppEvent::Top,
            KeyCode::Char('G') | KeyCode::End => AppEvent::Bottom,

            KeyCode::Tab => AppEvent::ToggleFocus,
            KeyCode::Char('h') | KeyCode::Left => AppEvent::Focus(Focus::Repos),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => AppEvent::Focus(Focus::Detail),

            KeyCode::Char('0') => AppEvent::SetFilter(Filter::All),
            KeyCode::Char('1') => AppEvent::SetFilter(Filter::Only(Outcome::AutoSelected)),
            KeyCode::Char('2') => AppEvent::SetFilter(Filter::Only(Outcome::NeedsInteraction)),
            KeyCode::Char('3') => AppEvent::SetFilter(Filter::Only(Outcome::NoMatch)),
            KeyCode::Char('4') => AppEvent::SetFilter(Filter::Only(Outcome::Error)),
            KeyCode::Char('m') => AppEvent::ToggleMultiStem,
            KeyCode::Char('/') => AppEvent::SearchStart,

            KeyCode::Char(' ') => AppEvent::ToggleMark,
            KeyCode::Char('e') => AppEvent::ExportOpen {
                after: AfterExport::Stay,
            },
            _ => return,
        };
        self.events.send(event);
    }

    /// The only place state changes. Every key press arrives here as an [`AppEvent`].
    pub fn apply(&mut self, event: AppEvent) {
        match event {
            AppEvent::Quit => self.request_quit(),
            AppEvent::ForceQuit => self.running = false,
            AppEvent::Move(delta) => self.move_by(delta),
            AppEvent::Top => match self.focus {
                Focus::Repos => self.select(0),
                Focus::Detail => self.detail_scroll = 0,
            },
            AppEvent::Bottom => match self.focus {
                Focus::Repos => self.select(self.visible.len().saturating_sub(1)),
                Focus::Detail => self.detail_scroll = u16::MAX,
            },
            AppEvent::Focus(focus) => self.focus = focus,
            AppEvent::ToggleFocus => {
                self.focus = match self.focus {
                    Focus::Repos => Focus::Detail,
                    Focus::Detail => Focus::Repos,
                }
            }
            AppEvent::SetFilter(filter) => {
                self.filter = filter;
                self.refilter();
            }
            AppEvent::ToggleMultiStem => {
                self.multi_stem_only = !self.multi_stem_only;
                self.refilter();
            }
            AppEvent::SearchStart => {
                self.searching = true;
                self.focus = Focus::Repos;
            }
            AppEvent::SearchPush(c) => {
                self.query.push(c);
                self.refilter();
            }
            AppEvent::SearchPop => {
                self.query.pop();
                self.refilter();
            }
            AppEvent::SearchEnd { keep } => {
                self.searching = false;
                if !keep {
                    self.query.clear();
                    self.refilter();
                }
            }
            AppEvent::ToggleMark => self.toggle_mark(),
            AppEvent::ExportOpen { after } => self.open_export(after),
            AppEvent::ExportPush(c) => {
                if let Some(dialog) = &mut self.export {
                    dialog.input.push(c);
                    dialog.confirm_overwrite = false;
                    dialog.message = None;
                }
            }
            AppEvent::ExportPop => {
                if let Some(dialog) = &mut self.export {
                    dialog.input.pop();
                    dialog.confirm_overwrite = false;
                    dialog.message = None;
                }
            }
            AppEvent::ExportCancel => self.export = None,
            AppEvent::ExportConfirm => self.confirm_export(),
        }
    }

    /// The repo under the cursor, if any repo is visible.
    pub fn selected(&self) -> Option<&RepoView> {
        let index = *self.visible.get(self.table.selected()?)?;
        self.repos.get(index)
    }

    /// The footer message, while it is still fresh.
    pub fn status(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_secs() < 4)
            .map(|(message, _)| message.as_str())
    }

    /// Quitting with marks nobody exported offers to export them first, so a session of
    /// browsing is never lost to a reflexive `q`.
    fn request_quit(&mut self) {
        if self.unsaved_marks && !self.marked.is_empty() {
            self.open_export(AfterExport::Quit);
        } else {
            self.running = false;
        }
    }

    /// Mark or unmark the repo under the cursor, then step down — so a run of repos can be
    /// marked by holding space.
    fn toggle_mark(&mut self) {
        let Some(position) = self.table.selected() else {
            return;
        };
        let index = self.visible[position];
        if !self.marked.remove(&index) {
            self.marked.insert(index);
        }
        self.unsaved_marks = true;
        // A leftover "nothing marked" would contradict the mark just made.
        self.status = None;
        self.select(position + 1);
    }

    fn open_export(&mut self, after: AfterExport) {
        if self.marked.is_empty() {
            self.status = Some((
                "nothing marked — press space on a repo to mark it".to_string(),
                Instant::now(),
            ));
            return;
        }
        self.export = Some(ExportDialog {
            input: self.default_export.clone(),
            after,
            confirm_overwrite: false,
            message: None,
        });
    }

    fn confirm_export(&mut self) {
        let Some(dialog) = &mut self.export else {
            return;
        };
        let name = dialog.input.trim();
        if name.is_empty() {
            dialog.message = Some("enter a file name".to_string());
            return;
        }
        let path = PathBuf::from(name);
        if path.exists() && !dialog.confirm_overwrite {
            dialog.confirm_overwrite = true;
            dialog.message = Some(format!(
                "{} already exists — enter again to overwrite",
                path.display()
            ));
            return;
        }

        let marked = self.marked.iter().map(|&index| &self.repos[index]);
        match export::write(&path, marked) {
            Ok(count) => {
                let after = dialog.after;
                let message = format!("exported {count} repos → {}", path.display());
                self.export = None;
                self.unsaved_marks = false;
                match after {
                    AfterExport::Stay => self.status = Some((message, Instant::now())),
                    AfterExport::Quit => {
                        self.exit_message = Some(message);
                        self.running = false;
                    }
                }
            }
            Err(error) => {
                dialog.confirm_overwrite = false;
                dialog.message = Some(format!("could not write {}: {error}", path.display()));
            }
        }
    }

    fn move_by(&mut self, delta: isize) {
        match self.focus {
            Focus::Repos => {
                let current = self.table.selected().unwrap_or(0);
                self.select(current.saturating_add_signed(delta));
            }
            Focus::Detail => {
                let step = delta.unsigned_abs().min(u16::MAX as usize) as u16;
                self.detail_scroll = if delta < 0 {
                    self.detail_scroll.saturating_sub(step)
                } else {
                    self.detail_scroll.saturating_add(step)
                };
            }
        }
    }

    fn select(&mut self, position: usize) {
        if self.visible.is_empty() {
            self.table.select(None);
            return;
        }
        let position = position.min(self.visible.len() - 1);
        if self.table.selected() != Some(position) {
            self.detail_scroll = 0;
        }
        self.table.select(Some(position));
    }

    /// Recompute `visible`, keeping the same repo selected when it survives the change.
    fn refilter(&mut self) {
        let previous = self
            .table
            .selected()
            .and_then(|position| self.visible.get(position))
            .copied();
        let query = self.query.to_lowercase();

        self.visible = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, repo)| match self.filter {
                Filter::All => true,
                Filter::Only(outcome) => repo.outcome == outcome,
            })
            .filter(|(_, repo)| !self.multi_stem_only || repo.stems.len() > 1)
            .filter(|(_, repo)| repo.matches(&query))
            .map(|(index, _)| index)
            .collect();

        let position = previous
            .and_then(|index| self.visible.iter().position(|&i| i == index))
            .unwrap_or(0);
        self.table.select(None);
        self.select(position);
    }
}
