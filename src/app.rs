use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use base64::Engine;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{layout::Rect, style::Color, widgets::TableState};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::{Config, HostConfig, StateFile},
    deploy::TierStatus,
    fetch::{self, Ctx, Envelope, Msg, RunAction, SectionKind},
    fuzzy::{self, Hit},
    github::{Client, models::*},
    icons::{self, Icons},
    metrics::{self, Sample},
    theme::{self, THEMES, Theme},
};

/// How often a usage sample is taken, and how many are kept (a 5 minute window).
const SAMPLE_SECS: u64 = 30;
const SAMPLES: usize = 10;
/// How often the usage summary is written to the log.
const USAGE_LOG_SECS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Repos,
    Actions,
    Pulls,
    Issues,
    Deploys,
    Explore,
    Inbox,
}

impl View {
    pub const ALL: [View; 8] = [
        View::Dashboard,
        View::Repos,
        View::Actions,
        View::Pulls,
        View::Issues,
        View::Deploys,
        View::Explore,
        View::Inbox,
    ];
    pub fn title(self) -> &'static str {
        match self {
            View::Dashboard => "dashboard",
            View::Repos => "repos",
            View::Actions => "actions",
            View::Pulls => "pulls",
            View::Issues => "issues",
            View::Deploys => "deploys",
            View::Explore => "explore",
            View::Inbox => "inbox",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Repos,
    Actions,
    Deploys,
    Pulse,
    Pulls,
    Issues,
    Explore,
}

impl Panel {
    pub const ALL: [Panel; 7] =
        [Panel::Repos, Panel::Actions, Panel::Deploys, Panel::Pulse, Panel::Pulls, Panel::Issues, Panel::Explore];
    pub fn list(self) -> Option<ListId> {
        match self {
            Panel::Repos => Some(ListId::Repos),
            Panel::Actions => Some(ListId::Runs),
            Panel::Deploys => Some(ListId::Deploys),
            Panel::Pulse => None,
            Panel::Pulls => Some(ListId::Pulls),
            Panel::Issues => Some(ListId::Issues),
            Panel::Explore => Some(ListId::Explore),
        }
    }
    pub fn view(self) -> View {
        match self {
            Panel::Repos => View::Repos,
            Panel::Actions => View::Actions,
            Panel::Deploys => View::Deploys,
            Panel::Pulse => View::Dashboard,
            Panel::Pulls => View::Pulls,
            Panel::Issues => View::Issues,
            Panel::Explore => View::Explore,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListId {
    Repos,
    Runs,
    Jobs,
    Deploys,
    Pulls,
    Issues,
    Explore,
    Inbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Repos,
    Runs,
    Issues,
    Commands,
    Hosts,
    Themes,
}

impl PickerKind {
    pub fn title(self) -> &'static str {
        match self {
            PickerKind::Repos => "Repositories",
            PickerKind::Runs => "Workflow runs",
            PickerKind::Issues => "Pull requests & issues",
            PickerKind::Commands => "Commands",
            PickerKind::Hosts => "Hosts",
            PickerKind::Themes => "Themes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Quit,
    Refresh,
    Goto(View),
    Pick(PickerKind),
    NextHost,
    Zoom,
    Watch,
    NextTheme,
    EditConfig,
    Open,
    Yank,
    Help,
    Errors,
    Filter,
    RerunFailed,
    RerunAll,
    CancelRun,
    ToggleAllRepos,
    ReloadIndex,
    Stats,
}

pub struct KeyNode {
    pub key: char,
    pub desc: &'static str,
    pub kind: NodeKind,
}

pub enum NodeKind {
    Cmd(Cmd),
    Group(Vec<KeyNode>),
}

fn node(key: char, desc: &'static str, cmd: Cmd) -> KeyNode {
    KeyNode { key, desc, kind: NodeKind::Cmd(cmd) }
}

fn group(key: char, desc: &'static str, children: Vec<KeyNode>) -> KeyNode {
    KeyNode { key, desc, kind: NodeKind::Group(children) }
}

/// The `<space>` tree, shown by the which-key popup.
pub fn leader_tree() -> Vec<KeyNode> {
    vec![
        node(' ', "find repo", Cmd::Pick(PickerKind::Repos)),
        group(
            'f',
            "find",
            vec![
                node('f', "repos", Cmd::Pick(PickerKind::Repos)),
                node('r', "workflow runs", Cmd::Pick(PickerKind::Runs)),
                node('p', "pulls & issues", Cmd::Pick(PickerKind::Issues)),
                node('c', "commands", Cmd::Pick(PickerKind::Commands)),
                node('h', "hosts", Cmd::Pick(PickerKind::Hosts)),
                node('t', "themes", Cmd::Pick(PickerKind::Themes)),
            ],
        ),
        group(
            'g',
            "goto",
            vec![
                node('d', "dashboard", Cmd::Goto(View::Dashboard)),
                node('r', "repos", Cmd::Goto(View::Repos)),
                node('a', "actions", Cmd::Goto(View::Actions)),
                node('p', "pulls", Cmd::Goto(View::Pulls)),
                node('i', "issues", Cmd::Goto(View::Issues)),
                node('D', "deploys", Cmd::Goto(View::Deploys)),
                node('e', "explore", Cmd::Goto(View::Explore)),
                node('n', "inbox", Cmd::Goto(View::Inbox)),
            ],
        ),
        group(
            'a',
            "actions",
            vec![
                node('r', "re-run failed jobs", Cmd::RerunFailed),
                node('R', "re-run all jobs", Cmd::RerunAll),
                node('x', "cancel run", Cmd::CancelRun),
            ],
        ),
        node('h', "switch host", Cmd::Pick(PickerKind::Hosts)),
        node('r', "refresh", Cmd::Refresh),
        node('R', "reload repo index", Cmd::ReloadIndex),
        node('w', "watch / unwatch", Cmd::Watch),
        node('z', "zoom panel", Cmd::Zoom),
        node('t', "next theme", Cmd::NextTheme),
        node('c', "edit config", Cmd::EditConfig),
        node('o', "open in browser", Cmd::Open),
        node('y', "yank url", Cmd::Yank),
        node('e', "error log", Cmd::Errors),
        node('s', "api & resource usage", Cmd::Stats),
        node('?', "help", Cmd::Help),
        node('q', "quit", Cmd::Quit),
    ]
}

pub fn command_list() -> Vec<(Cmd, &'static str, &'static str)> {
    vec![
        (Cmd::Pick(PickerKind::Repos), "Find repository", "C-p"),
        (Cmd::Pick(PickerKind::Runs), "Find workflow run", "␣fr"),
        (Cmd::Pick(PickerKind::Issues), "Find pull request / issue", "␣fp"),
        (Cmd::Goto(View::Dashboard), "Go to dashboard", "1"),
        (Cmd::Goto(View::Repos), "Go to repos", "2"),
        (Cmd::Goto(View::Actions), "Go to actions", "3"),
        (Cmd::Goto(View::Pulls), "Go to pull requests", "4"),
        (Cmd::Goto(View::Issues), "Go to issues", "5"),
        (Cmd::Goto(View::Deploys), "Go to deployments", "6"),
        (Cmd::Goto(View::Explore), "Go to explore (trending / top)", "7"),
        (Cmd::Goto(View::Inbox), "Go to inbox (notifications)", "8"),
        (Cmd::Refresh, "Refresh everything", "r"),
        (Cmd::ReloadIndex, "Reload repository index", "␣R"),
        (Cmd::Pick(PickerKind::Hosts), "Switch host", "H"),
        (Cmd::Pick(PickerKind::Themes), "Pick theme", "␣ft"),
        (Cmd::NextTheme, "Next theme", "t"),
        (Cmd::Watch, "Watch / unwatch selected repo", "w"),
        (Cmd::ToggleAllRepos, "Toggle watched / all repos", "a"),
        (Cmd::Zoom, "Zoom focused panel", "z"),
        (Cmd::RerunFailed, "Re-run failed jobs", "R"),
        (Cmd::RerunAll, "Re-run all jobs", "␣aR"),
        (Cmd::CancelRun, "Cancel run", "x"),
        (Cmd::Open, "Open in browser", "o"),
        (Cmd::Yank, "Copy URL (OSC 52)", "y"),
        (Cmd::EditConfig, "Edit config in $EDITOR", "␣c"),
        (Cmd::Errors, "Show error log", "E"),
        (Cmd::Stats, "API & resource usage", "S"),
        (Cmd::Help, "Help", "?"),
        (Cmd::Quit, "Quit", "q"),
    ]
}

pub enum PickAction {
    Repo(String),
    Url(String),
    Cmd(Cmd),
    Host(usize),
    Theme(usize),
    Run(String, u64),
}

pub struct PickItem {
    pub icon: String,
    pub color: Color,
    pub text: String,
    pub hint: String,
    pub action: PickAction,
}

pub struct Picker {
    pub kind: PickerKind,
    pub query: String,
    pub sel: usize,
}

pub enum Overlay {
    Picker(Picker),
    Leader(Vec<char>),
    Help(u16),
    Errors(u16),
    Stats(u16),
    Confirm(String, String, u64, RunAction),
}

#[derive(Default, Clone)]
pub struct SectionData {
    pub total: u64,
    pub items: Vec<Issue>,
    pub loaded: bool,
}

#[derive(Default)]
pub struct Data {
    pub viewer: Option<User>,
    pub watched: Vec<Repo>,
    pub all_repos: Vec<Repo>,
    pub runs: HashMap<String, Vec<WorkflowRun>>,
    pub deploys: HashMap<String, Vec<TierStatus>>,
    pub deploy_sigs: HashMap<String, String>,
    pub deploy_at: HashMap<String, Instant>,
    pub pulls: Vec<SectionData>,
    pub issues: Vec<SectionData>,
    pub notifications: Vec<Notification>,
    pub trending: Vec<Repo>,
    pub top: Vec<Repo>,
    pub calendar: Option<Calendar>,
    pub jobs: HashMap<u64, Vec<Job>>,
    pub details: HashMap<String, RepoDetail>,
    pub requested: HashSet<String>,
    pub last_refresh: Option<DateTime<Local>>,
    pub loaded: bool,
}

pub enum JobRow<'a> {
    Job(&'a Job),
    Step(&'a Step),
}

pub struct App {
    pub cfg: Arc<Config>,
    pub cfg_path: PathBuf,
    pub clients: Vec<Arc<Client>>,
    pub host_idx: usize,
    pub data: HashMap<String, Data>,
    pub state_file: StateFile,
    pub session_watch: HashMap<String, Vec<String>>,
    pub theme: Theme,
    pub theme_idx: usize,
    pub icons: Icons,
    pub view: View,
    pub panel: Panel,
    pub zoom: bool,
    pub overlay: Option<Overlay>,
    pub filtering: Option<ListId>,
    pub filters: HashMap<ListId, String>,
    pub states: RefCell<HashMap<ListId, TableState>>,
    pub rects: RefCell<Vec<(Panel, Rect)>>,
    pub explore_tab: usize,
    pub pull_sec: usize,
    pub issue_sec: usize,
    pub jobs_focus: bool,
    pub repos_all: bool,
    pub toast: Option<(String, bool, Instant)>,
    pub errors: Vec<(DateTime<Local>, String)>,
    pub tick: u64,
    pub busy: Arc<AtomicUsize>,
    pub tx: UnboundedSender<Envelope>,
    last_refresh: Option<Instant>,
    last_fast: Instant,
    last_slow: Option<Instant>,
    last_move: Instant,
    /// Usage at startup, recent samples (for the stats overlay) and the last logged sample.
    pub usage_start: Sample,
    pub usage: VecDeque<Sample>,
    usage_logged: Sample,
    last_throttle: u64,
    /// Rates are measured from the end of the first refresh, so the one-off startup load
    /// (every repo, deploys, index) does not inflate the per-hour numbers.
    pub usage_warm: bool,
    /// Ignore the rate limit when pacing (bench only, to measure the unthrottled cost).
    pub no_throttle: bool,
    pub quit: bool,
    pub suspend: bool,
}

const PAGE: isize = 10;

impl App {
    pub fn new(
        cfg: Config,
        cfg_path: PathBuf,
        host: Option<String>,
        tx: UnboundedSender<Envelope>,
    ) -> anyhow::Result<Self> {
        let state_file = StateFile::load();
        let clients = cfg.hosts.iter().map(|h| Client::new(h).map(Arc::new)).collect::<anyhow::Result<Vec<_>>>()?;
        let want = host.or_else(|| state_file.host.clone()).or_else(|| cfg.general.default_host.clone());
        let host_idx = want.and_then(|w| cfg.hosts.iter().position(|h| h.name == w || h.host == w)).unwrap_or(0);
        let theme_idx = theme::by_name(state_file.theme.as_deref().unwrap_or(&cfg.general.theme));
        let icons = icons::by_name(&cfg.general.icons);
        let mut data = HashMap::new();
        data.insert(cfg.hosts[host_idx].name.clone(), Data::default());
        let usage_start = Sample::take(clients.iter().map(|c| &c.metrics));
        Ok(Self {
            cfg: Arc::new(cfg),
            cfg_path,
            clients,
            host_idx,
            data,
            state_file,
            session_watch: HashMap::new(),
            theme: THEMES[theme_idx],
            theme_idx,
            icons,
            view: View::Dashboard,
            panel: Panel::Repos,
            zoom: false,
            overlay: None,
            filtering: None,
            filters: HashMap::new(),
            states: RefCell::new(HashMap::new()),
            rects: RefCell::new(Vec::new()),
            explore_tab: 0,
            pull_sec: 0,
            issue_sec: 0,
            jobs_focus: false,
            repos_all: false,
            toast: None,
            errors: Vec::new(),
            tick: 0,
            busy: Arc::new(AtomicUsize::new(0)),
            tx,
            last_refresh: None,
            last_fast: Instant::now(),
            last_slow: None,
            last_move: Instant::now(),
            usage_start,
            usage: VecDeque::from([usage_start]),
            usage_logged: usage_start,
            last_throttle: 1,
            usage_warm: false,
            no_throttle: false,
            quit: false,
            suspend: false,
        })
    }

    // ── Accessors ────────────────────────────────────────────────────────────

    pub fn host(&self) -> &HostConfig {
        &self.cfg.hosts[self.host_idx]
    }

    pub fn client(&self) -> &Arc<Client> {
        &self.clients[self.host_idx]
    }

    pub fn d(&self) -> &Data {
        &self.data[&self.host().name]
    }

    fn dm(&mut self) -> &mut Data {
        let name = self.host().name.clone();
        self.data.entry(name).or_default()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst) > 0
    }

    pub fn filter(&self, id: ListId) -> &str {
        self.filters.get(&id).map(String::as_str).unwrap_or("")
    }

    pub fn sel(&self, id: ListId) -> usize {
        self.states.borrow().get(&id).and_then(|s| s.selected()).unwrap_or(0)
    }

    fn set_sel(&self, id: ListId, i: usize) {
        self.states.borrow_mut().entry(id).or_default().select(Some(i));
    }

    pub fn section_titles(&self, kind: SectionKind) -> Vec<&str> {
        let secs = match kind {
            SectionKind::Pulls => &self.cfg.sections.pulls,
            SectionKind::Issues => &self.cfg.sections.issues,
        };
        secs.iter().map(|s| s.title.as_str()).collect()
    }

    pub fn section(&self, kind: SectionKind) -> Option<&SectionData> {
        match kind {
            SectionKind::Pulls => self.d().pulls.get(self.pull_sec),
            SectionKind::Issues => self.d().issues.get(self.issue_sec),
        }
    }

    // ── List items (shared by dashboard panels and full views) ─────────────

    pub fn repo_rows(&self) -> Vec<Hit<&Repo>> {
        let d = self.d();
        let src: Vec<&Repo> = if self.repos_all && !d.all_repos.is_empty() {
            d.all_repos.iter().collect()
        } else {
            d.watched.iter().collect()
        };
        fuzzy::filter(src, |r| r.full_name.clone(), self.filter(ListId::Repos))
    }

    pub fn run_rows(&self) -> Vec<Hit<&WorkflowRun>> {
        let mut v: Vec<&WorkflowRun> = self.d().runs.values().flatten().collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
        fuzzy::filter(
            v,
            |r| format!("{} {} {} {} {}", r.workflow(), r.repo, r.branch(), r.title(), r.event),
            self.filter(ListId::Runs),
        )
    }

    pub fn job_rows(&self) -> Vec<JobRow<'_>> {
        let Some(run) = self.selected_run() else { return vec![] };
        let Some(jobs) = self.d().jobs.get(&run.id) else { return vec![] };
        let mut out = Vec::new();
        for j in jobs {
            out.push(JobRow::Job(j));
            for s in &j.steps {
                out.push(JobRow::Step(s));
            }
        }
        out
    }

    pub fn deploy_rows(&self) -> Vec<Hit<(&str, &Vec<TierStatus>)>> {
        let d = self.d();
        let v: Vec<(&str, &Vec<TierStatus>)> = d
            .watched
            .iter()
            .filter_map(|r| d.deploys.get(&r.full_name).map(|t| (r.full_name.as_str(), t)))
            .filter(|(_, t)| t.iter().any(|t| !t.entries.is_empty()))
            .collect();
        fuzzy::filter(v, |(n, _)| n.to_string(), self.filter(ListId::Deploys))
    }

    pub fn issue_rows(&self, kind: SectionKind) -> Vec<Hit<&Issue>> {
        let id = match kind {
            SectionKind::Pulls => ListId::Pulls,
            SectionKind::Issues => ListId::Issues,
        };
        let items: Vec<&Issue> = self.section(kind).map(|s| s.items.iter().collect()).unwrap_or_default();
        fuzzy::filter(items, |i| format!("#{} {} {} {}", i.number, i.title, i.repo(), i.author()), self.filter(id))
    }

    pub fn explore_rows(&self) -> Vec<Hit<&Repo>> {
        let d = self.d();
        let src = if self.explore_tab == 0 { &d.trending } else { &d.top };
        fuzzy::filter(src.iter(), |r| r.full_name.clone(), self.filter(ListId::Explore))
    }

    pub fn inbox_rows(&self) -> Vec<Hit<&Notification>> {
        fuzzy::filter(
            self.d().notifications.iter(),
            |n| format!("{} {} {}", n.repository.full_name, n.subject.title, n.reason),
            self.filter(ListId::Inbox),
        )
    }

    pub fn list_len(&self, id: ListId) -> usize {
        match id {
            ListId::Repos => self.repo_rows().len(),
            ListId::Runs => self.run_rows().len(),
            ListId::Jobs => self.job_rows().len(),
            ListId::Deploys => self.deploy_rows().len(),
            ListId::Pulls => self.issue_rows(SectionKind::Pulls).len(),
            ListId::Issues => self.issue_rows(SectionKind::Issues).len(),
            ListId::Explore => self.explore_rows().len(),
            ListId::Inbox => self.inbox_rows().len(),
        }
    }

    pub fn selected_repo(&self) -> Option<&Repo> {
        self.repo_rows().into_iter().nth(self.sel(ListId::Repos)).map(|h| h.item)
    }

    pub fn selected_run(&self) -> Option<&WorkflowRun> {
        self.run_rows().into_iter().nth(self.sel(ListId::Runs)).map(|h| h.item)
    }

    pub fn selected_issue(&self, kind: SectionKind) -> Option<&Issue> {
        let id = if kind == SectionKind::Pulls { ListId::Pulls } else { ListId::Issues };
        self.issue_rows(kind).into_iter().nth(self.sel(id)).map(|h| h.item)
    }

    pub fn selected_deploy(&self) -> Option<(&str, &Vec<TierStatus>)> {
        self.deploy_rows().into_iter().nth(self.sel(ListId::Deploys)).map(|h| h.item)
    }

    /// Latest run on the default branch (falls back to any branch).
    pub fn repo_ci(&self, r: &Repo) -> Option<&WorkflowRun> {
        let runs = self.d().runs.get(&r.full_name)?;
        runs.iter().find(|x| x.branch() == r.default_branch).or_else(|| runs.first())
    }

    pub fn find_repo(&self, name: &str) -> Option<&Repo> {
        let d = self.d();
        d.watched
            .iter()
            .chain(d.all_repos.iter())
            .chain(d.trending.iter())
            .chain(d.top.iter())
            .find(|r| r.full_name == name)
    }

    pub fn cur_list(&self) -> Option<ListId> {
        match self.view {
            View::Dashboard => self.panel.list(),
            View::Repos => Some(ListId::Repos),
            View::Actions => Some(if self.jobs_focus { ListId::Jobs } else { ListId::Runs }),
            View::Pulls => Some(ListId::Pulls),
            View::Issues => Some(ListId::Issues),
            View::Deploys => Some(ListId::Deploys),
            View::Explore => Some(ListId::Explore),
            View::Inbox => Some(ListId::Inbox),
        }
    }

    pub fn cur_url(&self) -> Option<String> {
        let id = self.cur_list()?;
        let i = self.sel(id);
        match id {
            ListId::Repos => self.selected_repo().map(|r| r.html_url.clone()),
            ListId::Runs => self.selected_run().map(|r| r.html_url.clone()),
            ListId::Jobs => {
                let rows = self.job_rows();
                let job = rows.iter().take(i + 1).rev().find_map(|r| match r {
                    JobRow::Job(j) => Some(*j),
                    _ => None,
                });
                job.and_then(|j| j.html_url.clone()).or_else(|| self.selected_run().map(|r| r.html_url.clone()))
            }
            ListId::Deploys => self.selected_deploy().and_then(|(name, tiers)| {
                tiers
                    .iter()
                    .filter_map(|t| t.latest())
                    .max_by_key(|e| e.at)
                    .map(|e| e.url.clone())
                    .or_else(|| Some(format!("{}/{name}", self.client().web)))
            }),
            ListId::Pulls => self.selected_issue(SectionKind::Pulls).map(|x| x.html_url.clone()),
            ListId::Issues => self.selected_issue(SectionKind::Issues).map(|x| x.html_url.clone()),
            ListId::Explore => self.explore_rows().into_iter().nth(i).map(|h| h.item.html_url.clone()),
            ListId::Inbox => self.inbox_rows().into_iter().nth(i).map(|h| {
                let n = h.item;
                match &n.subject.url {
                    Some(u) => self.client().web_url(u),
                    None => format!("{}/actions", n.repository.html_url),
                }
            }),
        }
    }

    pub fn active_repos(&self) -> Vec<String> {
        self.d().runs.iter().filter(|(_, runs)| runs.iter().any(|r| r.is_active())).map(|(n, _)| n.clone()).collect()
    }

    pub fn mode(&self) -> (&'static str, Color) {
        let t = &self.theme;
        match &self.overlay {
            Some(Overlay::Picker(_)) => ("PICKER", t.magenta),
            Some(Overlay::Leader(_)) => ("LEADER", t.yellow),
            Some(Overlay::Help(_)) => ("HELP", t.cyan),
            Some(Overlay::Errors(_)) => ("ERRORS", t.red),
            Some(Overlay::Stats(_)) => ("STATS", t.cyan),
            Some(Overlay::Confirm(..)) => ("CONFIRM", t.red),
            None if self.filtering.is_some() => ("FILTER", t.green),
            None => ("NORMAL", t.accent),
        }
    }

    // ── Background work ────────────────────────────────────────────────────

    fn ctx(&self) -> Arc<Ctx> {
        let h = self.host();
        let mut extra = self.state_file.watch.get(&h.name).cloned().unwrap_or_default();
        extra.extend(self.session_watch.get(&h.name).cloned().unwrap_or_default());
        let d = self.d();
        Arc::new(Ctx {
            host: h.name.clone(),
            client: self.client().clone(),
            cfg: self.cfg.clone(),
            hcfg: h.clone(),
            extra,
            known: d.watched.clone(),
            have_runs: d.runs.keys().cloned().collect(),
            active: self.active_repos().into_iter().collect(),
            deploy_sigs: d.deploy_sigs.clone(),
            deploy_at: d.deploy_at.clone(),
            tx: self.tx.clone(),
            busy: self.busy.clone(),
        })
    }

    pub fn refresh(&mut self, manual: bool) {
        let slow = manual
            || self.last_slow.is_none_or(|t| t.elapsed() >= Duration::from_secs(self.cfg.general.slow_refresh_secs));
        self.last_refresh = Some(Instant::now());
        self.last_fast = Instant::now();
        if slow {
            self.last_slow = Some(Instant::now());
        }
        if manual {
            let d = self.dm();
            d.requested.retain(|k| !k.starts_with("jobs:"));
            d.jobs.clear();
            d.deploy_at.clear();
            self.notify("refreshing…");
        }
        tokio::spawn(fetch::refresh(self.ctx(), slow));
    }

    /// Runs the schedulers. Returns true when something visible changed.
    pub fn on_tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        let g = &self.cfg.general;
        let throttle = self.throttle();
        if throttle != self.last_throttle {
            tracing::warn!(host = %self.host().name, from = self.last_throttle, to = throttle, "throttle changed");
            self.last_throttle = throttle;
        }
        let due = self.last_refresh.is_none_or(|t| t.elapsed() >= Duration::from_secs(g.refresh_secs * throttle));
        if due && (!self.is_busy() || self.last_refresh.is_none()) {
            self.refresh(false);
        } else if self.last_fast.elapsed() >= Duration::from_secs(g.fast_refresh_secs * throttle) {
            self.last_fast = Instant::now();
            let active = self.active_repos();
            if !active.is_empty() {
                let d = self.dm();
                for name in &active {
                    if let Some(runs) = d.runs.get(name) {
                        for r in runs.iter().filter(|r| r.is_active()) {
                            d.requested.remove(&format!("jobs:{}", r.id));
                        }
                    }
                }
                tracing::debug!(repos = active.len(), "fast refresh");
                tokio::spawn(fetch::statuses(self.ctx(), active));
            }
        }
        if self.last_move.elapsed() >= Duration::from_millis(200) {
            self.lazy_load();
        }
        self.sample_usage();
        let had_toast = self.toast.is_some();
        if self.toast.as_ref().is_some_and(|(_, _, t)| t.elapsed() > Duration::from_secs(4)) {
            self.toast = None;
        }
        had_toast != self.toast.is_some()
    }

    /// Something on screen animates (spinners), so redraw on every tick.
    pub fn animating(&self) -> bool {
        self.is_busy() || !self.d().loaded || self.d().runs.values().flatten().any(|r| r.is_active())
    }

    fn sample_usage(&mut self) {
        let last = self.usage.back().map(|s| s.at).unwrap_or(self.usage_start.at);
        if last.elapsed() < Duration::from_secs(SAMPLE_SECS) {
            return;
        }
        let now = Sample::take(self.clients.iter().map(|c| &c.metrics));
        self.usage.push_back(now);
        while self.usage.len() > SAMPLES + 1 {
            self.usage.pop_front();
        }
        if now.at.duration_since(self.usage_logged.at) >= Duration::from_secs(USAGE_LOG_SECS) {
            let r = metrics::rates(&self.usage_logged, &now);
            let core = self.client().rates().core;
            tracing::info!(
                host = %self.host().name,
                watched = self.d().watched.len(),
                remaining = core.map(|c| c.remaining),
                limit = core.map(|c| c.limit),
                "usage {r}"
            );
            self.usage_logged = now;
        }
    }

    /// Usage rates since start and over the recent window.
    pub fn usage_rates(&self) -> (metrics::Rates, metrics::Rates) {
        let now = Sample::take(self.clients.iter().map(|c| &c.metrics));
        let window = self.usage.front().copied().unwrap_or(self.usage_start);
        (metrics::rates(&self.usage_start, &now), metrics::rates(&window, &now))
    }

    /// Stretches polling intervals as the core rate limit runs low.
    pub fn throttle(&self) -> u64 {
        if self.no_throttle {
            return 1;
        }
        match self.client().rates().core {
            Some(r) if r.limit > 0 => match r.remaining * 100 / r.limit {
                0..=9 => 6,
                10..=24 => 3,
                25..=39 => 2,
                _ => 1,
            },
            _ => 1,
        }
    }

    /// Fetches details for whatever is selected once the cursor has settled.
    fn lazy_load(&mut self) {
        let want_detail =
            self.view == View::Repos || (self.view == View::Dashboard && self.zoom && self.panel == Panel::Repos);
        if want_detail && let Some(name) = self.selected_repo().map(|r| r.full_name.clone()) {
            let key = format!("detail:{name}");
            if self.dm().requested.insert(key) {
                tokio::spawn(fetch::detail(self.ctx(), name));
            }
        }
        if self.view == View::Actions
            && let Some((repo, id)) = self.selected_run().map(|r| (r.repo.clone(), r.id))
        {
            let key = format!("jobs:{id}");
            if self.dm().requested.insert(key) {
                tokio::spawn(fetch::jobs(self.ctx(), repo, id));
            }
        }
    }

    pub fn on_msg(&mut self, env: Envelope) {
        let current = env.host == self.host().name;
        let d = self.data.entry(env.host.clone()).or_default();
        match env.msg {
            Msg::Viewer(u) => d.viewer = Some(u),
            Msg::Watched(v) => {
                d.watched = v;
                d.loaded = true;
            }
            Msg::AllRepos(v) => d.all_repos = v,
            Msg::Runs(name, runs) => {
                for r in runs.iter().filter(|r| r.is_active()) {
                    d.jobs.remove(&r.id);
                    d.requested.remove(&format!("jobs:{}", r.id));
                }
                d.runs.insert(name, runs);
            }
            Msg::Deploys(name, t, sig, at) => {
                d.deploy_sigs.insert(name.clone(), sig);
                d.deploy_at.insert(name.clone(), at);
                d.deploys.insert(name, t);
            }
            Msg::Section(kind, i, total, items) => {
                let secs = match kind {
                    SectionKind::Pulls => &mut d.pulls,
                    SectionKind::Issues => &mut d.issues,
                };
                if secs.len() <= i {
                    secs.resize(i + 1, SectionData::default());
                }
                secs[i] = SectionData { total, items, loaded: true };
            }
            Msg::Notifications(n) => d.notifications = n,
            Msg::Trending(v) => d.trending = v,
            Msg::Top(v) => d.top = v,
            Msg::Contributions(c) => d.calendar = Some(c),
            Msg::Jobs(id, jobs) => {
                d.jobs.insert(id, jobs);
            }
            Msg::Detail(name, det) => {
                d.details.insert(name, det);
            }
            Msg::RefreshDone => {
                d.last_refresh = Some(Local::now());
                if !self.usage_warm {
                    self.usage_warm = true;
                    let now = Sample::take(self.clients.iter().map(|c| &c.metrics));
                    (self.usage_start, self.usage_logged) = (now, now);
                    self.usage = VecDeque::from([now]);
                }
            }
            Msg::Info(s) => {
                if current {
                    self.notify(&s)
                }
            }
            Msg::Error(e) => {
                if self.errors.last().is_none_or(|(_, last)| *last != e) {
                    self.errors.push((Local::now(), e.clone()));
                }
                if self.errors.len() > 300 {
                    self.errors.drain(..100);
                }
                if current {
                    self.toast = Some((e, true, Instant::now()));
                }
            }
        }
    }

    // ── Input ──────────────────────────────────────────────────────────────

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(ov) = self.overlay.take() {
            self.on_overlay_key(ov, k);
            return;
        }
        if let Some(id) = self.filtering {
            self.on_filter_key(id, k);
            return;
        }
        self.on_normal_key(k);
    }

    fn on_normal_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => self.escape(),
            KeyCode::Char(' ') => self.overlay = Some(Overlay::Leader(vec![])),
            KeyCode::Char('?') => self.exec(Cmd::Help),
            KeyCode::Char(':') => self.exec(Cmd::Pick(PickerKind::Commands)),
            KeyCode::Char('p') if ctrl => self.exec(Cmd::Pick(PickerKind::Repos)),
            KeyCode::Char('/') => self.exec(Cmd::Filter),
            KeyCode::Char(c @ '1'..='8') => self.exec(Cmd::Goto(View::ALL[(c as u8 - b'1') as usize])),
            KeyCode::Char('h') if ctrl => self.focus_dir(-1, 0),
            KeyCode::Backspace if self.view == View::Dashboard => self.focus_dir(-1, 0),
            KeyCode::Char('l') if ctrl => self.focus_dir(1, 0),
            KeyCode::Char('j') if ctrl => self.focus_dir(0, 1),
            KeyCode::Char('k') if ctrl => self.focus_dir(0, -1),
            KeyCode::Char('d') if ctrl => self.move_by(PAGE),
            KeyCode::Char('u') if ctrl => self.move_by(-PAGE),
            KeyCode::Tab => self.tab(1),
            KeyCode::BackTab => self.tab(-1),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::PageDown => self.move_by(PAGE),
            KeyCode::PageUp => self.move_by(-PAGE),
            KeyCode::Char('g') | KeyCode::Home => self.move_to(0),
            KeyCode::Char('G') | KeyCode::End => self.move_to(usize::MAX),
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Char('[') => self.section_step(-1),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(']') => self.section_step(1),
            KeyCode::Enter => self.activate(),
            KeyCode::Char('o') => self.exec(Cmd::Open),
            KeyCode::Char('y') => self.exec(Cmd::Yank),
            KeyCode::Char('r') => self.exec(Cmd::Refresh),
            KeyCode::Char('R') => self.exec(Cmd::RerunFailed),
            KeyCode::Char('x') => self.exec(Cmd::CancelRun),
            KeyCode::Char('w') => self.exec(Cmd::Watch),
            KeyCode::Char('a') => self.exec(Cmd::ToggleAllRepos),
            KeyCode::Char('z') => self.exec(Cmd::Zoom),
            KeyCode::Char('H') => self.exec(Cmd::NextHost),
            KeyCode::Char('t') => self.exec(Cmd::NextTheme),
            KeyCode::Char('E') => self.exec(Cmd::Errors),
            KeyCode::Char('S') => self.exec(Cmd::Stats),
            _ => {}
        }
    }

