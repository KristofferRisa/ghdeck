mod dash;
pub mod fmt;
mod overlay;
mod tables;
mod views;

use std::cell::RefMut;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, TableState},
};

use crate::{
    app::{App, ListId, View},
    deploy::DState,
    icons::{SPINNER, SUP},
};

pub fn draw(f: &mut Frame, app: &App) {
    let t = &app.theme;
    let area = f.area();
    let base = if app.cfg.general.transparent { Style::new().fg(t.fg) } else { Style::new().bg(t.bg).fg(t.fg) };
    f.render_widget(Block::new().style(base), area);
    if area.width < 40 || area.height < 10 {
        f.render_widget(Paragraph::new("ghdeck needs at least 40×10"), area);
        return;
    }
    let [header, body, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(area);
    app.rects.borrow_mut().clear();
    draw_header(f, app, header);
    match app.view {
        View::Dashboard => dash::draw(f, app, body),
        v => views::draw(f, app, body, v),
    }
    draw_status(f, app, status);
    overlay::draw(f, app, area);
}

// ── Shared helpers ─────────────────────────────────────────────────────────

pub fn sup(n: usize) -> String {
    n.to_string().chars().map(|c| SUP[c.to_digit(10).unwrap_or(0) as usize]).collect()
}

pub fn spinner(app: &App) -> &'static str {
    SPINNER[(app.tick as usize) % SPINNER.len()]
}

/// btop-style box: rounded border, superscript hotkey + title on the top edge.
pub fn panel<'a>(app: &App, num: Option<usize>, icon: &str, title: &str, focused: bool) -> Block<'a> {
    let t = &app.theme;
    let border = if focused { t.accent } else { t.border };
    let mut spans = vec![Span::styled("─", Style::new().fg(border))];
    if let Some(n) = num {
        spans.push(Span::styled(sup(n), Style::new().fg(if focused { t.orange } else { t.dim })));
    }
    spans.push(Span::styled(
        format!("{icon} {title} "),
        Style::new().fg(if focused { t.fg } else { t.dim }).add_modifier(Modifier::BOLD),
    ));
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(border)).title(Line::from(spans))
}

pub fn right_title<'a>(app: &App, text: String) -> Line<'a> {
    Line::from(Span::styled(format!(" {text} "), Style::new().fg(app.theme.dim))).right_aligned()
}

pub fn highlight_style(app: &App, focused: bool) -> Style {
    if focused { Style::new().bg(app.theme.sel).add_modifier(Modifier::BOLD) } else { Style::new() }
}

/// Returns the (clamped) table state for a list.
pub fn list_state<'a>(app: &'a App, id: ListId, len: usize) -> RefMut<'a, TableState> {
    let states = app.states.borrow_mut();
    let mut st = RefMut::map(states, |s| s.entry(id).or_default());
    if len == 0 {
        st.select(None);
    } else {
        let i = st.selected().unwrap_or(0).min(len - 1);
        st.select(Some(i));
    }
    st
}

pub fn run_icon(app: &App, status: Option<&str>, conclusion: Option<&str>) -> (&'static str, Color) {
    let t = &app.theme;
    match (status, conclusion) {
        (Some("completed"), Some("success")) => ("✓", t.green),
        (Some("completed"), Some("failure" | "startup_failure")) => ("✗", t.red),
        (Some("completed"), Some("timed_out")) => ("⧗", t.red),
        (Some("completed"), Some("cancelled")) => ("⊘", t.dim),
        (Some("completed"), Some("skipped")) => ("↷", t.dim),
        (Some("completed"), Some("action_required")) => ("!", t.orange),
        (Some("completed"), _) => ("●", t.dim),
        (Some("in_progress"), _) => (spinner(app), t.yellow),
        (Some(_), _) => ("◌", t.yellow),
        _ => ("·", t.dim),
    }
}

pub fn dstate_icon(app: &App, s: DState) -> (&'static str, Color) {
    let t = &app.theme;
    match s {
        DState::Success => ("●", t.green),
        DState::Failure => ("✗", t.red),
        DState::Running => (spinner(app), t.yellow),
        DState::Pending => ("◌", t.yellow),
        DState::Inactive => ("○", t.dim),
        DState::Unknown => ("?", t.dim),
    }
}

