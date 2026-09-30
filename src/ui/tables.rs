use std::ops::Range;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Row, Table, TableState},
};

use super::{dstate_icon, fmt, highlight_style, list_state, placeholder, run_icon};
use crate::{
    app::{App, JobRow, ListId},
    deploy::{Source, TierStatus},
    fetch::SectionKind,
    fuzzy::highlight,
    github::models::span,
    theme,
};

/// A table plus the length of the full list and the index of its first row (> 0 when only
/// the visible window of a long list was built).
pub type Built = (Table<'static>, usize, usize);

/// Renders a table inside `block`, or a placeholder when empty.
#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    app: &App,
    area: Rect,
    block: Block,
    id: ListId,
    (table, len, start): Built,
    loaded: bool,
    empty: &str,
    focused: bool,
) {
    let inner = block.inner(area);
    f.render_widget(block, area);
    if len == 0 {
        let y = inner.y + inner.height.saturating_sub(1) / 2;
        f.render_widget(placeholder(app, loaded, empty), Rect { y, height: 1, ..inner });
        return;
    }
    let table = table
        .row_highlight_style(highlight_style(app, focused))
        .highlight_symbol(if focused { "▌" } else { " " })
        .column_spacing(1);
    let mut st = list_state(app, id, len);
    if start == 0 {
        f.render_stateful_widget(table, inner, &mut st);
    } else {
        let mut local = TableState::new().with_selected(st.selected().map(|s| s - start));
        f.render_stateful_widget(table, inner, &mut local);
    }
}

/// Rows of a long list that fit in `area` (below borders and header), scrolled the way
/// ratatui would so the selection stays in view. Only these rows need to be built.
fn window(app: &App, id: ListId, len: usize, area: Rect) -> Range<usize> {
    let rows = area.height.saturating_sub(3).max(1) as usize;
    let mut states = app.states.borrow_mut();
    let st = states.entry(id).or_default();
    let last = len.saturating_sub(1);
    let sel = st.selected().unwrap_or(0).min(last);
    let mut off = st.offset().min(last);
    if sel < off {
        off = sel;
    } else if sel >= off + rows {
        off = sel + 1 - rows;
    }
    *st.offset_mut() = off;
    off..(off + rows).min(len)
}

fn hl(app: &App) -> Style {
    Style::new().fg(app.theme.orange).add_modifier(Modifier::BOLD)
}

fn dim(app: &App) -> Style {
    Style::new().fg(app.theme.dim)
}

fn header(app: &App, cols: &[&str]) -> Row<'static> {
    Row::new(cols.iter().map(|c| Cell::from(c.to_string())).collect::<Vec<_>>())
        .style(Style::new().fg(app.theme.dim).add_modifier(Modifier::BOLD))
}

/// `owner/name` with the owner dimmed and fuzzy matches highlighted.
fn repo_name(app: &App, full: &str, idx: &[u32], show_owner: bool) -> Line<'static> {
    let t = &app.theme;
    let slash = full.find('/').map(|i| full[..i].chars().count()).unwrap_or(0);
    if !show_owner && idx.is_empty() {
        let name = full.rsplit('/').next().unwrap_or(full);
        return Line::from(Span::styled(name.to_string(), Style::new().fg(t.fg)));
    }
    Line::from(highlight(
        full,
        idx,
        |i| if i <= slash { Style::new().fg(t.dim) } else { Style::new().fg(t.fg) },
        hl(app),
    ))
}

