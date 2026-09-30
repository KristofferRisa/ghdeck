use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use super::{fmt, run_icon};
use crate::{
    app::{App, NodeKind, Overlay, PickAction, Picker, PickerKind, leader_tree},
    fuzzy::{self, highlight},
};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    match &app.overlay {
        Some(Overlay::Picker(p)) => picker(f, app, area, p),
        Some(Overlay::Leader(path)) => which_key(f, app, area, path),
        Some(Overlay::Help(s)) => help(f, app, area, *s),
        Some(Overlay::Errors(s)) => errors(f, app, area, *s),
        Some(Overlay::Stats(s)) => stats(f, app, area, *s),
        Some(Overlay::Confirm(msg, ..)) => confirm(f, app, area, msg),
        None => {}
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn frame<'a>(app: &App, title: &str, color: ratatui::style::Color) -> Block<'a> {
    let t = &app.theme;
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .style(Style::new().bg(t.bg_alt).fg(t.fg))
        .title(Line::from(Span::styled(format!(" {title} "), Style::new().fg(color).add_modifier(Modifier::BOLD))))
}

// ── Picker (telescope / snacks.picker style) ────────────────────────────────

fn picker(f: &mut Frame, app: &App, area: Rect, p: &Picker) {
    let t = &app.theme;
    let ic = &app.icons;
    let w = (area.width as u32 * 85 / 100) as u16;
    let h = (area.height as u32 * 80 / 100) as u16;
    let r = centered(area, w.max(50), h.max(12));
    f.render_widget(Clear, r);

    let has_preview = matches!(p.kind, PickerKind::Repos | PickerKind::Runs | PickerKind::Issues) && r.width >= 90;
    let [left, right] = if has_preview {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(r)
    } else {
        [r, Rect::default()]
    };
    let [input, list] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(left);

    let items = app.picker_items(p.kind);
    let total = items.len();
    let hits = fuzzy::filter(items, |i| i.text.clone(), &p.query);

    let input_block = frame(app, p.kind.title(), t.accent)
        .title(Line::from(Span::styled(format!(" {}/{} ", hits.len(), total), Style::new().fg(t.dim))).right_aligned());
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} ", ic.search), Style::new().fg(t.orange)),
            Span::styled(p.query.clone(), Style::new().fg(t.fg)),
            Span::styled("▏", Style::new().fg(t.accent)),
        ]))
        .block(input_block),
        input,
    );

    let hl = Style::new().fg(t.orange).add_modifier(Modifier::BOLD);
    let rows: Vec<Row> = hits
        .iter()
        .map(|h| {
            let it = &h.item;
            Row::new(vec![
                Cell::from(Span::styled(it.icon.clone(), Style::new().fg(it.color))),
                Cell::from(Line::from(highlight(&it.text, &h.idx, |_| Style::new().fg(t.fg), hl))),
                Cell::from(Line::from(Span::styled(it.hint.clone(), Style::new().fg(t.dim))).right_aligned()),
            ])
        })
        .collect();
    let list_block = frame(app, "Results", t.border).title_bottom(
        Line::from(Span::styled(" ⏎ select · C-j/C-k move · C-o open · C-u clear · esc ", Style::new().fg(t.dim)))
            .right_aligned(),
    );
    let table = Table::new(rows, [Constraint::Length(2), Constraint::Fill(1), Constraint::Length(8)])
        .block(list_block)
        .row_highlight_style(Style::new().bg(t.sel).add_modifier(Modifier::BOLD))
        .highlight_symbol("▌");
    let mut st =
        TableState::default().with_selected((!hits.is_empty()).then_some(p.sel.min(hits.len().saturating_sub(1))));
    f.render_stateful_widget(table, list, &mut st);

    if has_preview {
        let block = frame(app, "Preview", t.border);
        let lines = hits.get(p.sel).map(|h| preview(app, &h.item.action)).unwrap_or_default();
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), right);
    }
}

