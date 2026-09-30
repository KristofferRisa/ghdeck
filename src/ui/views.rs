use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span, Text},
    widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph, Wrap},
};

use super::{dstate_icon, fmt, panel, right_title, run_icon, tables};
use crate::{
    app::{App, ListId, View},
    deploy::{Source, TierStatus},
    fetch::SectionKind,
    github::models::{Issue, Repo, WorkflowRun},
    theme::{self, parse_hex},
};

pub fn draw(f: &mut Frame, app: &App, area: Rect, v: View) {
    let wide = area.width >= 100;
    let split = |pct: u16| -> [Rect; 2] {
        if wide {
            Layout::horizontal([Constraint::Percentage(pct), Constraint::Percentage(100 - pct)]).areas(area)
        } else {
            [area, Rect::default()]
        }
    };
    let ic = &app.icons;
    let d = app.d();
    match v {
        View::Dashboard => {}
        View::Repos => {
            let [l, r] = split(55);
            let block = panel(app, Some(2), ic.repo, if app.repos_all { "all repos" } else { "watched repos" }, true)
                .title(right_title(app, format!("{}  a: toggle all · w: watch · ⏎ runs", app.list_len(ListId::Repos))));
            tables::render(f, app, l, block, ListId::Repos, tables::repos(app, false), d.loaded, "no repos", true);
            if wide {
                match app.selected_repo() {
                    Some(repo) => repo_detail(f, app, r, repo),
                    None => f.render_widget(panel(app, None, ic.repo, "details", false), r),
                }
            }
        }
        View::Actions => {
            let [l, r] = split(58);
            let block = panel(app, Some(3), ic.run, "workflow runs", !app.jobs_focus)
                .title(right_title(app, "⏎ jobs · R re-run · x cancel · o open".into()));
            tables::render(
                f,
                app,
                l,
                block,
                ListId::Runs,
                tables::runs(app, !wide, l),
                d.loaded,
                "no workflow runs",
                !app.jobs_focus,
            );
            if wide {
                run_detail(f, app, r);
            }
        }
        View::Pulls => sections(f, app, area, SectionKind::Pulls),
        View::Issues => sections(f, app, area, SectionKind::Issues),
        View::Deploys => {
            let [l, r] = split(60);
            let block = panel(app, Some(6), ic.deploy, "deployments", true);
            tables::render(
                f,
                app,
                l,
                block,
                ListId::Deploys,
                tables::deploys(app, !wide),
                d.loaded,
                "no deployments found",
                true,
            );
            if wide {
                deploy_detail(f, app, r);
            }
        }
        View::Explore => {
            let [l, r] = split(55);
            let t = &app.theme;
            let tab = |i: usize, name: &str| {
                let st = if app.explore_tab == i {
                    Style::new().fg(t.accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::new().fg(t.dim)
                };
                Span::styled(name.to_string(), st)
            };
            let block = panel(app, Some(7), ic.flame, "explore", true).title(
                Line::from(vec![
                    tab(0, " trending"),
                    Span::styled(" · ", Style::new().fg(t.dim)),
                    tab(1, "top "),
                    Span::styled("(h/l) ", Style::new().fg(t.dim)),
                ])
                .right_aligned(),
            );
            let loaded = d.last_refresh.is_some();
            tables::render(f, app, l, block, ListId::Explore, tables::explore(app, true), loaded, "nothing here", true);
            if wide {
                match app.explore_rows().into_iter().nth(app.sel(ListId::Explore)) {
                    Some(h) => repo_detail(f, app, r, h.item),
                    None => f.render_widget(panel(app, None, ic.repo, "details", false), r),
                }
            }
        }
        View::Inbox => {
            let block = panel(app, Some(8), ic.bell, "notifications", true)
                .title(right_title(app, format!("{} unread", d.notifications.iter().filter(|n| n.unread).count())));
            tables::render(
                f,
                app,
                area,
                block,
                ListId::Inbox,
                tables::inbox(app),
                d.last_refresh.is_some(),
                "inbox zero 🎉",
                true,
            );
        }
    }
}

fn kv(app: &App, k: &str, v: String, c: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:>9} "), Style::new().fg(app.theme.dim)),
        Span::styled(v, Style::new().fg(c)),
    ])
}