pub fn repos(app: &App, compact: bool) -> Built {
    let t = &app.theme;
    let ic = &app.icons;
    let rows_src = app.repo_rows();
    let len = rows_src.len();
    let rows: Vec<Row> = rows_src
        .iter()
        .map(|h| {
            let r = h.item;
            let (ci, cc) = app
                .repo_ci(r)
                .map(|run| run_icon(app, run.status.as_deref(), run.conclusion.as_deref()))
                .unwrap_or(("·", t.dim));
            let name = repo_name(app, &r.full_name, &h.idx, !compact);
            let mut name = name;
            if r.private {
                name.spans.push(Span::styled(format!(" {}", ic.lock), Style::new().fg(t.dim)));
            }
            let stars = Span::styled(fmt::count(r.stargazers_count), Style::new().fg(t.yellow));
            let pushed = Span::styled(fmt::ago_opt(r.pushed_at), dim(app));
            if compact {
                Row::new(vec![
                    Cell::from(Span::styled(ci, Style::new().fg(cc))),
                    Cell::from(name),
                    Cell::from(Line::from(stars).right_aligned()),
                    Cell::from(Line::from(pushed).right_aligned()),
                ])
            } else {
                let lang = r.language.clone().unwrap_or_default();
                let lc = theme::language_color(&lang, t.dim);
                Row::new(vec![
                    Cell::from(Span::styled(ci, Style::new().fg(cc))),
                    Cell::from(name),
                    Cell::from(Line::from(vec![Span::styled("● ", Style::new().fg(lc)), Span::styled(lang, dim(app))])),
                    Cell::from(Line::from(stars).right_aligned()),
                    Cell::from(Line::from(Span::styled(fmt::count(r.forks_count), dim(app))).right_aligned()),
                    Cell::from(
                        Line::from(Span::styled(r.open_issues_count.to_string(), Style::new().fg(t.green)))
                            .right_aligned(),
                    ),
                    Cell::from(Line::from(pushed).right_aligned()),
                ])
            }
        })
        .collect();
    let table = if compact {
        Table::new(rows, [Constraint::Length(1), Constraint::Fill(1), Constraint::Length(5), Constraint::Length(4)])
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(4),
                Constraint::Length(4),
            ],
        )
        .header(header(app, &["", "repository", "language", ic.star, ic.fork, ic.issue, "push"]))
    };
    (table, len, 0)
}

pub fn runs(app: &App, compact: bool, area: Rect) -> Built {
    let t = &app.theme;
    let src = app.run_rows();
    let len = src.len();
    let win = window(app, ListId::Runs, len, area);
    let start = win.start;
    let rows: Vec<Row> = src[win]
        .iter()
        .map(|h| {
            let r = h.item;
            let (icon, c) = run_icon(app, r.status.as_deref(), r.conclusion.as_deref());
            let repo_short = r.repo.rsplit('/').next().unwrap_or(&r.repo).to_string();
            let wf = Span::styled(r.workflow().to_string(), Style::new().fg(t.fg));
            let branch = Span::styled(r.branch().to_string(), Style::new().fg(t.cyan));
            let age = Span::styled(fmt::ago(r.created_at), dim(app));
            if compact {
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::new().fg(c))),
                    Cell::from(wf),
                    Cell::from(Span::styled(repo_short, Style::new().fg(t.accent2))),
                    Cell::from(branch),
                    Cell::from(Line::from(age).right_aligned()),
                ])
            } else {
                let dur_style = if r.is_active() { Style::new().fg(t.yellow) } else { dim(app) };
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::new().fg(c))),
                    Cell::from(wf),
                    Cell::from(Span::styled(r.title().to_string(), dim(app))),
                    Cell::from(Span::styled(repo_short, Style::new().fg(t.accent2))),
                    Cell::from(branch),
                    Cell::from(Span::styled(r.event.clone(), dim(app))),
                    Cell::from(Line::from(Span::styled(fmt::dur(r.duration()), dur_style)).right_aligned()),
                    Cell::from(Line::from(age).right_aligned()),
                ])
            }
        })
        .collect();
    let table = if compact {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Fill(3),
                Constraint::Fill(2),
                Constraint::Fill(2),
                Constraint::Length(4),
            ],
        )
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Fill(2),
                Constraint::Fill(3),
                Constraint::Fill(2),
                Constraint::Fill(1),
                Constraint::Length(12),
                Constraint::Length(7),
                Constraint::Length(4),
            ],
        )
        .header(header(app, &["", "workflow", "title", "repo", "branch", "event", "took", "age"]))
    };
    (table, len, start)
}