    fn escape(&mut self) {
        if let Some(id) = self.cur_list()
            && self.filters.remove(&id).is_some_and(|f| !f.is_empty())
        {
            return;
        }
        if self.jobs_focus {
            self.jobs_focus = false;
        } else if self.zoom {
            self.zoom = false;
        } else if self.view != View::Dashboard {
            self.view = View::Dashboard;
        }
    }

    fn on_filter_key(&mut self, id: ListId, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let f = self.filters.entry(id).or_default();
        match k.code {
            KeyCode::Esc => {
                f.clear();
                self.filtering = None;
            }
            KeyCode::Enter => self.filtering = None,
            KeyCode::Backspace => {
                f.pop();
            }
            KeyCode::Char('u') if ctrl => f.clear(),
            KeyCode::Char('w') if ctrl => delete_word(f),
            KeyCode::Down => return self.move_by(1),
            KeyCode::Up => return self.move_by(-1),
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => return self.move_by(1),
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => return self.move_by(-1),
            KeyCode::Char(c) => f.push(c),
            _ => {}
        }
        self.set_sel(id, 0);
    }

    fn on_overlay_key(&mut self, ov: Overlay, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match ov {
            Overlay::Leader(mut path) => match k.code {
                KeyCode::Esc => {}
                KeyCode::Backspace => {
                    if path.pop().is_some() {
                        self.overlay = Some(Overlay::Leader(path));
                    }
                }
                KeyCode::Char(c) => {
                    let tree = leader_tree();
                    let mut nodes = &tree;
                    for p in &path {
                        match nodes.iter().find(|n| n.key == *p).map(|n| &n.kind) {
                            Some(NodeKind::Group(ch)) => nodes = ch,
                            _ => return,
                        }
                    }
                    match nodes.iter().find(|n| n.key == c).map(|n| &n.kind) {
                        Some(NodeKind::Group(_)) => {
                            path.push(c);
                            self.overlay = Some(Overlay::Leader(path));
                        }
                        Some(NodeKind::Cmd(cmd)) => self.exec(cmd.clone()),
                        None => self.notify_err(&format!("no mapping for ␣{}{c}", path.iter().collect::<String>())),
                    }
                }
                _ => self.overlay = Some(Overlay::Leader(path)),
            },
            Overlay::Picker(mut p) => {
                match k.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter => {
                        let items = self.picker_hits(&p);
                        if let Some(h) = items.into_iter().nth(p.sel) {
                            let item = h.item;
                            self.pick(item.action);
                        }
                        return;
                    }
                    KeyCode::Char('o') if ctrl => {
                        let items = self.picker_hits(&p);
                        if let Some(h) = items.into_iter().nth(p.sel) {
                            let url = match &h.item.action {
                                PickAction::Repo(n) => self.find_repo(n).map(|r| r.html_url.clone()),
                                PickAction::Url(u) => Some(u.clone()),
                                PickAction::Run(repo, id) => {
                                    Some(format!("{}/{repo}/actions/runs/{id}", self.client().web))
                                }
                                _ => None,
                            };
                            if let Some(u) = url {
                                self.open_url(&u);
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Tab => p.sel += 1,
                    KeyCode::Up | KeyCode::BackTab => p.sel = p.sel.saturating_sub(1),
                    KeyCode::Char('j') | KeyCode::Char('n') if ctrl => p.sel += 1,
                    KeyCode::Char('k') | KeyCode::Char('p') if ctrl => p.sel = p.sel.saturating_sub(1),
                    KeyCode::Char('d') if ctrl => p.sel += PAGE as usize,
                    KeyCode::Char('u') if ctrl => {
                        p.query.clear();
                        p.sel = 0;
                    }
                    KeyCode::Char('w') if ctrl => {
                        delete_word(&mut p.query);
                        p.sel = 0;
                    }
                    KeyCode::Backspace => {
                        if p.query.pop().is_none() {
                            return;
                        }
                        p.sel = 0;
                    }
                    KeyCode::Char(c) if !ctrl => {
                        p.query.push(c);
                        p.sel = 0;
                    }
                    _ => {}
                }
                let n = self.picker_hits(&p).len();
                p.sel = p.sel.min(n.saturating_sub(1));
                self.overlay = Some(Overlay::Picker(p));
            }
            Overlay::Help(s) | Overlay::Errors(s) | Overlay::Stats(s) => {
                let s = match k.code {
                    KeyCode::Esc | KeyCode::Char('q' | '?' | 'E' | 'S') => return,
                    KeyCode::Char('j') | KeyCode::Down => s.saturating_add(1),
                    KeyCode::Char('k') | KeyCode::Up => s.saturating_sub(1),
                    KeyCode::Char('d') if ctrl => s.saturating_add(10),
                    KeyCode::Char('u') if ctrl => s.saturating_sub(10),
                    KeyCode::Char('g') => 0,
                    _ => s,
                };
                self.overlay = Some(match ov {
                    Overlay::Help(_) => Overlay::Help(s),
                    Overlay::Errors(_) => Overlay::Errors(s),
                    _ => Overlay::Stats(s),
                });
            }
            Overlay::Confirm(msg, repo, id, action) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    tokio::spawn(fetch::run_action(self.ctx(), repo, id, action));
                    self.notify("sent…");
                }
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.overlay = Some(Overlay::Confirm(msg, repo, id, action)),
            },
        }
    }