fn section_rule(app: &App, title: &str, width: u16) -> Line<'static> {
    let t = &app.theme;
    let rest = (width as usize).saturating_sub(title.chars().count() + 4);
    Line::from(vec![
        Span::styled("── ", Style::new().fg(t.border)),
        Span::styled(title.to_string(), Style::new().fg(t.accent2).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {}", "─".repeat(rest)), Style::new().fg(t.border)),
    ])
}

fn repo_detail(f: &mut Frame, app: &App, area: Rect, repo: &Repo) {
    let t = &app.theme;
    let ic = &app.icons;
    let d = app.d();
    let block = panel(app, None, ic.repo, "details", false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width;

    let mut head = vec![];
    let mut title = vec![Span::styled(repo.full_name.clone(), Style::new().fg(t.accent).add_modifier(Modifier::BOLD))];
    for (flag, label, c) in
        [(repo.private, "private", t.orange), (repo.archived, "archived", t.red), (repo.fork, "fork", t.dim)]
    {
        if flag {
            title.push(Span::styled(format!("  {label}"), Style::new().fg(c)));
        }
    }
    head.push(Line::from(title));
    if let Some(desc) = repo.description.as_deref().filter(|s| !s.is_empty()) {
        head.push(Line::from(Span::styled(desc.to_string(), Style::new().fg(t.fg))));
    }
    head.push(Line::from(vec![
        Span::styled(format!("{} {}  ", ic.star, fmt::count(repo.stargazers_count)), Style::new().fg(t.yellow)),
        Span::styled(format!("{} {}  ", ic.fork, fmt::count(repo.forks_count)), Style::new().fg(t.fg)),
        Span::styled(format!("{} {} open  ", ic.issue, repo.open_issues_count), Style::new().fg(t.green)),
        Span::styled(format!("{} {}  ", ic.branch, repo.default_branch), Style::new().fg(t.cyan)),
        Span::styled(format!("pushed {}", fmt::ago_opt(repo.pushed_at)), Style::new().fg(t.dim)),
    ]));
    if !repo.topics.is_empty() {
        head.push(Line::from(
            repo.topics
                .iter()
                .take(8)
                .flat_map(|tp| [Span::styled(format!(" {tp} "), Style::new().fg(t.cyan).bg(t.sel)), Span::raw(" ")])
                .collect::<Vec<_>>(),
        ));
    }
    let head_h = (head.len() as u16 + 1).min(inner.height);

    let detail = d.details.get(&repo.full_name);
    let tiers = d.deploys.get(&repo.full_name);
    let tier_h = tiers.map(|t| t.len() as u16 + 1).unwrap_or(0);
    let areas = Layout::vertical([
        Constraint::Length(head_h),
        Constraint::Length(3),
        Constraint::Length(9),
        Constraint::Length(tier_h),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(Paragraph::new(head).wrap(Wrap { trim: true }), areas[0]);

    // Languages
    let mut lang_lines = vec![section_rule(app, "languages", w)];
    match detail {
        Some(det) if !det.languages.is_empty() => {
            let total: u64 = det.languages.iter().map(|(_, v)| v).sum::<u64>().max(1);
            let mut bar = vec![];
            let mut used = 0usize;
            let n = det.languages.len();
            for (i, (lang, bytes)) in det.languages.iter().enumerate() {
                let cells = if i == n - 1 {
                    (w as usize).saturating_sub(used)
                } else {
                    ((*bytes as f64 / total as f64) * w as f64).round() as usize
                };
                let cells = cells.min((w as usize).saturating_sub(used));
                used += cells;
                bar.push(Span::styled("▆".repeat(cells), Style::new().fg(theme::language_color(lang, t.dim))));
            }
            lang_lines.push(Line::from(bar));
            let legend: Vec<Span> = det
                .languages
                .iter()
                .take(5)
                .flat_map(|(lang, bytes)| {
                    [
                        Span::styled("● ", Style::new().fg(theme::language_color(lang, t.dim))),
                        Span::styled(
                            format!("{lang} {:.1}%  ", *bytes as f64 * 100.0 / total as f64),
                            Style::new().fg(t.dim),
                        ),
                    ]
                })
                .collect();
            f.render_widget(
                Paragraph::new(vec![lang_lines[0].clone(), lang_lines[1].clone(), Line::from(legend)]),
                areas[1],
            );
        }
        Some(_) => f.render_widget(
            Paragraph::new(vec![
                lang_lines.remove(0),
                Line::from(Span::styled("no language data", Style::new().fg(t.dim))),
            ]),
            areas[1],
        ),
        None => f.render_widget(Paragraph::new(vec![lang_lines.remove(0), placeholder_line(app)]), areas[1]),
    }

    // Commit activity (braille, btop-style)
    let chart_area = areas[2];
    let rule = section_rule(app, "commits · 52 weeks", w);
    f.render_widget(Paragraph::new(rule), Rect { height: 1, ..chart_area });
    let graph_area = Rect { y: chart_area.y + 1, height: chart_area.height.saturating_sub(1), ..chart_area };
    match detail.map(|d| &d.weekly) {
        Some(weekly) if !weekly.is_empty() => {
            let pts: Vec<(f64, f64)> = weekly.iter().enumerate().map(|(i, v)| (i as f64, *v as f64)).collect();
            let max = weekly.iter().copied().max().unwrap_or(1).max(1) as f64;
            let total: u32 = weekly.iter().sum();
            let ds = Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(t.green))
                .data(&pts);
            let chart = Chart::new(vec![ds])
                .block(Block::new().title(
                    Line::from(Span::styled(format!("{total} commits"), Style::new().fg(t.dim))).right_aligned(),
                ))
                .x_axis(Axis::default().bounds([0.0, (pts.len().max(2) - 1) as f64]))
                .y_axis(
                    Axis::default()
                        .bounds([0.0, max * 1.1])
                        .labels([
                            Span::styled("0", Style::new().fg(t.dim)),
                            Span::styled(format!("{max:.0}"), Style::new().fg(t.dim)),
                        ])
                        .style(Style::new().fg(t.border)),
                );
            f.render_widget(chart, graph_area);
        }
        Some(_) => f.render_widget(Paragraph::new(Span::styled("no activity", Style::new().fg(t.dim))), graph_area),
        None => f.render_widget(Paragraph::new(placeholder_line(app)), graph_area),
    }

    // Deploy tiers
    if let Some(ts) = tiers {
        let mut lines = vec![section_rule(app, "deployments", w)];
        for tier in ts {
            lines.push(Line::from(
                [
                    vec![Span::styled(format!("{:>6} ", tier.tier), Style::new().fg(t.dim))],
                    tables::tier_cell(app, Some(tier), false).spans,
                ]
                .concat(),
            ));
        }
        f.render_widget(Paragraph::new(lines), areas[3]);
    }

    // Recent runs + releases
    let mut lines = vec![section_rule(app, "recent runs", w)];
    let runs: Vec<&WorkflowRun> = d.runs.get(&repo.full_name).map(|v| v.iter().take(6).collect()).unwrap_or_default();
    if runs.is_empty() {
        lines.push(Line::from(Span::styled("no runs", Style::new().fg(t.dim))));
    }
    for r in runs {
        let (icon, c) = run_icon(app, r.status.as_deref(), r.conclusion.as_deref());
        lines.push(Line::from(vec![
            Span::styled(format!("{icon} "), Style::new().fg(c)),
            Span::styled(fmt::trunc(r.workflow(), 24), Style::new().fg(t.fg)),
            Span::styled(format!("  {}", fmt::trunc(r.branch(), 20)), Style::new().fg(t.cyan)),
            Span::styled(format!("  {}", fmt::ago(r.created_at)), Style::new().fg(t.dim)),
        ]));
    }
    if let Some(det) = detail.filter(|d| !d.releases.is_empty()) {
        lines.push(section_rule(app, "releases", w));
        for rel in det.releases.iter().take(4) {
            let (label, c) = if rel.draft {
                ("draft", t.dim)
            } else if rel.prerelease {
                ("pre", t.orange)
            } else {
                ("", t.green)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", ic.tag), Style::new().fg(c)),
                Span::styled(rel.tag_name.clone(), Style::new().fg(t.fg).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {label}"), Style::new().fg(c)),
                Span::styled(
                    format!("  {}", fmt::ago(rel.published_at.unwrap_or(rel.created_at))),
                    Style::new().fg(t.dim),
                ),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), areas[4]);
}

fn placeholder_line(app: &App) -> Line<'static> {
    Line::from(Span::styled(format!("{} loading…", super::spinner(app)), Style::new().fg(app.theme.dim)))
}

fn run_detail(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let ic = &app.icons;
    let Some(r) = app.selected_run() else {
        f.render_widget(panel(app, None, ic.run, "run", false), area);
        return;
    };
    let [top, bottom] = Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(area);
    let (icon, c) = run_icon(app, r.status.as_deref(), r.conclusion.as_deref());
    let state = r.conclusion.clone().or(r.status.clone()).unwrap_or_default().replace('_', " ");
    let block = panel(app, None, ic.run, &format!("{} #{}", r.workflow(), r.run_number), false);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{icon} {state}"), Style::new().fg(c).add_modifier(Modifier::BOLD)),
            Span::styled(
                r.run_attempt.filter(|a| *a > 1).map(|a| format!("  attempt {a}")).unwrap_or_default(),
                Style::new().fg(t.orange),
            ),
        ]),
        Line::from(Span::styled(r.title().to_string(), Style::new().fg(t.fg))),
        kv(app, "repo", r.repo.clone(), t.accent2),
        kv(app, "branch", format!("{} @ {}", r.branch(), r.head_sha.get(..7).unwrap_or("")), t.cyan),
        kv(app, "event", r.event.clone(), t.fg),
        kv(app, "actor", r.actor.as_ref().map(|a| a.login.clone()).unwrap_or_default(), t.magenta),
        kv(
            app,
            "started",
            format!("{} ago · took {}", fmt::ago(r.run_started_at.unwrap_or(r.created_at)), fmt::dur(r.duration())),
            t.fg,
        ),
    ];
    if let Some(p) = &r.path {
        lines.push(kv(app, "file", p.clone(), t.dim));
    }
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), top);

    let jobs_block =
        panel(app, None, ic.run, "jobs", app.jobs_focus).title(right_title(app, "tab: focus · o: open job".into()));
    let loaded = app.d().jobs.contains_key(&r.id);
    tables::render(f, app, bottom, jobs_block, ListId::Jobs, tables::jobs(app), loaded, "no jobs", app.jobs_focus);
}

