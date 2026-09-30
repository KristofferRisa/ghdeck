mod app;
mod config;
mod deploy;
mod fetch;
mod fuzzy;
mod github;
mod icons;
mod local;
mod logging;
mod metrics;
mod theme;
mod ui;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{Event, EventStream};
use futures::StreamExt;

use app::App;

#[derive(Parser)]
#[command(name = "ghdeck", version, about = "A btop-style GitHub dashboard for the terminal")]
struct Cli {
    /// Config file (default: ~/.config/ghdeck/config.toml)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// Host to open, by name or hostname
    #[arg(short = 'H', long)]
    host: Option<String>,
    #[command(subcommand)]
    cmd: Option<Sub>,
}

#[derive(Subcommand)]
enum Sub {
    /// Check tokens, endpoints and rate limits for every configured host
    Doctor,
    /// Print the config file path
    ConfigPath,
    /// Run the refresh loop headless for a while and report API, CPU and memory cost
    Bench {
        /// How long to run, in seconds
        #[arg(long, default_value_t = 120)]
        secs: u64,
        /// Also render every frame to an off-screen 200×50 terminal, like the TUI would
        #[arg(long)]
        render: bool,
        /// Poll at full speed even when the rate limit is low (measures the worst case)
        #[arg(long)]
        no_throttle: bool,
    },
    /// Print the log file path
    LogPath,
    /// Write the default config (use --force to overwrite)
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    let path = cli.config.clone().unwrap_or_else(config::config_path);
    match cli.cmd {
        Some(Sub::ConfigPath) => {
            println!("{}", path.display());
            return Ok(());
        }
        Some(Sub::Init { force }) => {
            if config::ensure(&path, force)? {
                println!("wrote {}", path.display());
            } else {
                println!("{} already exists (use --force to overwrite)", path.display());
            }
            return Ok(());
        }
        Some(Sub::Doctor) => return doctor(&path).await,
        Some(Sub::LogPath) => {
            println!("{}", logging::path().display());
            return Ok(());
        }
        Some(Sub::Bench { secs, render, no_throttle }) => {
            return bench(&path, cli.host, secs, render, no_throttle).await;
        }
        None => {}
    }

    let cfg = config::load(&path)?;
    logging::init(&cfg.general.log_level);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"), pid = std::process::id(), config = %path.display(),
        refresh = cfg.general.refresh_secs, fast = cfg.general.fast_refresh_secs, slow = cfg.general.slow_refresh_secs,
        "start"
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(cfg, path, cli.host, tx)?;

    let mut terminal = ratatui::init();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    // Redraw only when input, data or an animation changed something, and once a second for
    // the clock. An idle dashboard costs one frame per second instead of four.
    let mut dirty = true;
    let mut drawn_sec = 0;
    let res: Result<()> = async {
        loop {
            let sec = chrono::Local::now().timestamp();
            if dirty || sec != drawn_sec {
                let t0 = Instant::now();
                terminal.draw(|f| ui::draw(f, &app))?;
                metrics::record_draw(t0.elapsed());
                dirty = false;
                drawn_sec = sec;
            }
            tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(Event::Key(k))) => {
                        app.on_key(k);
                        dirty = true;
                    }
                    Some(Ok(_)) => dirty = true,
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                Some(env) = rx.recv() => {
                    app.on_msg(env);
                    while let Ok(env) = rx.try_recv() {
                        app.on_msg(env);
                    }
                    dirty = true;
                }
                _ = tick.tick() => dirty |= app.on_tick() | app.animating(),
            }
            if app.quit {
                break;
            }
            if app.suspend {
                app.suspend = false;
                ratatui::restore();
                let editor = app
                    .cfg
                    .general
                    .editor
                    .clone()
                    .or_else(|| std::env::var("VISUAL").ok())
                    .or_else(|| std::env::var("EDITOR").ok())
                    .unwrap_or_else(|| "nvim".into());
                let mut parts = editor.split_whitespace();
                let prog = parts.next().unwrap_or("nvim").to_string();
                let status = std::process::Command::new(&prog).args(parts).arg(&app.cfg_path).status();
                terminal = ratatui::init();
                terminal.clear()?;
                events = EventStream::new();
                match status {
                    Ok(_) => app.reload_config(),
                    Err(e) => app.notify(&format!("could not start {prog}: {e}")),
                }
            }
        }
        Ok(())
    }
    .await;
    ratatui::restore();
    let (total, _) = app.usage_rates();
    tracing::info!("stop {total}");
    res
}