    fn move_by(&mut self, delta: isize) {
        let Some(id) = self.cur_list() else { return };
        let len = self.list_len(id);
        if len == 0 {
            return;
        }
        let cur = self.sel(id) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        self.set_sel(id, next);
        self.last_move = Instant::now();
    }

    fn move_to(&mut self, i: usize) {
        let Some(id) = self.cur_list() else { return };
        let len = self.list_len(id);
        if len > 0 {
            self.set_sel(id, i.min(len - 1));
            self.last_move = Instant::now();
        }
    }

    fn tab(&mut self, dir: isize) {
        match self.view {
            View::Dashboard => {
                let i = Panel::ALL.iter().position(|p| *p == self.panel).unwrap_or(0) as isize;
                let n = Panel::ALL.len() as isize;
                let visible: Vec<Panel> = self.rects.borrow().iter().map(|(p, _)| *p).collect();
                for step in 1..=n {
                    let cand = Panel::ALL[((i + dir * step).rem_euclid(n)) as usize];
                    if visible.is_empty() || visible.contains(&cand) {
                        self.panel = cand;
                        break;
                    }
                }
            }
            View::Actions => self.jobs_focus = !self.jobs_focus,
            _ => self.section_step(dir),
        }
    }

    fn section_step(&mut self, dir: isize) {
        let step = |cur: usize, n: usize| -> usize {
            if n == 0 { 0 } else { (cur as isize + dir).rem_euclid(n as isize) as usize }
        };
        let in_panel = |p: Panel| self.view == View::Dashboard && self.panel == p;
        if self.view == View::Pulls || in_panel(Panel::Pulls) {
            self.pull_sec = step(self.pull_sec, self.cfg.sections.pulls.len());
            self.set_sel(ListId::Pulls, 0);
        } else if self.view == View::Issues || in_panel(Panel::Issues) {
            self.issue_sec = step(self.issue_sec, self.cfg.sections.issues.len());
            self.set_sel(ListId::Issues, 0);
        } else if self.view == View::Explore || in_panel(Panel::Explore) {
            self.explore_tab = step(self.explore_tab, 2);
            self.set_sel(ListId::Explore, 0);
        } else if self.view == View::Actions {
            self.jobs_focus = dir > 0;
        } else if self.view == View::Dashboard {
            self.focus_dir(dir as i32, 0);
        }
    }