fn preview(app: &App, action: &PickAction) -> Vec<Line<'static>> {
    let t = &app.theme;
    let ic = &app.icons;
    let d = app.d();
    let dim = Style::new().fg(t.dim);
    match action {
        PickAction::Repo(name) => {
            let Some(r) = app.find_repo(name) else { return vec![] };
            let mut v = vec![
                Line::from(Span::styled(r.full_name.clone(), Style::new().fg(t.accent).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(r.description.clone().unwrap_or_default(), Style::new().fg(t.fg))),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        format!("{} {}  ", ic.star, fmt::count(r.stargazers_count)),
                        Style::new().fg(t.yellow),
                    ),
                    Span::styled(format!("{} {}  ", ic.fork, fmt::count(r.forks_count)), dim),
                    Span::styled(format!("{} {}", ic.issue, r.open_issues_count), Style::new().fg(t.green)),
                ]),
                Line::from(vec![
                    Span::styled(format!("{} {}  ", ic.branch, r.default_branch), Style::new().fg(t.cyan)),
                    Span::styled(r.language.clone().unwrap_or_default(), dim),
                ]),
                Line::from(Span::styled(format!("pushed {} ago", fmt::ago_opt(r.pushed_at)), dim)),
                Line::from(Span::styled(
                    r.visibility.clone().unwrap_or_else(|| if r.private { "private".into() } else { "public".into() }),
                    Style::new().fg(t.orange),
                )),
            ];
            if let Some(runs) = d.runs.get(name) {
                v.push(Line::from(""));
                v.push(Line::from(Span::styled(
                    "recent runs",
                    Style::new().fg(t.accent2).add_modifier(Modifier::BOLD),
                )));
                for run in runs.iter().take(8) {
                    let (i, c) = run_icon(app, run.status.as_deref(), run.conclusion.as_deref());
                    v.push(Line::from(vec![
                        Span::styled(format!("{i} "), Style::new().fg(c)),
                        Span::styled(run.workflow().to_string(), Style::new().fg(t.fg)),
                        Span::styled(format!("  {}  {}", run.branch(), fmt::ago(run.created_at)), dim),
                    ]));
                }
            }
            v
        }
        PickAction::Run(repo, id) => {
            let Some(r) = d.runs.get(repo).and_then(|v| v.iter().find(|r| r.id == *id)) else { return vec![] };
            let (i, c) = run_icon(app, r.status.as_deref(), r.conclusion.as_deref());
            vec![
                Line::from(vec![
                    Span::styled(format!("{i} "), Style::new().fg(c)),
                    Span::styled(
                        format!("{} #{}", r.workflow(), r.run_number),
                        Style::new().fg(t.fg).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(r.title().to_string(), Style::new().fg(t.fg))),
                Line::from(""),
                Line::from(Span::styled(format!("{}  {} {}", r.repo, ic.branch, r.branch()), Style::new().fg(t.cyan))),
                Line::from(Span::styled(
                    format!("{} · {} ago · {}", r.event, fmt::ago(r.created_at), fmt::dur(r.duration())),
                    dim,
                )),
            ]
        }
        PickAction::Url(url) => {
            let Some(i) = d.pulls.iter().chain(d.issues.iter()).flat_map(|s| &s.items).find(|i| i.html_url == *url)
            else {
                return vec![];
            };
            let mut v = vec![
                Line::from(Span::styled(i.title.clone(), Style::new().fg(t.fg).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(
                    format!("{}#{} · {} · {} ago", i.repo(), i.number, i.author(), fmt::ago(i.updated_at)),
                    dim,
                )),
                Line::from(""),
            ];
            for l in i.body.clone().unwrap_or_default().replace('\r', "").lines().take(60) {
                v.push(Line::from(Span::styled(l.to_string(), Style::new().fg(t.fg))));
            }
            v
        }
        _ => vec![],
    }
}

// ── which-key ───────────────────────────────────────────────────────────────

fn which_key(f: &mut Frame, app: &App, area: Rect, path: &[char]) {
    let t = &app.theme;
    let tree = leader_tree();
    let mut nodes = &tree;
    for p in path {
        if let Some(NodeKind::Group(ch)) = nodes.iter().find(|n| n.key == *p).map(|n| &n.kind) {
            nodes = ch;
        }
    }
    let lines: Vec<Line> = nodes
        .iter()
        .map(|n| {
            let key = if n.key == ' ' { "␣".to_string() } else { n.key.to_string() };
            let (desc, c) = match n.kind {
                NodeKind::Group(_) => (format!("+{}", n.desc), t.accent2),
                NodeKind::Cmd(_) => (n.desc.to_string(), t.fg),
            };
            Line::from(vec![
                Span::styled(format!(" {key:>2} "), Style::new().fg(t.orange).add_modifier(Modifier::BOLD)),
                Span::styled("➜ ", Style::new().fg(t.dim)),
                Span::styled(desc, Style::new().fg(c)),
            ])
        })
        .collect();
    let w = lines.iter().map(|l| l.width()).max().unwrap_or(10) as u16 + 4;
    let h = lines.len() as u16 + 2;
    let r = Rect {
        x: area.right().saturating_sub(w + 1),
        y: area.bottom().saturating_sub(h + 1),
        width: w.min(area.width),
        height: h.min(area.height),
    };
    f.render_widget(Clear, r);
    let title = format!("␣{}", path.iter().collect::<String>());
    let block = frame(app, &title, t.yellow)
        .title_bottom(Line::from(Span::styled(" esc close · ⌫ back ", Style::new().fg(t.dim))).right_aligned());
    f.render_widget(Paragraph::new(lines).block(block), r);
}

// ── Help ────────────────────────────────────────────────────────────────────

const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "navigation",
        &[
            ("1-8", "switch view (dashboard, repos, actions, pulls, issues, deploys, explore, inbox)"),
            ("j/k ↑/↓", "move"),
            ("g/G", "top / bottom"),
            ("C-d/C-u", "half page down / up"),
            ("tab/S-tab", "next panel / pane / section"),
            ("C-h/j/k/l", "focus panel left/down/up/right (dashboard)"),
            ("h/l [ ]", "previous / next section (pulls, issues, explore)"),
            ("z", "zoom the focused panel"),
            ("esc", "clear filter · unzoom · back to dashboard"),
        ],
    ),
    (
        "find",
        &[
            ("/", "fuzzy filter the current list (fzf syntax: 'exact ^prefix suffix$ !not)"),
            ("C-p ␣␣", "find a repository"),
            (":", "command palette"),
            ("␣", "leader menu (which-key)"),
        ],
    ),
    (
        "actions",
        &[
            ("⏎", "drill in (repo → runs → jobs) or open"),
            ("o", "open in browser"),
            ("y", "copy URL to the clipboard (OSC 52)"),
            ("r", "refresh now"),
            ("R", "re-run failed jobs of the selected run"),
            ("x", "cancel the selected run"),
            ("w", "watch / unwatch the selected repo"),
            ("a", "repos: toggle watched / all accessible"),
            ("H", "next host"),
            ("t", "next theme"),
            ("E", "error log"),
            ("S", "API & resource usage (what ghdeck costs)"),
            ("q", "quit"),
        ],
    ),
];

