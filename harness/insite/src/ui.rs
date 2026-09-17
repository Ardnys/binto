use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Table,
    },
};

use crate::app::{AfterExport, App, Filter, Focus};
use crate::model::{Dim, Outcome, RepoView};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const SELECTED_BG: Color = Color::Rgb(45, 50, 62);

pub fn render(frame: &mut Frame, app: &mut App) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [list, detail] =
        Layout::horizontal([Constraint::Percentage(36), Constraint::Fill(1)]).areas(body);

    render_header(frame, app, header);
    render_repos(frame, app, list);
    render_detail(frame, app, detail);
    render_footer(frame, app, footer);
    if app.export.is_some() {
        render_export(frame, app, body);
    }
}

const MARK: Color = Color::Magenta;

fn render_export(frame: &mut Frame, app: &App, area: Rect) {
    let Some(dialog) = &app.export else {
        return;
    };
    let width = area.width.saturating_sub(4).min(80);
    let height = 8.min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };

    let (title, hints) = match dialog.after {
        AfterExport::Stay => (" Export marked repositories ", "enter export · esc cancel"),
        AfterExport::Quit => (
            " Export before quitting? ",
            "enter export & quit · esc keep browsing · ctrl-c quit without saving",
        ),
    };
    let message = match &dialog.message {
        Some(text) => Line::styled(
            format!(" {text}"),
            Style::new().fg(if dialog.confirm_overwrite {
                Color::Yellow
            } else {
                Color::Red
            }),
        ),
        None => Line::raw(""),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} marked", app.marked.len()),
                Style::new().fg(MARK).bold(),
            ),
            Span::styled(
                "  → JSONL, one repo per line, re-runnable with runner --dataset",
                Style::new().fg(MUTED),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(" file  ", Style::new().fg(MUTED)),
            Span::styled(dialog.input.clone(), Style::new().bold()),
            Span::styled("█", Style::new().fg(ACCENT)),
        ]),
        Line::raw(""),
        message,
        Line::styled(format!(" {hints}"), Style::new().fg(MUTED)),
    ];

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(MARK))
        .title(Span::styled(title, Style::new().fg(MARK).bold()));

    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn outcome_color(outcome: Outcome) -> Color {
    match outcome {
        Outcome::AutoSelected => Color::Green,
        Outcome::NeedsInteraction => Color::Yellow,
        Outcome::NoMatch => Color::Red,
        Outcome::Error => Color::Magenta,
    }
}

fn tier_color(tier: Option<i64>) -> Color {
    match tier {
        Some(0) => Color::Green,
        Some(1) => Color::Yellow,
        Some(_) => Color::Red,
        None => Color::Gray,
    }
}