    /// Moves focus to the nearest dashboard panel in a direction (like <C-w>hjkl).
    fn focus_dir(&mut self, dx: i32, dy: i32) {
        if self.view != View::Dashboard {
            return;
        }
        let rects = self.rects.borrow().clone();
        let Some((_, cur)) = rects.iter().find(|(p, _)| *p == self.panel) else { return };
        let c = |r: &Rect| (r.x as i32 + r.width as i32 / 2, r.y as i32 + r.height as i32 / 2);
        let (cx, cy) = c(cur);
        let best = rects
            .iter()
            .filter(|(p, _)| *p != self.panel)
            .filter_map(|(p, r)| {
                let (x, y) = c(r);
                let (ddx, ddy) = (x - cx, y - cy);
                let along = ddx * dx + ddy * dy;
                if along <= 0 {
                    return None;
                }
                let across = (ddx * dy).abs() + (ddy * dx).abs();
                Some((along + across * 2, *p))
            })
            .min_by_key(|(score, _)| *score);
        if let Some((_, p)) = best {
            self.panel = p;
        }
    }

    fn activate(&mut self) {
        match self.view {
            View::Dashboard => {
                let target = self.panel.view();
                if target == View::Dashboard {
                    return;
                }
                if self.panel == Panel::Explore || self.panel == Panel::Pulls || self.panel == Panel::Issues {
                    return self.exec(Cmd::Open);
                }
                self.view = target;
            }
            View::Repos => {
                if let Some(name) = self.selected_repo().map(|r| r.full_name.clone()) {
                    self.filters.insert(ListId::Runs, format!("'{name}"));
                    self.set_sel(ListId::Runs, 0);
                    self.view = View::Actions;
                    self.jobs_focus = false;
                }
            }
            View::Actions if !self.jobs_focus => {
                self.jobs_focus = true;
                self.set_sel(ListId::Jobs, 0);
                self.last_move = Instant::now() - Duration::from_secs(1);
                self.lazy_load();
            }
            _ => self.exec(Cmd::Open),
        }
    }