fn help(f: &mut Frame, app: &App, area: Rect, scroll: u16) {
    let t = &app.theme;
    let r = centered(area, 90, area.height.saturating_sub(4));
    f.render_widget(Clear, r);
    let mut lines = vec![];
    for (section, keys) in HELP {
        lines.push(Line::from(Span::styled(
            section.to_uppercase(),
            Style::new().fg(t.accent2).add_modifier(Modifier::BOLD),
        )));
        for (k, d) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {k:<12}"), Style::new().fg(t.orange).add_modifier(Modifier::BOLD)),
                Span::styled(d.to_string(), Style::new().fg(t.fg)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(format!("config: {}", app.cfg_path.display()), Style::new().fg(t.dim))));
    let block = frame(app, "ghdeck · help", t.cyan)
        .title_bottom(Line::from(Span::styled(" j/k scroll · esc close ", Style::new().fg(t.dim))).right_aligned());
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), r);
}

fn errors(f: &mut Frame, app: &App, area: Rect, scroll: u16) {
    let t = &app.theme;
    let r = centered(area, area.width.saturating_sub(10).max(40), area.height.saturating_sub(6));
    f.render_widget(Clear, r);
    let mut lines: Vec<Line> = app
        .errors
        .iter()
        .rev()
        .map(|(ts, e)| {
            Line::from(vec![
                Span::styled(format!("{} ", ts.format("%H:%M:%S")), Style::new().fg(t.dim)),
                Span::styled(e.clone(), Style::new().fg(t.red)),
            ])
        })
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("no errors 🎉", Style::new().fg(t.green))));
    }
    let block = frame(app, &format!("errors ({})", app.errors.len()), t.red);
    f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).scroll((scroll, 0)), r);
}