fn sections(f: &mut Frame, app: &App, area: Rect, kind: SectionKind) {
    let t = &app.theme;
    let ic = &app.icons;
    let (id, sec, num, icon, label) = match kind {
        SectionKind::Pulls => (ListId::Pulls, app.pull_sec, 4, ic.pr, "pull requests"),
        SectionKind::Issues => (ListId::Issues, app.issue_sec, 5, ic.issue, "issues"),
    };
    let secs = match kind {
        SectionKind::Pulls => &app.d().pulls,
        SectionKind::Issues => &app.d().issues,
    };
    let [tabs, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let mut spans = vec![Span::raw(" ")];
    for (i, title) in app.section_titles(kind).iter().enumerate() {
        let total = secs.get(i).map(|s| s.total.to_string()).unwrap_or_else(|| "…".into());
        let active = i == sec;
        let st = if active {
            Style::new().fg(t.bg).bg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(t.dim).bg(t.bg_alt)
        };
        spans.push(Span::styled(format!(" {title} "), st));
        spans.push(Span::styled(format!("{total} "), if active { st } else { Style::new().fg(t.orange).bg(t.bg_alt) }));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(" h/l or tab to switch", Style::new().fg(t.dim)));
    f.render_widget(Paragraph::new(Line::from(spans)), tabs);

    let wide = body.width >= 100;
    let [l, r] = if wide {
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(body)
    } else {
        [body, Rect::default()]
    };
    let block = panel(app, Some(num), icon, label, true);
    let loaded = secs.get(sec).is_some_and(|s| s.loaded);
    tables::render(f, app, l, block, id, tables::issues(app, kind, !wide), loaded, "all clear ✨", true);
    if wide {
        match app.selected_issue(kind) {
            Some(i) => issue_preview(f, app, r, i),
            None => f.render_widget(panel(app, None, icon, "preview", false), r),
        }
    }
}

fn issue_preview(f: &mut Frame, app: &App, area: Rect, i: &Issue) {
    let t = &app.theme;
    let ic = &app.icons;
    let block =
        panel(app, None, if i.is_pr() { ic.pr } else { ic.issue }, &format!("{}#{}", i.repo(), i.number), false);
    let mut lines = vec![
        Line::from(Span::styled(i.title.clone(), Style::new().fg(t.fg).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(format!("{} {}", ic.user, i.author()), Style::new().fg(t.cyan)),
            Span::styled(
                format!("  opened {} ago · updated {} ago", fmt::ago(i.created_at), fmt::ago(i.updated_at)),
                Style::new().fg(t.dim),
            ),
            Span::styled(format!("  {} {}", ic.comment, i.comments), Style::new().fg(t.dim)),
            Span::styled(if i.draft == Some(true) { "  draft" } else { "" }, Style::new().fg(t.orange)),
        ]),
    ];
    if !i.labels.is_empty() {
        let spans: Vec<Span> = i
            .labels
            .iter()
            .flat_map(|l| {
                let bg = parse_hex(&l.color).unwrap_or(t.sel);
                let fg = match bg {
                    Color::Rgb(r, g, b) if (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000 > 140 => {
                        Color::Black
                    }
                    _ => Color::White,
                };
                [Span::styled(format!(" {} ", l.name), Style::new().fg(fg).bg(bg)), Span::raw(" ")]
            })
            .collect();
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    let body = i.body.clone().unwrap_or_default().replace('\r', "");
    let body = if body.trim().is_empty() { "no description".to_string() } else { body };
    let mut text = Text::from(lines);
    for l in body.lines().take(200) {
        let style = if l.starts_with('#') {
            Style::new().fg(t.accent).add_modifier(Modifier::BOLD)
        } else if l.trim_start().starts_with("- [x]") {
            Style::new().fg(t.green)
        } else if l.trim_start().starts_with("```") {
            Style::new().fg(t.dim)
        } else {
            Style::new().fg(t.fg)
        };
        text.lines.push(Line::from(Span::styled(l.to_string(), style)));
    }
    f.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: false }), area);
}

