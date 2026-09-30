use chrono::Utc;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Sparkline},
};

use super::{fmt, panel, right_title, tables};
use crate::{
    app::{App, ListId, Panel},
    fetch::SectionKind,
    github::models::Calendar,
    theme::blend,
};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let panels = if app.zoom { vec![(app.panel, area)] } else { layout(app, area) };
    for (p, r) in panels {
        app.rects.borrow_mut().push((p, r));
        draw_panel(f, app, p, r, p == app.panel, app.zoom);
    }
}

fn layout(app: &App, area: Rect) -> Vec<(Panel, Rect)> {
    if area.width >= 120 && area.height >= 26 {
        let [top, bottom] = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);
        let [repos, actions, deploys] =
            Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(40), Constraint::Percentage(30)])
                .areas(top);
        let [pulse, pulls, issues, explore] = Layout::horizontal([
            Constraint::Percentage(30),
            Constraint::Percentage(24),
            Constraint::Percentage(23),
            Constraint::Percentage(23),
        ])
        .areas(bottom);
        vec![
            (Panel::Repos, repos),
            (Panel::Actions, actions),
            (Panel::Deploys, deploys),
            (Panel::Pulse, pulse),
            (Panel::Pulls, pulls),
            (Panel::Issues, issues),
            (Panel::Explore, explore),
        ]
    } else if area.width >= 70 && area.height >= 18 {
        let rows: [Rect; 3] = Layout::vertical([Constraint::Ratio(1, 3); 3]).areas(area);
        let split = |r: Rect| -> [Rect; 2] { Layout::horizontal([Constraint::Percentage(50); 2]).areas(r) };
        let [a, b] = split(rows[0]);
        let [c, d] = split(rows[1]);
        let [e, g] = split(rows[2]);
        vec![
            (Panel::Repos, a),
            (Panel::Actions, b),
            (Panel::Deploys, c),
            (Panel::Pulse, d),
            (Panel::Pulls, e),
            (Panel::Issues, g),
        ]
    } else {
        let p = if app.panel == Panel::Explore { Panel::Repos } else { app.panel };
        vec![(p, area)]
    }
}

fn draw_panel(f: &mut Frame, app: &App, p: Panel, area: Rect, focused: bool, zoom: bool) {
    let ic = &app.icons;
    let t = &app.theme;
    let d = app.d();
    let compact = !zoom;
    match p {
        Panel::Repos => {
            let block = panel(app, Some(2), ic.repo, "repos", focused).title(right_title(
                app,
                format!("{} {}", d.watched.len(), if app.repos_all { "all" } else { "watched" }),
            ));
            tables::render(
                f,
                app,
                area,
                block,
                ListId::Repos,
                tables::repos(app, compact),
                d.loaded,
                "no repos, add some under [[hosts]] repos",
                focused,
            );
        }
        Panel::Actions => {
            let running = d.runs.values().flatten().filter(|r| r.is_active()).count();
            let failed = d
                .runs
                .values()
                .filter_map(|runs| runs.first())
                .filter(|r| r.conclusion.as_deref() == Some("failure"))
                .count();
            let mut right = vec![];
            if running > 0 {
                right.push(Span::styled(
                    format!(" {} {running} running", super::spinner(app)),
                    Style::new().fg(t.yellow),
                ));
            }
            if failed > 0 {
                right.push(Span::styled(format!(" ✗ {failed} failing"), Style::new().fg(t.red)));
            }
            right.push(Span::raw(" "));
            let block = panel(app, Some(3), ic.run, "actions", focused).title(Line::from(right).right_aligned());
            tables::render(
                f,
                app,
                area,
                block,
                ListId::Runs,
                tables::runs(app, compact, area),
                d.loaded,
                "no workflow runs",
                focused,
            );
        }
        Panel::Deploys => {
            let block = panel(app, Some(6), ic.deploy, "deploys", focused);
            tables::render(
                f,
                app,
                area,
                block,
                ListId::Deploys,
                tables::deploys(app, compact),
                d.loaded,
                "no deployments found (see [deploy] in config)",
                focused,
            );
        }
        Panel::Pulse => draw_pulse(f, app, area, focused),
        Panel::Pulls | Panel::Issues => {
            let (kind, num, icon, id, sec) = if p == Panel::Pulls {
                (SectionKind::Pulls, 4, ic.pr, ListId::Pulls, app.pull_sec)
            } else {
                (SectionKind::Issues, 5, ic.issue, ListId::Issues, app.issue_sec)
            };
            let titles = app.section_titles(kind);
            let title = titles.get(sec).copied().unwrap_or("");
            let s = app.section(kind);
            let label = if p == Panel::Pulls { "pulls" } else { "issues" };
            let block = panel(app, Some(num), icon, &format!("{label} · {}", title.to_lowercase()), focused)
                .title(right_title(app, format!("{}  {}/{}", s.map(|s| s.total).unwrap_or(0), sec + 1, titles.len())));
            let loaded = s.is_some_and(|s| s.loaded);
            tables::render(
                f,
                app,
                area,
                block,
                id,
                tables::issues(app, kind, compact),
                loaded,
                "all clear ✨",
                focused,
            );
        }
        Panel::Explore => {
            let tab = |i: usize, name: &str| {
                let st = if app.explore_tab == i {
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::new().fg(t.dim)
                };
                Span::styled(name.to_string(), st)
            };
            let block = panel(app, Some(7), ic.flame, "explore", focused).title(
                Line::from(vec![tab(0, " trending"), Span::styled(" · ", Style::new().fg(t.dim)), tab(1, "top ")])
                    .right_aligned(),
            );
            let loaded = !d.trending.is_empty() || !d.top.is_empty() || d.last_refresh.is_some();
            tables::render(
                f,
                app,
                area,
                block,
                ListId::Explore,
                tables::explore(app, compact),
                loaded,
                "nothing trending",
                focused,
            );
        }
    }
}