/// Platform mismatches are expected and quiet; packaging gaps are where binto could grow,
/// so they stand out.
fn reason_color(reason: &str) -> Color {
    match reason {
        "foreign_os" | "foreign_arch" => Color::Blue,
        "package" | "unsupported_archive" => Color::Magenta,
        "sidecar" | "sbom" => MUTED,
        _ => Color::Gray,
    }
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let border = if focused { ACCENT } else { MUTED };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Span::styled(
            title,
            Style::new()
                .fg(if focused { ACCENT } else { Color::Gray })
                .bold(),
        ))
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let total = app.repos.len();
    let mut spans = vec![
        Span::styled(" insite ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw(" "),
        Span::styled(app.source.clone(), Style::new().fg(MUTED)),
        Span::raw("   "),
    ];

    let chip = |label: String, color: Color, active: bool| {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::new().fg(Color::Black).bg(color).bold(),
            )
        } else {
            Span::styled(format!(" {label} "), Style::new().fg(color))
        }
    };

    spans.push(chip(
        format!("0 all {total}"),
        Color::Gray,
        app.filter == Filter::All,
    ));
    for (key, outcome) in Outcome::ALL.into_iter().enumerate() {
        let count = app.counts[outcome.index()];
        if outcome == Outcome::Error && count == 0 {
            continue;
        }
        spans.push(chip(
            format!(
                "{} {} {} {count}",
                key + 1,
                outcome.glyph(),
                outcome.label()
            ),
            outcome_color(outcome),
            app.filter == Filter::Only(outcome),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(chip(
        "m ⑂ multi-stem".to_string(),
        Color::Magenta,
        app.multi_stem_only,
    ));

    frame.render_widget(Line::from(spans), area);
}

fn render_repos(frame: &mut Frame, app: &mut App, area: Rect) {
    let marked = if app.marked.is_empty() {
        String::new()
    } else {
        format!("· {} marked ", app.marked.len())
    };
    let mut block = pane(
        format!(
            " Repositories {}/{} {marked}",
            app.visible.len(),
            app.repos.len()
        ),
        app.focus == Focus::Repos,
    );
    if !app.query.is_empty() {
        block = block.title_bottom(Line::from(vec![
            Span::styled(" /", Style::new().fg(ACCENT)),
            Span::raw(format!("{} ", app.query)),
        ]));
    }

    if app.visible.is_empty() {
        let empty = Paragraph::new("no repositories match")
            .fg(MUTED)
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let rows = app.visible.iter().map(|&index| {
        let repo = &app.repos[index];
        let stems = if repo.stems.len() > 1 {
            Span::styled(
                format!("⑂{}", repo.stems.len()),
                Style::new().fg(Color::Magenta),
            )
        } else {
            Span::raw("")
        };
        let is_marked = app.marked.contains(&index);
        Row::new([
            Cell::from(if is_marked {
                Span::styled("✚", Style::new().fg(MARK).bold())
            } else {
                Span::raw(" ")
            }),
            Cell::from(Span::styled(
                repo.outcome.glyph(),
                Style::new().fg(outcome_color(repo.outcome)),
            )),
            Cell::from(if is_marked {
                Span::styled(repo.repo.as_str(), Style::new().fg(MARK))
            } else {
                Span::raw(repo.repo.as_str())
            }),
            Cell::from(
                Line::from(format!("{}/{}", repo.survivors.len(), repo.n_assets))
                    .right_aligned()
                    .fg(MUTED),
            ),
            Cell::from(stems),
        ])
    });

    let highlight = if app.focus == Focus::Repos {
        Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(7),
            Constraint::Length(4),
        ],
    )
    .block(block)
    .column_spacing(1)
    .row_highlight_style(highlight)
    .highlight_symbol("▌");

    frame.render_stateful_widget(table, area, &mut app.table);
}

fn render_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = pane(" Release ".to_string(), app.focus == Focus::Detail);

    let Some(repo) = app.selected() else {
        let empty = Paragraph::new("select a repository").fg(MUTED).block(block);
        frame.render_widget(empty, area);
        return;
    };

    let lines = detail_lines(repo);
    let height = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(height);
    let scroll = (app.detail_scroll as usize).min(max_scroll);
    app.detail_scroll = scroll as u16;

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .scroll((scroll as u16, 0));
    frame.render_widget(paragraph, area);

    if max_scroll > 0 {
        let mut state = ScrollbarState::new(max_scroll).position(scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut state,
        );
    }
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$}")
}

fn section(title: &str, count: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(title.to_string(), Style::new().fg(ACCENT).bold()),
        Span::styled(format!("  {count}"), Style::new().fg(MUTED)),
    ])
}