fn stats(f: &mut Frame, app: &App, area: Rect, scroll: u16) {
    let t = &app.theme;
    let r = centered(area, 100, area.height.saturating_sub(4));
    f.render_widget(Clear, r);
    let (total, recent) = app.usage_rates();
    let head =
        |s: &str| Line::from(Span::styled(s.to_string(), Style::new().fg(t.accent2).add_modifier(Modifier::BOLD)));
    let dim = Style::new().fg(t.dim);
    let row = |k: &str, a: String, b: String| {
        Line::from(vec![
            Span::styled(format!("  {k:<14}"), Style::new().fg(t.orange)),
            Span::styled(format!("{a:<22}"), Style::new().fg(t.fg)),
            Span::styled(b, Style::new().fg(t.fg)),
        ])
    };
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_048_576.0);
    let mut lines = vec![
        head("USAGE"),
        Line::from(vec![
            Span::styled(format!("  {:<14}", ""), dim),
            Span::styled(format!("{:<22}", format!("since load ({})", fmt::secs(total.secs as i64))), dim),
            Span::styled(format!("last {}", fmt::secs(recent.secs as i64)), dim),
        ]),
        row("core api", format!("{:.0} calls/h", total.core_per_hour), format!("{:.0} calls/h", recent.core_per_hour)),
        row("search", format!("{:.1}/min", total.search_per_min), format!("{:.1}/min", recent.search_per_min)),
        row("graphql", format!("{:.0}/h", total.graphql_per_hour), format!("{:.0}/h", recent.graphql_per_hour)),
        row(
            "free (304)",
            format!("{:.0}% of {}", total.free_pct, total.calls),
            format!("{:.0}% of {}", recent.free_pct, recent.calls),
        ),
        row("cpu", format!("{:.2}%", total.cpu_pct), format!("{:.2}%", recent.cpu_pct)),
        row("memory", mb(total.rss), mb(recent.rss)),
        row(
            "redraws",
            format!("{:.2}/s · {:.2} ms", total.draws_per_sec, total.ms_per_draw),
            format!("{:.2}/s · {:.2} ms", recent.draws_per_sec, recent.ms_per_draw),
        ),
        Line::from(Span::styled(
            if !app.usage_warm {
                "  measuring starts after the first refresh (the startup load is not counted)"
            } else if recent.secs < 120.0 {
                "  per-hour numbers settle after a few minutes"
            } else {
                "  the startup load is not counted; ↯ in the status line is the last-5-min core rate"
            },
            dim,
        )),
        Line::from(""),
    ];
    let g = &app.cfg.general;
    let rates = app.client().rates();
    let quota = |r: Option<crate::github::models::Rate>| {
        r.map(|r| format!("{}/{}", fmt::count(r.remaining), fmt::count(r.limit))).unwrap_or_else(|| "?".into())
    };
    lines.push(head("LIMITS & POLLING"));
    lines.push(row("remaining", format!("core {}", quota(rates.core)), format!("search {}", quota(rates.search))));
    lines.push(row(
        "intervals",
        format!("{}s / {}s / {}s", g.refresh_secs, g.fast_refresh_secs, g.slow_refresh_secs),
        format!("throttle ×{}  (refresh / fast / slow)", app.throttle()),
    ));
    lines.push(row(
        "repos",
        format!("{} watched", app.d().watched.len()),
        format!("{} with active runs", app.active_repos().len()),
    ));
    lines.push(Line::from(""));
    lines.push(head("ENDPOINTS"));
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<44}{:>7}{:>7}{:>7}{:>8}{:>9}",
            "request (all, incl. startup)", "calls", "304", "cost", "avg ms", "KB"
        ),
        dim,
    )));
    let mut snap: Vec<_> = app.clients.iter().flat_map(|c| c.metrics.snapshot()).collect();
    snap.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.cost()));
    for (_, kind, s) in snap {
        lines.push(Line::from(Span::styled(
            format!(
                "  {:<44}{:>7}{:>7}{:>7}{:>8}{:>9}",
                fmt::trunc(&kind, 43),
                s.calls,
                s.not_modified,
                s.cost(),
                s.millis / s.calls.max(1),
                s.bytes / 1024
            ),
            Style::new().fg(t.fg),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("log: {}", crate::logging::path().display()), dim)));
    let block = frame(app, "ghdeck · usage", t.cyan)
        .title_bottom(Line::from(Span::styled(" j/k scroll · esc close ", dim)).right_aligned());
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), r);
}

fn confirm(f: &mut Frame, app: &App, area: Rect, msg: &str) {
    let t = &app.theme;
    let w = (msg.chars().count() as u16 + 6).clamp(30, area.width);
    let r = centered(area, w, 6);
    f.render_widget(Clear, r);
    let lines = vec![
        Line::from(Span::styled(msg.to_string(), Style::new().fg(t.fg).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::new().fg(t.bg).bg(t.green).add_modifier(Modifier::BOLD)),
            Span::styled(" yes    ", Style::new().fg(t.fg)),
            Span::styled(" n ", Style::new().fg(t.bg).bg(t.red).add_modifier(Modifier::BOLD)),
            Span::styled(" no", Style::new().fg(t.fg)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).centered().block(frame(app, "confirm", t.red)).wrap(Wrap { trim: true }), r);
}