/// `━━━━━━────  87%`-style meter.
fn meter(app: &App, label: &str, width: u16, ratio: f64, color: Color, value: String) -> Line<'static> {
    let t = &app.theme;
    let label_w = 7;
    let value_w = value.chars().count() as u16 + 1;
    let bar_w = width.saturating_sub(label_w + value_w + 1) as usize;
    let filled = ((ratio.clamp(0.0, 1.0) * bar_w as f64).round() as usize).min(bar_w);
    Line::from(vec![
        Span::styled(format!("{label:<6} "), Style::new().fg(t.dim)),
        Span::styled("━".repeat(filled), Style::new().fg(color)),
        Span::styled("─".repeat(bar_w - filled), Style::new().fg(t.border)),
        Span::styled(format!(" {value}"), Style::new().fg(color).add_modifier(Modifier::BOLD)),
    ])
}

fn level_color(app: &App, level: u8) -> Color {
    let t = &app.theme;
    match level {
        0 => t.sel,
        1 => blend(t.sel, t.green, 0.35),
        2 => blend(t.sel, t.green, 0.6),
        3 => blend(t.sel, t.green, 0.8),
        _ => t.green,
    }
}

fn heatmap(app: &App, cal: &Calendar, width: u16) -> Vec<Line<'static>> {
    let t = &app.theme;
    let cols = (width.saturating_sub(2)) as usize;
    let weeks = &cal.weeks[cal.weeks.len().saturating_sub(cols)..];
    let max = weeks.iter().flat_map(|w| &w.days).map(|d| d.count).max().unwrap_or(0).max(1);
    let mut lines = vec![Line::from(vec![
        Span::styled(fmt::count(cal.total), Style::new().fg(t.green).add_modifier(Modifier::BOLD)),
        Span::styled(" contributions · last year", Style::new().fg(t.dim)),
    ])];
    for wd in 0..7u8 {
        let label = match wd {
            1 => "M ",
            3 => "W ",
            5 => "F ",
            _ => "  ",
        };
        let mut spans = vec![Span::styled(label, Style::new().fg(t.dim))];
        for w in weeks {
            match w.days.iter().find(|d| d.weekday == wd) {
                Some(d) => {
                    let level = if d.count == 0 { 0 } else { ((d.count * 4).div_ceil(max)).clamp(1, 4) as u8 };
                    spans.push(Span::styled("■", Style::new().fg(level_color(app, level))));
                }
                None => spans.push(Span::raw(" ")),
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn draw_pulse(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let t = &app.theme;
    let ic = &app.icons;
    let d = app.d();
    let block = panel(app, Some(1), ic.pulse, "pulse", focused);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let runs: Vec<_> = d.runs.values().flatten().collect();
    let decided: Vec<_> = runs
        .iter()
        .filter(|r| matches!(r.conclusion.as_deref(), Some("success" | "failure" | "timed_out" | "startup_failure")))
        .collect();
    let ok = decided.iter().filter(|r| r.conclusion.as_deref() == Some("success")).count();
    let failed = decided.len() - ok;
    let running = runs.iter().filter(|r| r.is_active()).count();
    let ci = if decided.is_empty() { 0.0 } else { ok as f64 / decided.len() as f64 };

    let days = 14usize;
    let today = Utc::now().date_naive();
    let mut per_day = vec![0u64; days];
    for r in &runs {
        let ago = (today - r.created_at.date_naive()).num_days();
        if (0..days as i64).contains(&ago) {
            per_day[days - 1 - ago as usize] += 1;
        }
    }

    let show_cal = d.calendar.as_ref().is_some_and(|c| !c.weeks.is_empty()) && inner.height >= 16;
    let mut cons = vec![];
    if show_cal {
        cons.push(Constraint::Length(9));
    }
    cons.extend([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
    ]);
    let areas = Layout::vertical(cons).split(inner);
    let mut i = 0;
    if show_cal {
        let cal = d.calendar.as_ref().unwrap();
        f.render_widget(Paragraph::new(heatmap(app, cal, inner.width)), areas[0]);
        i = 1;
    }

    let w = inner.width;
    let ci_color = if ci >= 0.9 {
        t.green
    } else if ci >= 0.7 {
        t.yellow
    } else {
        t.red
    };
    let ci_value = if decided.is_empty() { "–".to_string() } else { format!("{:>3.0}%", ci * 100.0) };
    f.render_widget(Paragraph::new(meter(app, "ci ok", w, ci, ci_color, ci_value)), areas[i]);

    let rates = app.client().rates();
    let rate_line = |label: &str, r: Option<crate::github::models::Rate>| match r {
        Some(r) => {
            let frac = r.remaining as f64 / r.limit.max(1) as f64;
            let c = if frac < 0.1 {
                t.red
            } else if frac < 0.3 {
                t.yellow
            } else {
                t.cyan
            };
            meter(app, label, w, frac, c, format!("{}/{}", fmt::count(r.remaining), fmt::count(r.limit)))
        }
        None => meter(app, label, w, 0.0, t.dim, "–".into()),
    };
    f.render_widget(Paragraph::new(rate_line("api", rates.core)), areas[i + 1]);
    f.render_widget(Paragraph::new(rate_line("search", rates.search)), areas[i + 2]);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("runs {days}d "), Style::new().fg(t.dim)),
            Span::styled(format!("✓ {ok} "), Style::new().fg(t.green)),
            Span::styled(format!("✗ {failed} "), Style::new().fg(t.red)),
            Span::styled(format!("▶ {running}"), Style::new().fg(t.yellow)),
        ])),
        areas[i + 3],
    );
    let spark_area = areas[i + 4];
    let reps = (spark_area.width as usize / days).max(1);
    let data: Vec<u64> = per_day.iter().flat_map(|v| std::iter::repeat_n(*v, reps)).collect();
    f.render_widget(Sparkline::default().data(data).style(Style::new().fg(t.accent2)), spark_area);

    let stars: u64 = d.watched.iter().map(|r| r.stargazers_count).sum();
    let open: u64 = d.watched.iter().map(|r| r.open_issues_count).sum();
    let prs: u64 = d.pulls.first().map(|s| s.total).unwrap_or(0);
    let unread = d.notifications.iter().filter(|n| n.unread).count();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{} {} ", ic.star, fmt::count(stars)), Style::new().fg(t.yellow)),
            Span::styled(format!("{} {open} ", ic.issue), Style::new().fg(t.green)),
            Span::styled(format!("{} {prs} review ", ic.pr), Style::new().fg(t.magenta)),
            Span::styled(format!("{} {unread}", ic.bell), Style::new().fg(t.orange)),
        ])),
        areas[i + 5],
    );
}