fn detail_lines(repo: &RepoView) -> Vec<Line<'static>> {
    let color = outcome_color(repo.outcome);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {} ", repo.repo), Style::new().bold()),
            Span::styled(
                format!(" {} {} ", repo.outcome.glyph(), repo.outcome.label()),
                Style::new().fg(Color::Black).bg(color).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled(" tag      ", Style::new().fg(MUTED)),
            Span::styled(repo.tag.clone(), Style::new().fg(Color::Yellow).bold()),
        ]),
        Line::from(vec![
            Span::styled(" checksum ", Style::new().fg(MUTED)),
            match &repo.checksum {
                Some(name) => Span::raw(name.clone()),
                None => Span::styled("none found", Style::new().fg(MUTED)),
            },
        ]),
    ];

    if repo.stems.len() > 1 {
        lines.push(Line::from(vec![
            Span::styled(" binaries ", Style::new().fg(MUTED)),
            Span::styled(
                format!("⑂ {} — {}", repo.stems.len(), repo.stems.join(", ")),
                Style::new().fg(Color::Magenta).bold(),
            ),
        ]));
    }

    lines.push(Line::raw(""));
    lines.push(funnel(repo));
    lines.push(Line::raw(""));

    if let Some(error) = &repo.error {
        lines.push(section(" ERROR", 1));
        lines.push(Line::styled(
            format!(" {error}"),
            Style::new().fg(Color::Red),
        ));
        lines.push(Line::raw(""));
    }

    if !repo.survivors.is_empty() {
        lines.push(section(" RANKED", repo.survivors.len()));
        lines.extend(survivor_lines(repo));
        lines.push(Line::raw(""));
    }

    if !repo.rejected.is_empty() {
        lines.push(section(" REJECTED", repo.rejected.len()));
        lines.extend(rejected_lines(repo));
        lines.push(Line::raw(""));
    }

    if !repo.raw_trace.is_empty() {
        lines.push(section(" RAW STDERR", repo.raw_trace.len()));
        for raw in &repo.raw_trace {
            lines.push(Line::styled(format!(" {raw}"), Style::new().fg(MUTED)));
        }
    }

    lines
}

/// `12 assets → 8 rejected → 4 ranked → 2 tied`: where the release's assets went.
fn funnel(repo: &RepoView) -> Line<'static> {
    let arrow = || Span::styled("  →  ", Style::new().fg(MUTED));
    let end = match repo.outcome {
        Outcome::AutoSelected => Span::styled("1 selected", Style::new().fg(Color::Green).bold()),
        Outcome::NeedsInteraction => Span::styled(
            format!("{} tied", repo.survivors.iter().filter(|s| s.tied).count()),
            Style::new().fg(Color::Yellow).bold(),
        ),
        Outcome::NoMatch => Span::styled("nothing installable", Style::new().fg(Color::Red).bold()),
        Outcome::Error => Span::styled("binto failed", Style::new().fg(Color::Magenta).bold()),
    };
    Line::from(vec![
        Span::raw(format!(" {} assets", repo.n_assets)),
        arrow(),
        Span::styled(
            format!("{} rejected", repo.rejected.len()),
            Style::new().fg(Color::Gray),
        ),
        arrow(),
        Span::raw(format!("{} ranked", repo.survivors.len())),
        arrow(),
        end,
    ])
}

fn survivor_lines(repo: &RepoView) -> Vec<Line<'static>> {
    // Asset names are what this tool is for, so they come first and never clip. The tier
    // columns matter least and absorb the overflow on a narrow pane.
    let name_width = repo
        .survivors
        .iter()
        .map(|s| s.name.chars().count())
        .chain(["asset".len()])
        .max()
        .unwrap_or(0);
    let stem_width = repo
        .survivors
        .iter()
        .map(|s| s.stem.as_deref().map_or(1, |stem| stem.chars().count()))
        .chain(["stem".len()])
        .max()
        .unwrap_or(4);
    let dim_width = |i: usize, header: &str| {
        repo.survivors
            .iter()
            .map(|s| display_label(&s.dims[i]).chars().count())
            .chain([header.len()])
            .max()
            .unwrap_or(0)
    };
    let widths = [
        dim_width(0, "arch"),
        dim_width(1, "os"),
        dim_width(2, "libc"),
        dim_width(3, "fmt"),
    ];

    let header = format!(
        "      {}  {}  {}  {}  {}  {}",
        pad("asset", name_width),
        pad("stem", stem_width),
        pad("arch", widths[0]),
        pad("os", widths[1]),
        pad("libc", widths[2]),
        pad("fmt", widths[3]),
    );
    let mut lines = vec![Line::styled(header, Style::new().fg(MUTED))];

    for (rank, survivor) in repo.survivors.iter().enumerate() {
        let marker = if survivor.selected {
            Span::styled(" ★", Style::new().fg(Color::Green).bold())
        } else if survivor.tied {
            Span::styled(" ◆", Style::new().fg(Color::Yellow))
        } else {
            Span::raw("  ")
        };
        let name_style = if survivor.selected {
            Style::new().bold()
        } else {
            Style::new()
        };

        let mut spans = vec![
            marker,
            Span::styled(format!("{:>3} ", rank + 1), Style::new().fg(MUTED)),
            Span::styled(pad(&survivor.name, name_width), name_style),
            Span::raw("  "),
            match &survivor.stem {
                Some(stem) if !stem.is_empty() => {
                    Span::styled(pad(stem, stem_width), Style::new().fg(ACCENT))
                }
                _ => Span::styled(pad("—", stem_width), Style::new().fg(MUTED)),
            },
        ];
        for (dim, width) in survivor.dims.iter().zip(widths) {
            spans.push(Span::raw("  "));
            let shown = Dim {
                label: display_label(dim).to_string(),
                tier: dim.tier,
            };
            spans.push(dim_span(&shown, width));
        }
        lines.push(Line::from(spans));

        if !survivor.notes.is_empty() {
            lines.push(Line::styled(
                format!("        ↳ {}", survivor.notes.join("  ·  ")),
                Style::new().fg(MUTED).italic(),
            ));
        }
    }
    lines
}