pub fn jobs(app: &App) -> Built {
    let t = &app.theme;
    let src = app.job_rows();
    let len = src.len();
    let rows: Vec<Row> = src
        .iter()
        .map(|row| match row {
            JobRow::Job(j) => {
                let (icon, c) = run_icon(app, j.status.as_deref(), j.conclusion.as_deref());
                let took = span(j.started_at, j.completed_at).map(fmt::dur).unwrap_or_default();
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::new().fg(c))),
                    Cell::from(Span::styled(j.name.clone(), Style::new().fg(t.fg).add_modifier(Modifier::BOLD))),
                    Cell::from(Line::from(Span::styled(took, dim(app))).right_aligned()),
                ])
            }
            JobRow::Step(s) => {
                let (icon, c) = run_icon(app, s.status.as_deref(), s.conclusion.as_deref());
                let took = span(s.started_at, s.completed_at).map(fmt::dur).unwrap_or_default();
                Row::new(vec![
                    Cell::from(""),
                    Cell::from(Line::from(vec![
                        Span::styled(format!("{icon} "), Style::new().fg(c)),
                        Span::styled(s.name.clone(), dim(app)),
                    ])),
                    Cell::from(Line::from(Span::styled(took, dim(app))).right_aligned()),
                ])
            }
        })
        .collect();
    (Table::new(rows, [Constraint::Length(1), Constraint::Fill(1), Constraint::Length(7)]), len, 0)
}

pub fn tier_cell(app: &App, tier: Option<&TierStatus>, compact: bool) -> Line<'static> {
    let t = &app.theme;
    let Some(e) = tier.and_then(|t| t.latest()) else {
        return Line::from(Span::styled("–", dim(app)));
    };
    let (icon, c) = dstate_icon(app, e.state);
    let label = match e.source {
        Source::Release => e.git_ref.clone(),
        Source::Environment | Source::Workflow => e.git_ref.clone(),
    };
    let n = if compact { 10 } else { 18 };
    Line::from(vec![
        Span::styled(format!("{icon} "), Style::new().fg(c)),
        Span::styled(fmt::trunc(&label, n), Style::new().fg(t.fg)),
        Span::styled(format!(" {}", fmt::ago(e.at)), dim(app)),
    ])
}

pub fn deploys(app: &App, compact: bool) -> Built {
    let t = &app.theme;
    let tiers: Vec<&str> = app.cfg.deploy.tiers.iter().map(|t| t.name.as_str()).collect();
    let src = app.deploy_rows();
    let len = src.len();
    let rows: Vec<Row> = src
        .iter()
        .map(|h| {
            let (name, ts) = h.item;
            let mut cells = vec![Cell::from(repo_name(app, name, &h.idx, !compact))];
            for tn in &tiers {
                cells.push(Cell::from(tier_cell(app, ts.iter().find(|x| x.tier == *tn), compact)));
            }
            Row::new(cells)
        })
        .collect();
    let mut widths = vec![Constraint::Fill(if compact { 3 } else { 2 })];
    widths.extend(tiers.iter().map(|_| Constraint::Fill(if compact { 3 } else { 2 })));
    let mut head = vec!["repository"];
    head.extend(tiers.iter());
    let table = Table::new(rows, widths).header(
        Row::new(head.iter().map(|h| Cell::from(h.to_string())).collect::<Vec<_>>())
            .style(Style::new().fg(t.dim).add_modifier(Modifier::BOLD)),
    );
    (table, len, 0)
}