fn deploy_detail(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let ic = &app.icons;
    let Some((name, tiers)) = app.selected_deploy() else {
        f.render_widget(panel(app, None, ic.deploy, "details", false), area);
        return;
    };
    let block = panel(app, None, ic.deploy, name, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = vec![];
    for tier in tiers.iter() {
        lines.push(tier_header(app, tier, inner.width));
        if tier.entries.is_empty() {
            lines.push(Line::from(Span::styled("   no matching deployment", Style::new().fg(t.dim))));
        }
        let mut entries: Vec<_> = tier.entries.iter().collect();
        entries.sort_by_key(|x| std::cmp::Reverse(x.at));
        for e in entries {
            let (si, sc) = dstate_icon(app, e.state);
            let (src, what) = match e.source {
                Source::Environment => (ic.env, "environment"),
                Source::Workflow => (ic.run, "workflow"),
                Source::Release => (ic.tag, "release"),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {si} "), Style::new().fg(sc)),
                Span::styled(format!("{src} "), Style::new().fg(t.accent2)),
                Span::styled(fmt::trunc(&e.label, 28), Style::new().fg(t.fg).add_modifier(Modifier::BOLD)),
                Span::styled(format!("  {what}"), Style::new().fg(t.dim)),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("      {} {}", ic.branch, e.git_ref), Style::new().fg(t.cyan)),
                Span::styled(format!("  {} ago", fmt::ago(e.at)), Style::new().fg(t.dim)),
                Span::styled(
                    e.actor.as_ref().map(|a| format!("  {} {a}", ic.user)).unwrap_or_default(),
                    Style::new().fg(t.magenta),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }
    let hint = Line::from(Span::styled("newest source wins · o opens it", Style::new().fg(t.dim)));
    lines.push(hint);
    f.render_widget(Paragraph::new(lines), inner);
}

fn tier_header(app: &App, tier: &TierStatus, width: u16) -> Line<'static> {
    section_rule(app, &tier.tier.to_uppercase(), width)
}