pub fn placeholder(app: &App, loaded: bool, empty_msg: &str) -> Paragraph<'static> {
    let t = &app.theme;
    let text = if loaded { empty_msg.to_string() } else { format!("{} loading…", spinner(app)) };
    Paragraph::new(Line::from(Span::styled(text, Style::new().fg(t.dim)))).centered()
}

// ── Header (btop-ish top bar) ──────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let ic = &app.icons;
    let d = app.d();
    let bar = Style::new().bg(t.bg_alt);
    f.render_widget(Block::new().style(bar), area);

    let mut left = vec![
        Span::styled(format!(" {} ghdeck ", ic.logo), Style::new().fg(t.bg).bg(t.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {} {}", ic.host, app.host().host), Style::new().fg(t.cyan)),
    ];
    if app.clients.len() > 1 {
        left.push(Span::styled(format!(" {}/{}", app.host_idx + 1, app.clients.len()), Style::new().fg(t.dim)));
    }
    if let Some(v) = &d.viewer {
        left.push(Span::styled(format!("  {} {}", ic.user, v.login), Style::new().fg(t.magenta)));
    }
    left.push(Span::raw(" "));

    let unread = d.notifications.iter().filter(|n| n.unread).count();
    let mut right = vec![];
    if unread > 0 {
        right.push(Span::styled(
            format!("{} {unread} ", ic.bell),
            Style::new().fg(t.orange).add_modifier(Modifier::BOLD),
        ));
    }
    right.push(Span::styled(
        format!("{} {} ", ic.clock, chrono::Local::now().format("%H:%M:%S")),
        Style::new().fg(t.fg),
    ));

    let tabs: Vec<Span> = View::ALL
        .iter()
        .enumerate()
        .flat_map(|(i, v)| {
            let active = *v == app.view;
            let style = if active {
                Style::new().fg(t.accent).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::new().fg(t.dim)
            };
            [Span::styled(sup(i + 1), Style::new().fg(t.orange)), Span::styled(format!("{} ", v.title()), style)]
        })
        .collect();

    let lw = Line::from(left.clone()).width() as u16;
    let rw = Line::from(right.clone()).width() as u16;
    let tw = Line::from(tabs.clone()).width() as u16;
    let [l, mid, r] =
        Layout::horizontal([Constraint::Length(lw), Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(Line::from(right)), r);
    if mid.width >= tw {
        f.render_widget(Paragraph::new(Line::from(tabs)).centered(), mid);
    } else {
        // Too narrow for all tabs: show only the current one.
        let i = View::ALL.iter().position(|v| *v == app.view).unwrap_or(0);
        let cur = Line::from(vec![
            Span::styled(sup(i + 1), Style::new().fg(t.orange)),
            Span::styled(app.view.title(), Style::new().fg(t.accent).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(cur).centered(), mid);
    }
}

// ── Status line (lualine-ish) ──────────────────────────────────────────────

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let ic = &app.icons;
    let d = app.d();
    f.render_widget(Block::new().style(Style::new().bg(t.bg_alt)), area);
    let (mode, mc) = app.mode();

    let mut left = vec![
        Span::styled(format!(" {mode} "), Style::new().fg(t.bg).bg(mc).add_modifier(Modifier::BOLD)),
        Span::styled(ic.sep_r, Style::new().fg(mc).bg(t.sel)),
        Span::styled(format!(" {} ", app.view.title()), Style::new().fg(t.fg).bg(t.sel)),
        Span::styled(ic.sep_r, Style::new().fg(t.sel).bg(t.bg_alt)),
    ];
    if let Some(id) = app.filtering.or(app.cur_list()) {
        let q = app.filter(id);
        if app.filtering.is_some() || !q.is_empty() {
            left.push(Span::styled(format!(" {} ", ic.search), Style::new().fg(t.green)));
            left.push(Span::styled(q.to_string(), Style::new().fg(t.fg)));
            if app.filtering.is_some() {
                left.push(Span::styled("▏", Style::new().fg(t.green)));
            }
        }
    }
    if let Some((msg, err, _)) = &app.toast {
        left.push(Span::styled(format!("  {msg}"), Style::new().fg(if *err { t.red } else { t.cyan })));
    }

    let mut right = vec![];
    if app.is_busy() {
        right.push(Span::styled(format!("{} syncing ", spinner(app)), Style::new().fg(t.yellow)));
    } else if let Some(ts) = d.last_refresh {
        let secs = (chrono::Local::now() - ts).num_seconds().max(0);
        right.push(Span::styled(format!("⟳ {} ", fmt::secs(secs)), Style::new().fg(t.dim)));
    }
    let throttle = app.throttle();
    if throttle > 1 {
        right.push(Span::styled(format!("throttled ×{throttle} "), Style::new().fg(t.orange)));
    }
    if !app.errors.is_empty() {
        right.push(Span::styled(format!("✗ {} ", app.errors.len()), Style::new().fg(t.red)));
    }
    let rates = app.client().rates();
    if let Some(core) = rates.core {
        let frac = core.remaining as f64 / core.limit.max(1) as f64;
        let c = if frac < 0.1 {
            t.red
        } else if frac < 0.3 {
            t.yellow
        } else {
            t.green
        };
        right.push(Span::styled(ic.sep_l, Style::new().fg(t.sel).bg(t.bg_alt)));
        right.push(Span::styled(
            format!(" api {}/{} ", fmt::count(core.remaining), fmt::count(core.limit)),
            Style::new().fg(c).bg(t.sel),
        ));
        // What ghdeck itself spends, over the recent window.
        let (_, recent) = app.usage_rates();
        if app.usage_warm && recent.secs >= 60.0 {
            right.push(Span::styled(
                format!("↯{}/h ", fmt::count(recent.core_per_hour as u64)),
                Style::new().fg(t.dim).bg(t.sel),
            ));
        }
    }
    right.push(Span::styled(ic.sep_l, Style::new().fg(t.accent2).bg(t.sel)));
    right.push(Span::styled(" ␣ leader  ? help ", Style::new().fg(t.bg).bg(t.accent2).add_modifier(Modifier::BOLD)));

    let rw = Line::from(right.clone()).width() as u16;
    let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(rw)]).areas(area);
    f.render_widget(Paragraph::new(Line::from(left)), l);
    f.render_widget(Paragraph::new(Line::from(right)), r);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    use super::*;
    use crate::{
        app::{Overlay, Picker, PickerKind},
        config::{Config, HostConfig},
        fetch::{Envelope, Msg, SectionKind},
    };

    fn app() -> App {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cfg = Config {
            hosts: vec![HostConfig { name: "t".into(), host: "example.ghe.com".into(), ..Default::default() }],
            deploy: toml::from_str("[[tiers]]\nname='prod'\nenvironments=['prod']\nrelease='latest'").unwrap(),
            sections: toml::from_str("[[pulls]]\ntitle='Mine'\nfilter='x'\n[[issues]]\ntitle='Mine'\nfilter='y'")
                .unwrap(),
            ..Default::default()
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(cfg, "/tmp/ghdeck-test.toml".into(), None, tx).unwrap();
        let repo = |n: &str| {
            serde_json::from_value(json!({
                "full_name": n, "name": n, "html_url": "https://x", "default_branch": "main",
                "pushed_at": "2026-09-01T00:00:00Z", "language": "Rust", "stargazers_count": 3
            }))
            .unwrap()
        };
        let run = |id: u64, status: &str| {
            serde_json::from_value(json!({
                "id": id, "name": "CI – deploy prod", "status": status, "conclusion": "success",
                "head_branch": "main", "event": "push", "run_number": id, "html_url": "https://x",
                "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:05:00Z"
            }))
            .unwrap()
        };
        let issue = serde_json::from_value(json!({
            "number": 7, "title": "Fix the thing with a very long title that must be truncated somewhere",
            "html_url": "https://x", "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:00:00Z",
            "repository_url": "https://api.x/repos/o/a", "labels": [{"name": "bug", "color": "ff0000"}],
            "body": "Body\nwith lines"
        }))
        .unwrap();
        let msgs = vec![
            Msg::Viewer(serde_json::from_value(json!({"login": "me"})).unwrap()),
            Msg::Watched(vec![repo("o/a"), repo("o/b")]),
            Msg::Runs("o/a".into(), vec![run(1, "completed"), run(2, "in_progress")]),
            Msg::Section(SectionKind::Pulls, 0, 1, vec![issue]),
            Msg::Trending(vec![repo("x/trend")]),
            Msg::Error("boom".into()),
            Msg::RefreshDone,
        ];
        for msg in msgs {
            app.on_msg(Envelope { host: "t".into(), msg });
        }
        app
    }

    /// Render cost with a realistic amount of data: `cargo test --release render_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn render_cost() {
        let mut app = app();
        let repos: Vec<crate::github::models::Repo> = (0..23)
            .map(|i| {
                serde_json::from_value(json!({"full_name": format!("org/repo-{i}"), "name": "r", "html_url": "",
                    "default_branch": "main", "pushed_at": "2026-09-01T00:00:00Z"}))
                .unwrap()
            })
            .collect();
        let names: Vec<String> = repos.iter().map(|r| r.full_name.clone()).collect();
        app.on_msg(Envelope { host: "t".into(), msg: Msg::Watched(repos) });
        for (i, n) in names.iter().enumerate() {
            let runs = (0..20)
                .map(|j| {
                    serde_json::from_value(json!({"id": i * 100 + j, "name": "CI", "status": "completed",
                        "conclusion": "success", "head_branch": "main", "event": "push", "run_number": j,
                        "html_url": "", "created_at": "2026-09-01T00:00:00Z", "updated_at": "2026-09-01T00:05:00Z"}))
                    .unwrap()
                })
                .collect();
            app.on_msg(Envelope { host: "t".into(), msg: Msg::Runs(n.clone(), runs) });
        }
        let mut term = Terminal::new(TestBackend::new(200, 50)).unwrap();
        for view in [View::Dashboard, View::Repos, View::Actions, View::Deploys] {
            app.view = view;
            let t0 = std::time::Instant::now();
            for _ in 0..200 {
                term.draw(|f| draw(f, &app)).unwrap();
            }
            println!("{:<10} {:.3} ms/frame", view.title(), t0.elapsed().as_secs_f64() * 1000.0 / 200.0);
        }
    }

    /// Every view and overlay renders at small, medium and large sizes without panicking.
    #[test]
    fn renders_everything_at_every_size() {
        let mut app = app();
        let overlays: Vec<fn() -> Option<Overlay>> = vec![
            || None,
            || Some(Overlay::Leader(vec![])),
            || Some(Overlay::Leader(vec!['g'])),
            || Some(Overlay::Help(0)),
            || Some(Overlay::Errors(0)),
            || Some(Overlay::Stats(0)),
            || Some(Overlay::Picker(Picker { kind: PickerKind::Repos, query: "a".into(), sel: 0 })),
            || Some(Overlay::Picker(Picker { kind: PickerKind::Commands, query: String::new(), sel: 3 })),
        ];
        for (w, h) in [(30, 8), (40, 10), (80, 24), (130, 40), (220, 60)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            for view in View::ALL {
                for ov in &overlays {
                    for zoom in [false, true] {
                        app.view = view;
                        app.zoom = zoom;
                        app.overlay = ov();
                        term.draw(|f| draw(f, &app)).unwrap();
                    }
                }
            }
        }
    }

    /// Only the visible window of the runs list is built; scrolling must keep the selection on screen.
    #[test]
    fn long_run_list_scrolls_to_selection() {
        let mut app = app();
        let runs = (0..60)
            .map(|j| {
                serde_json::from_value(json!({"id": 1000 + j, "name": format!("wf-{j:02}"), "status": "completed",
                    "conclusion": "success", "head_branch": "main", "event": "push", "run_number": j,
                    "html_url": "", "created_at": format!("2026-09-01T00:{:02}:00Z", 59 - j),
                    "updated_at": "2026-09-01T01:00:00Z"}))
                .unwrap()
            })
            .collect();
        app.on_msg(Envelope { host: "t".into(), msg: Msg::Runs("o/b".into(), runs) });
        app.view = View::Actions;
        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();
        let screen = |term: &Terminal<TestBackend>| -> String {
            term.backend().buffer().content().iter().map(|c| c.symbol()).collect()
        };
        for sel in [0, 30, 59, 5] {
            app.states.borrow_mut().entry(ListId::Runs).or_default().select(Some(sel));
            term.draw(|f| draw(f, &app)).unwrap();
            let want = app.selected_run().unwrap().workflow().to_string();
            assert!(screen(&term).contains(&want), "selected {want} not visible");
        }
    }

    #[test]
    fn dashboard_shows_data() {
        let app = app();
        let mut term = Terminal::new(TestBackend::new(160, 40)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        let text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("o/a") || text.contains(" a "), "watched repo missing");
        assert!(text.contains("Fix the thing"), "pull request missing");
        assert!(text.contains("@ me") || text.contains("me"), "viewer missing");
    }
}