/// `unspecified` is most of what a C/C++ release states, and spelling it out on every column
/// pushes the asset name off the pane. The tier colour already carries the meaning.
fn display_label(dim: &Dim) -> &str {
    if dim.label == "unspecified" {
        "—"
    } else {
        &dim.label
    }
}

fn dim_span(dim: &Dim, width: usize) -> Span<'static> {
    Span::styled(
        pad(&dim.label, width),
        Style::new().fg(tier_color(dim.tier)),
    )
}

fn rejected_lines(repo: &RepoView) -> Vec<Line<'static>> {
    let mut rejected: Vec<_> = repo.rejected.iter().collect();
    rejected.sort_by(|a, b| a.reason.cmp(&b.reason).then_with(|| a.name.cmp(&b.name)));

    let reason_width = rejected.iter().map(|r| r.reason.len()).max().unwrap_or(0);
    let marker_width = rejected
        .iter()
        .map(|r| r.marker.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    let mut previous: Option<&str> = None;
    for r in rejected {
        // Print the reason once per group so the eye reads it as a heading.
        let reason = if previous == Some(r.reason.as_str()) {
            Span::raw(pad("", reason_width))
        } else {
            Span::styled(
                pad(&r.reason, reason_width),
                Style::new().fg(reason_color(&r.reason)),
            )
        };
        previous = Some(r.reason.as_str());

        lines.push(Line::from(vec![
            Span::raw(" "),
            reason,
            Span::raw("  "),
            Span::styled(pad(&r.marker, marker_width), Style::new().fg(MUTED)),
            Span::raw("  "),
            Span::styled(r.name.clone(), Style::new().fg(Color::Gray)),
        ]));
    }
    lines
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if app.searching {
        let line = Line::from(vec![
            Span::styled(" /", Style::new().fg(ACCENT).bold()),
            Span::raw(app.query.clone()),
            Span::styled("█", Style::new().fg(ACCENT)),
            Span::styled(
                "   repo or asset name · enter keep · esc clear",
                Style::new().fg(MUTED),
            ),
        ]);
        frame.render_widget(line, area);
        return;
    }

    if let Some(status) = app.status() {
        let line = Line::from(vec![
            Span::styled(" ✚ ", Style::new().fg(MARK).bold()),
            Span::styled(status.to_string(), Style::new().fg(MARK)),
        ]);
        frame.render_widget(line, area);
        return;
    }

    let keys: &[(&str, &str)] = &[
        ("j/k", "move"),
        ("tab h/l", "pane"),
        ("0-4", "outcome"),
        ("m", "multi-stem"),
        ("/", "search"),
        ("space", "mark"),
        ("e", "export"),
        ("q", "quit"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (key, action) in keys {
        spans.push(Span::styled(*key, Style::new().fg(ACCENT)));
        spans.push(Span::styled(
            format!(" {action}   "),
            Style::new().fg(MUTED),
        ));
    }
    frame.render_widget(Line::from(spans), area);
}