    pub fn exec(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit => self.quit = true,
            Cmd::Refresh => self.refresh(true),
            Cmd::Goto(v) => {
                self.view = v;
                self.zoom = false;
                self.last_move = Instant::now();
            }
            Cmd::Pick(kind) => self.overlay = Some(Overlay::Picker(Picker { kind, query: String::new(), sel: 0 })),
            Cmd::NextHost => {
                if self.clients.len() > 1 {
                    self.switch_host((self.host_idx + 1) % self.clients.len());
                } else {
                    self.notify("only one host configured, add more under [[hosts]]");
                }
            }
            Cmd::Zoom => self.zoom = !self.zoom,
            Cmd::Watch => self.toggle_watch(),
            Cmd::NextTheme => self.set_theme((self.theme_idx + 1) % THEMES.len()),
            Cmd::EditConfig => self.suspend = true,
            Cmd::Open => match self.cur_url() {
                Some(u) => self.open_url(&u),
                None => self.notify("nothing to open"),
            },
            Cmd::Yank => match self.cur_url() {
                Some(u) => {
                    osc52(&u);
                    self.notify(&format!("copied {u}"));
                }
                None => self.notify("nothing to copy"),
            },
            Cmd::Help => self.overlay = Some(Overlay::Help(0)),
            Cmd::Errors => self.overlay = Some(Overlay::Errors(0)),
            Cmd::Stats => self.overlay = Some(Overlay::Stats(0)),
            Cmd::Filter => {
                if let Some(id) = self.cur_list() {
                    self.filtering = Some(id);
                    self.filters.entry(id).or_default();
                }
            }
            Cmd::RerunFailed | Cmd::RerunAll | Cmd::CancelRun => self.confirm_run(cmd),
            Cmd::ToggleAllRepos => {
                if self.cur_list() != Some(ListId::Repos) {
                    return;
                }
                self.repos_all = !self.repos_all;
                self.set_sel(ListId::Repos, 0);
                if self.repos_all && self.d().all_repos.is_empty() {
                    tokio::spawn(fetch::index(self.ctx()));
                }
                self.notify(if self.repos_all { "showing all accessible repos" } else { "showing watched repos" });
            }
            Cmd::ReloadIndex => {
                tokio::spawn(fetch::index(self.ctx()));
                self.notify("reloading repository index…");
            }
        }
    }

    fn confirm_run(&mut self, cmd: Cmd) {
        if !matches!(self.cur_list(), Some(ListId::Runs | ListId::Jobs)) {
            return self.notify("select a workflow run first (3 → actions)");
        }
        let Some(run) = self.selected_run() else { return };
        let (repo, id, num, wf) = (run.repo.clone(), run.id, run.run_number, run.workflow().to_string());
        let (action, verb) = match cmd {
            Cmd::RerunFailed => (RunAction::RerunFailed, "Re-run failed jobs of"),
            Cmd::RerunAll => (RunAction::RerunAll, "Re-run all jobs of"),
            _ => (RunAction::Cancel, "Cancel"),
        };
        self.overlay = Some(Overlay::Confirm(format!("{verb} {wf} #{num} in {repo}?"), repo, id, action));
    }

    fn pick(&mut self, action: PickAction) {
        match action {
            PickAction::Repo(name) => self.focus_repo(&name),
            PickAction::Url(u) => self.open_url(&u),
            PickAction::Cmd(c) => self.exec(c),
            PickAction::Host(i) => self.switch_host(i),
            PickAction::Theme(i) => self.set_theme(i),
            PickAction::Run(_, id) => {
                self.filters.remove(&ListId::Runs);
                if let Some(i) = self.run_rows().iter().position(|h| h.item.id == id) {
                    self.set_sel(ListId::Runs, i);
                }
                self.view = View::Actions;
                self.jobs_focus = false;
                self.last_move = Instant::now();
            }
        }
    }

    /// Selects a repo in the repos view, adding it to the session watch list if needed.
    pub fn focus_repo(&mut self, name: &str) {
        let host = self.host().name.clone();
        if !self.d().watched.iter().any(|r| r.full_name == name) {
            let Some(repo) = self.find_repo(name).cloned() else { return };
            self.dm().watched.insert(0, repo);
            self.session_watch.entry(host).or_default().push(name.to_string());
            let ctx = self.ctx();
            let n = name.to_string();
            tokio::spawn(async move { fetch::repo_status(&ctx, &n, true).await });
        }
        self.repos_all = false;
        self.filters.remove(&ListId::Repos);
        if let Some(i) = self.repo_rows().iter().position(|h| h.item.full_name == name) {
            self.set_sel(ListId::Repos, i);
        }
        self.view = View::Repos;
        self.last_move = Instant::now();
    }

    fn toggle_watch(&mut self) {
        let name = match self.cur_list() {
            Some(ListId::Repos) => self.selected_repo().map(|r| r.full_name.clone()),
            Some(ListId::Explore) => {
                self.explore_rows().into_iter().nth(self.sel(ListId::Explore)).map(|h| h.item.full_name.clone())
            }
            _ => None,
        };
        let Some(name) = name else { return self.notify("select a repo to watch") };
        let host = self.host().name.clone();
        let list = self.state_file.watch.entry(host.clone()).or_default();
        let msg = if let Some(pos) = list.iter().position(|n| *n == name) {
            list.remove(pos);
            if !self.host().repos.contains(&name) {
                if let Some(s) = self.session_watch.get_mut(&host) {
                    s.retain(|n| *n != name);
                }
                self.dm().watched.retain(|r| r.full_name != name);
            }
            format!("unwatched {name}")
        } else {
            list.push(name.clone());
            if !self.d().watched.iter().any(|r| r.full_name == name) {
                if let Some(r) = self.find_repo(&name).cloned() {
                    self.dm().watched.insert(0, r);
                }
                let ctx = self.ctx();
                let n = name.clone();
                tokio::spawn(async move { fetch::repo_status(&ctx, &n, true).await });
            }
            format!("watching {name}")
        };
        match self.state_file.save() {
            Ok(()) => self.notify(&msg),
            Err(e) => self.notify_err(&format!("save state: {e}")),
        }
    }

    pub fn switch_host(&mut self, i: usize) {
        if i >= self.clients.len() || i == self.host_idx {
            return;
        }
        self.host_idx = i;
        self.state_file.host = Some(self.host().name.clone());
        let _ = self.state_file.save();
        let fresh = !self.data.contains_key(&self.host().name);
        self.dm();
        self.filters.clear();
        self.states.borrow_mut().clear();
        self.last_slow = None;
        if fresh || !self.d().loaded {
            self.refresh(false);
        }
        self.notify(&format!("host → {}", self.host().host));
    }

    fn set_theme(&mut self, i: usize) {
        self.theme_idx = i;
        self.theme = THEMES[i];
        self.state_file.theme = Some(self.theme.name.to_string());
        let _ = self.state_file.save();
        self.notify(&format!("theme → {}", self.theme.name));
    }

    pub fn reload_config(&mut self) {
        match crate::config::load(&self.cfg_path) {
            Ok(cfg) => {
                let clients: anyhow::Result<Vec<_>> = cfg.hosts.iter().map(|h| Client::new(h).map(Arc::new)).collect();
                match clients {
                    Ok(c) => {
                        self.clients = c;
                        self.host_idx = self.host_idx.min(cfg.hosts.len() - 1);
                        self.icons = icons::by_name(&cfg.general.icons);
                        self.cfg = Arc::new(cfg);
                        self.data.clear();
                        self.dm();
                        self.pull_sec = 0;
                        self.issue_sec = 0;
                        self.refresh(true);
                        self.notify("config reloaded");
                    }
                    Err(e) => self.notify_err(&format!("config: {e:#}")),
                }
            }
            Err(e) => self.notify_err(&format!("config: {e:#}")),
        }
    }

    fn open_url(&mut self, url: &str) {
        let headless = std::env::var_os("DISPLAY").is_none()
            && std::env::var_os("WAYLAND_DISPLAY").is_none()
            && cfg!(target_os = "linux")
            && std::env::var_os("WSL_DISTRO_NAME").is_none();
        if headless || open::that_detached(url).is_err() {
            osc52(url);
            self.notify(&format!("no browser here, copied {url}"));
        } else {
            self.notify(&format!("opened {url}"));
        }
    }

    pub fn notify(&mut self, s: &str) {
        self.toast = Some((s.to_string(), false, Instant::now()));
    }

    fn notify_err(&mut self, s: &str) {
        self.toast = Some((s.to_string(), true, Instant::now()));
    }

    // ── Picker sources ─────────────────────────────────────────────────────

    pub fn picker_items(&self, kind: PickerKind) -> Vec<PickItem> {
        let t = &self.theme;
        let ic = &self.icons;
        let d = self.d();
        match kind {
            PickerKind::Repos => {
                let mut seen = HashSet::new();
                d.watched
                    .iter()
                    .chain(d.all_repos.iter())
                    .filter(|r| seen.insert(r.full_name.as_str()))
                    .map(|r| PickItem {
                        icon: (if r.private { ic.lock } else { ic.repo }).to_string(),
                        color: if r.archived { t.dim } else { t.accent },
                        text: r.full_name.clone(),
                        hint: format!("{} {}", ic.star, crate::ui::fmt::count(r.stargazers_count)),
                        action: PickAction::Repo(r.full_name.clone()),
                    })
                    .collect()
            }
            PickerKind::Runs => {
                let mut v: Vec<&WorkflowRun> = d.runs.values().flatten().collect();
                v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
                v.into_iter()
                    .map(|r| {
                        let (icon, color) = crate::ui::run_icon(self, r.status.as_deref(), r.conclusion.as_deref());
                        PickItem {
                            icon: icon.to_string(),
                            color,
                            text: format!("{} · {} · {}", r.workflow(), r.repo, r.branch()),
                            hint: crate::ui::fmt::ago(r.created_at),
                            action: PickAction::Run(r.repo.clone(), r.id),
                        }
                    })
                    .collect()
            }
            PickerKind::Issues => d
                .pulls
                .iter()
                .chain(d.issues.iter())
                .flat_map(|s| s.items.iter())
                .map(|i| PickItem {
                    icon: (if i.is_pr() { ic.pr } else { ic.issue }).to_string(),
                    color: if i.is_pr() { t.magenta } else { t.green },
                    text: format!("{}#{} {}", i.repo(), i.number, i.title),
                    hint: crate::ui::fmt::ago(i.updated_at),
                    action: PickAction::Url(i.html_url.clone()),
                })
                .collect(),
            PickerKind::Commands => command_list()
                .into_iter()
                .map(|(cmd, desc, key)| PickItem {
                    icon: ic.cmd.to_string(),
                    color: t.yellow,
                    text: desc.to_string(),
                    hint: key.to_string(),
                    action: PickAction::Cmd(cmd),
                })
                .collect(),
            PickerKind::Hosts => self
                .cfg
                .hosts
                .iter()
                .enumerate()
                .map(|(i, h)| PickItem {
                    icon: ic.host.to_string(),
                    color: if i == self.host_idx { t.green } else { t.accent },
                    text: format!("{} ({})", h.name, h.host),
                    hint: if i == self.host_idx { "active".into() } else { String::new() },
                    action: PickAction::Host(i),
                })
                .collect(),
            PickerKind::Themes => THEMES
                .iter()
                .enumerate()
                .map(|(i, th)| PickItem {
                    icon: "●".into(),
                    color: th.accent,
                    text: th.name.to_string(),
                    hint: if i == self.theme_idx { "active".into() } else { String::new() },
                    action: PickAction::Theme(i),
                })
                .collect(),
        }
    }

    pub fn picker_hits(&self, p: &Picker) -> Vec<Hit<PickItem>> {
        fuzzy::filter(self.picker_items(p.kind), |i| i.text.clone(), &p.query)
    }
}

fn delete_word(s: &mut String) {
    let trimmed = s.trim_end().len();
    s.truncate(trimmed);
    let cut = s.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-').map(|i| i + 1).unwrap_or(0);
    s.truncate(cut);
}

/// Copies to the system clipboard through the terminal (works over SSH and in tmux with
/// `set -g set-clipboard on`).
fn osc52(text: &str) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{b64}\x07");
    let _ = out.flush();
}