/// Drives the same scheduler as the TUI without a terminal and prints what it cost.
async fn bench(path: &PathBuf, host: Option<String>, secs: u64, render: bool, no_throttle: bool) -> Result<()> {
    let cfg = config::load(path).context("load config")?;
    let log = logging::init(&cfg.general.log_level);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(cfg, path.clone(), host, tx)?;
    app.no_throttle = no_throttle;
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 50))?;
    tracing::info!(secs, render, host = %app.host().name, "bench start");
    eprintln!("bench: {} for {secs}s{}", app.host().host, if render { " with rendering" } else { "" });

    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let end = tokio::time::Instant::now() + Duration::from_secs(secs);
    let (mut dirty, mut drawn_sec) = (true, 0);
    loop {
        let sec = chrono::Local::now().timestamp();
        if render && (dirty || sec != drawn_sec) {
            let t0 = Instant::now();
            term.draw(|f| ui::draw(f, &app))?;
            metrics::record_draw(t0.elapsed());
            (dirty, drawn_sec) = (false, sec);
        }
        tokio::select! {
            Some(env) = rx.recv() => {
                app.on_msg(env);
                dirty = true;
            }
            _ = tick.tick() => dirty |= app.on_tick() | app.animating(),
            _ = tokio::time::sleep_until(end) => break,
        }
    }

    let (total, _) = app.usage_rates();
    tracing::info!("bench done {total}");
    let d = app.d();
    let core = app.client().rates().core;
    println!(
        "\nran {:.0}s against {}  ·  {} repos watched, {} with active runs",
        total.secs,
        app.host().host,
        d.watched.len(),
        app.active_repos().len()
    );
    println!("\n  startup     {:>8} core calls (first full load, left out of the rates below)", app.usage_start.core);
    println!(
        "  core API    {:>8.0} calls/h   ({} calls, {:.0}% free 304s)",
        total.core_per_hour, total.calls, total.free_pct
    );
    println!("  search      {:>8.1} /min", total.search_per_min);
    println!("  graphql     {:>8.0} /h", total.graphql_per_hour);
    println!("  cpu         {:>8.2} %", total.cpu_pct);
    println!("  memory      {:>8.1} MB", total.rss as f64 / 1_048_576.0);
    if render {
        println!("  redraws     {:>8.2} /s  ({:.2} ms each)", total.draws_per_sec, total.ms_per_draw);
    }
    if let Some(c) = core {
        println!("  remaining   {:>8} / {} core (shared with every tool using your token)", c.remaining, c.limit);
    }
    println!("\n  {:<46}{:>7}{:>7}{:>7}{:>8}", "request (incl. startup)", "calls", "304", "cost", "avg ms");
    let mut snap = app.client().metrics.snapshot();
    snap.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.cost()));
    for (_, kind, s) in snap {
        println!("  {:<46}{:>7}{:>7}{:>7}{:>8}", kind, s.calls, s.not_modified, s.cost(), s.millis / s.calls.max(1));
    }
    if !app.errors.is_empty() {
        println!("\n  {} errors, first: {}", app.errors.len(), app.errors[0].1);
    }
    if let Some(p) = log {
        println!("\nlog: {}", p.display());
    }
    Ok(())
}

async fn doctor(path: &PathBuf) -> Result<()> {
    let cfg = config::load(path).context("load config")?;
    println!("config  {}", path.display());
    println!("tiers   {}", cfg.deploy.tiers.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "));
    println!("log     {}", logging::path().display());
    for h in &cfg.hosts {
        let c = github::Client::new(h)?;
        println!("\n[{}] {}", h.name, h.host);
        println!("  api      {}", c.api);
        println!("  graphql  {}", c.graphql);
        if !h.local_dirs.is_empty() {
            let found = local::discover(&h.local_dirs, &h.host);
            println!("  local    {} repos cloned in {}", found.len(), h.local_dirs.join(", "));
        }
        match c.viewer().await {
            Ok(u) => println!("  viewer   ✓ {}", u.login),
            Err(e) => {
                println!("  viewer   ✗ {e:#}");
                continue;
            }
        }
        match c.refresh_rates().await {
            Ok(()) => {
                let r = c.rates();
                if let Some(core) = r.core {
                    println!("  core     {}/{}", core.remaining, core.limit);
                }
                if let Some(s) = r.search {
                    println!("  search   {}/{}", s.remaining, s.limit);
                }
            }
            Err(e) => println!("  rate     ✗ {e:#}"),
        }
        match c.user_repos(5).await {
            Ok(v) => println!("  repos    ✓ {}", v.iter().map(|r| r.full_name.as_str()).collect::<Vec<_>>().join(", ")),
            Err(e) => println!("  repos    ✗ {e:#}"),
        }
        match c.contributions().await {
            Ok(cal) => println!("  graphql  ✓ {} contributions last year", cal.total),
            Err(e) => println!("  graphql  ✗ {e:#}"),
        }
    }
    Ok(())
}