pub fn issues(app: &App, kind: SectionKind, compact: bool) -> Built {
    let t = &app.theme;
    let ic = &app.icons;
    let src = app.issue_rows(kind);
    let len = src.len();
    let rows: Vec<Row> = src
        .iter()
        .map(|h| {
            let i = h.item;
            let (icon, c) = if i.is_pr() {
                if i.draft == Some(true) { (ic.pr, t.dim) } else { (ic.pr, t.green) }
            } else {
                (ic.issue, t.green)
            };
            let repo = i.repo();
            let repo_short = repo.rsplit('/').next().unwrap_or(&repo).to_string();
            let num = Span::styled(format!("#{}", i.number), dim(app));
            let title = Span::styled(i.title.clone(), Style::new().fg(t.fg));
            let age = Span::styled(fmt::ago(i.updated_at), dim(app));
            if compact {
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::new().fg(c))),
                    Cell::from(Line::from(vec![title])),
                    Cell::from(Span::styled(repo_short, Style::new().fg(t.accent2))),
                    Cell::from(Line::from(age).right_aligned()),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Span::styled(icon, Style::new().fg(c))),
                    Cell::from(Line::from(num).right_aligned()),
                    Cell::from(title),
                    Cell::from(Span::styled(repo_short, Style::new().fg(t.accent2))),
                    Cell::from(Span::styled(i.author().to_string(), Style::new().fg(t.cyan))),
                    Cell::from(
                        Line::from(Span::styled(
                            if i.comments > 0 { format!("{} {}", ic.comment, i.comments) } else { String::new() },
                            dim(app),
                        ))
                        .right_aligned(),
                    ),
                    Cell::from(Line::from(age).right_aligned()),
                ])
            }
        })
        .collect();
    let table = if compact {
        Table::new(rows, [Constraint::Length(1), Constraint::Fill(3), Constraint::Fill(1), Constraint::Length(4)])
    } else {
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(6),
                Constraint::Fill(4),
                Constraint::Fill(2),
                Constraint::Fill(1),
                Constraint::Length(5),
                Constraint::Length(4),
            ],
        )
        .header(header(app, &["", "#", "title", "repo", "author", "", "upd"]))
    };
    (table, len, 0)
}

pub fn explore(app: &App, compact: bool) -> Built {
    let t = &app.theme;
    let src = app.explore_rows();
    let len = src.len();
    let rows: Vec<Row> = src
        .iter()
        .map(|h| {
            let r = h.item;
            let lang = r.language.clone().unwrap_or_default();
            let lc = theme::language_color(&lang, t.dim);
            let stars = Span::styled(fmt::count(r.stargazers_count), Style::new().fg(t.yellow));
            let name = repo_name(app, &r.full_name, &h.idx, true);
            if compact {
                Row::new(vec![
                    Cell::from(Line::from(stars).right_aligned()),
                    Cell::from(name),
                    Cell::from(Span::styled("●", Style::new().fg(lc))),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Line::from(stars).right_aligned()),
                    Cell::from(name),
                    Cell::from(Line::from(vec![Span::styled("● ", Style::new().fg(lc)), Span::styled(lang, dim(app))])),
                    Cell::from(Span::styled(r.description.clone().unwrap_or_default(), dim(app))),
                ])
            }
        })
        .collect();
    let table = if compact {
        Table::new(rows, [Constraint::Length(5), Constraint::Fill(1), Constraint::Length(1)])
    } else {
        Table::new(rows, [Constraint::Length(6), Constraint::Fill(2), Constraint::Length(12), Constraint::Fill(3)])
    };
    (table, len, 0)
}

pub fn inbox(app: &App) -> Built {
    let t = &app.theme;
    let ic = &app.icons;
    let src = app.inbox_rows();
    let len = src.len();
    let rows: Vec<Row> = src
        .iter()
        .map(|h| {
            let n = h.item;
            let (icon, c) = match n.subject.kind.as_str() {
                "PullRequest" => (ic.pr, t.magenta),
                "Issue" => (ic.issue, t.green),
                "Release" => (ic.tag, t.cyan),
                "CheckSuite" | "WorkflowRun" => (ic.run, t.yellow),
                _ => (ic.bell, t.dim),
            };
            Row::new(vec![
                Cell::from(Span::styled(if n.unread { "●" } else { " " }, Style::new().fg(t.accent))),
                Cell::from(Span::styled(icon, Style::new().fg(c))),
                Cell::from(Span::styled(n.repository.full_name.clone(), Style::new().fg(t.accent2))),
                Cell::from(Span::styled(
                    n.subject.title.clone(),
                    if n.unread { Style::new().fg(t.fg).add_modifier(Modifier::BOLD) } else { dim(app) },
                )),
                Cell::from(Span::styled(n.reason.replace('_', " "), Style::new().fg(t.cyan))),
                Cell::from(Line::from(Span::styled(fmt::ago(n.updated_at), dim(app))).right_aligned()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(2),
            Constraint::Fill(5),
            Constraint::Length(16),
            Constraint::Length(4),
        ],
    )
    .header(header(app, &["", "", "repository", "subject", "reason", "upd"]));
    (table, len, 0)
}
