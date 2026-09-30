use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use futures::{StreamExt, future::join_all, stream};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    config::{Config, HostConfig, expand_query},
    deploy::{self, DState, Entry, Source, TierStatus},
    github::{Client, models::*},
    local,
    metrics::{Quota, Stat},
};

/// After a push, keep polling a repo's runs for this long; CI usually starts within a minute
/// and the fast path takes over while runs are active.
const PUSH_WINDOW_MINS: i64 = 10;
/// Deployments and releases are re-fetched when a repo's runs change, and at the latest after
/// this long (they can change without a run, e.g. a manual release).
const DEPLOY_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Pulls,
    Issues,
}

pub enum Msg {
    Viewer(User),
    Watched(Vec<Repo>),
    AllRepos(Vec<Repo>),
    Runs(String, Vec<WorkflowRun>),
    Deploys(String, Vec<TierStatus>, String, Instant),
    Section(SectionKind, usize, u64, Vec<Issue>),
    Notifications(Vec<Notification>),
    Trending(Vec<Repo>),
    Top(Vec<Repo>),
    Contributions(Calendar),
    Jobs(u64, Vec<Job>),
    Detail(String, RepoDetail),
    Error(String),
    Info(String),
    RefreshDone,
}

pub struct Envelope {
    pub host: String,
    pub msg: Msg,
}

/// Everything a background task needs for one host.
pub struct Ctx {
    pub host: String,
    pub client: Arc<Client>,
    pub cfg: Arc<Config>,
    pub hcfg: HostConfig,
    pub extra: Vec<String>,
    /// Current watch list, reused between slow refreshes to save calls.
    pub known: Vec<Repo>,
    /// Repos that already have runs loaded.
    pub have_runs: HashSet<String>,
    /// Repos with queued or running workflow runs.
    pub active: HashSet<String>,
    /// Run signature per repo at the time deploys were last fetched.
    pub deploy_sigs: HashMap<String, String>,
    /// When deploys were last fetched per repo.
    pub deploy_at: HashMap<String, Instant>,
    pub tx: UnboundedSender<Envelope>,
    pub busy: Arc<AtomicUsize>,
}

impl Ctx {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(Envelope { host: self.host.clone(), msg });
    }
    fn err(&self, e: anyhow::Error) {
        self.send(Msg::Error(format!("{e:#}")));
    }
    fn conc(&self) -> usize {
        self.cfg.general.concurrency
    }
}

/// Counts in-flight work so the UI can show a spinner.
struct Busy(Arc<AtomicUsize>);
impl Busy {
    fn new(c: &Arc<AtomicUsize>) -> Self {
        c.fetch_add(1, Ordering::SeqCst);
        Self(c.clone())
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub async fn refresh(ctx: Arc<Ctx>, slow: bool) {
    let _b = Busy::new(&ctx.busy);
    let t0 = Instant::now();
    let before = ctx.client.metrics.total(Quota::Core);
    let viewer = match ctx.client.viewer().await {
        Ok(v) => v,
        Err(e) => {
            ctx.err(e);
            ctx.send(Msg::RefreshDone);
            return;
        }
    };
    let login = viewer.login.clone();
    ctx.send(Msg::Viewer(viewer));
    let ((watched, polled), ..) =
        tokio::join!(watched(&ctx, slow), searches(&ctx, &login), notifications(&ctx), async {
            if slow {
                tokio::join!(explore(&ctx), all_repos(&ctx), contributions(&ctx));
            }
        });
    let _ = ctx.client.refresh_rates().await;
    let after = ctx.client.metrics.total(Quota::Core);
    let delta =
        Stat { calls: after.calls - before.calls, not_modified: after.not_modified - before.not_modified, ..after };
    tracing::info!(
        host = %ctx.host, slow, watched, polled, calls = delta.calls, not_modified = delta.not_modified,
        cost = delta.cost(), remaining = ctx.client.rates().core.map(|r| r.remaining),
        ms = t0.elapsed().as_millis() as u64, "refresh"
    );
    ctx.send(Msg::RefreshDone);
}

/// Decides whether a repo's runs are fetched in a regular (non-slow) refresh. Unchanged,
/// idle repos wait for the slow refresh.
pub fn should_poll(
    pushed: Option<chrono::DateTime<chrono::Utc>>,
    prev_pushed: Option<chrono::DateTime<chrono::Utc>>,
    has_runs: bool,
    active: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    !has_runs
        || active
        || (prev_pushed.is_some() && pushed != prev_pushed)
        || pushed.is_some_and(|p| now - p < chrono::Duration::minutes(PUSH_WINDOW_MINS))
}

/// Builds the watch list (pinned + cloned locally + recently pushed) and polls the repos that
/// may have changed. Returns (watched, polled).
async fn watched(ctx: &Ctx, slow: bool) -> (usize, usize) {
    let c = &ctx.client;
    let h = &ctx.hcfg;
    let (dirs, host) = (h.local_dirs.clone(), h.host.clone());
    let local = tokio::task::spawn_blocking(move || local::discover(&dirs, &host)).await.unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for n in h.repos.iter().chain(ctx.extra.iter()).chain(local.keys()) {
        if !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            names.push(n.clone());
        }
    }
    // Revalidate metadata every time: unchanged repos answer 304, which is free, and a new
    // pushed_at tells us which repos need their runs polled.
    let local = &local;
    let named: Vec<Repo> = stream::iter(names.clone())
        .map(|n| async move {
            match c.repo(&n).await {
                Ok(r) => Some(r),
                Err(e) => {
                    let known = ctx.known.iter().find(|r| r.full_name.eq_ignore_ascii_case(&n)).cloned();
                    if local.contains_key(&n) && !h.repos.contains(&n) {
                        // A clone whose remote was renamed or removed: not worth a UI error.
                        warn_once(&n, &format!("{e:#} (cloned at {})", local[&n].display()));
                    } else if known.is_none() {
                        ctx.err(e);
                    }
                    known
                }
            }
        })
        .buffered(ctx.conc())
        .filter_map(|r| async { r })
        .collect()
        .await;

    let mut auto: Vec<Repo> = Vec::new();
    if h.auto_repos > 0 {
        match c.user_repos(h.auto_repos).await {
            Ok(v) => auto.extend(v),
            Err(e) => ctx.err(e),
        }
    }
    let orgs = join_all(h.orgs.iter().map(|o| c.org_repos(o, h.org_repos))).await;
    for r in orgs {
        match r {
            Ok(v) => auto.extend(v),
            Err(e) => ctx.err(e),
        }
    }
    auto.retain(|r| !r.archived);
    auto.sort_by_key(|x| std::cmp::Reverse(x.pushed_at));

    // Pinned repos keep their order; the rest follow, most recently pushed first.
    let mut list: Vec<Repo> = Vec::with_capacity(named.len() + auto.len());
    let mut rest: Vec<Repo> = Vec::new();
    for r in named.into_iter().chain(auto) {
        if r.archived || list.iter().chain(rest.iter()).any(|x| x.full_name == r.full_name) {
            continue;
        }
        let is_pinned = h.repos.iter().chain(ctx.extra.iter()).any(|n| n.eq_ignore_ascii_case(&r.full_name));
        if is_pinned { list.push(r) } else { rest.push(r) }
    }
    rest.sort_by_key(|x| std::cmp::Reverse(x.pushed_at));
    list.extend(rest);
    ctx.send(Msg::Watched(list.clone()));

    let now = chrono::Utc::now();
    let poll: Vec<String> = list
        .iter()
        .filter(|r| {
            let prev = ctx.known.iter().find(|k| k.full_name == r.full_name).and_then(|k| k.pushed_at);
            slow || should_poll(
                r.pushed_at,
                prev,
                ctx.have_runs.contains(&r.full_name),
                ctx.active.contains(&r.full_name),
                now,
            )
        })
        .map(|r| r.full_name.clone())
        .collect();
    let polled = poll.len();
    stream::iter(poll)
        .for_each_concurrent(ctx.conc(), |name| async move { repo_status(ctx, &name, false).await })
        .await;
    (list.len(), polled)
}

fn warn_once(key: &str, msg: &str) {
    static SEEN: std::sync::Mutex<Option<HashSet<String>>> = std::sync::Mutex::new(None);
    if SEEN.lock().unwrap().get_or_insert_default().insert(key.to_string()) {
        tracing::warn!("{msg}");
    }
}

/// Identifies what the deploy tiers depend on in a repo's recent runs; deploys are re-fetched
/// when it changes. Deploy workflows count with every status change (they are shown live),
/// other runs only once they complete, so CI moving from queued to in_progress costs nothing.
fn run_signature(runs: &[WorkflowRun], tiers: &[crate::config::Tier]) -> String {
    runs.iter()
        .take(10)
        .filter_map(|r| {
            let conclusion = r.conclusion.as_deref().unwrap_or("");
            if tiers.iter().any(|t| deploy::run_matches(&t.workflows, r)) {
                Some(format!("{}:{}:{conclusion}", r.id, r.status.as_deref().unwrap_or("")))
            } else {
                (!r.is_active()).then(|| format!("{}:{conclusion}", r.id))
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Workflow runs plus the deploy tiers for one repo. Deployments and releases are only
/// fetched when `force` is set, the runs changed since last time, or DEPLOY_TTL has passed.
pub async fn repo_status(ctx: &Ctx, name: &str, force: bool) {
    let c = &ctx.client;
    let tiers = &ctx.cfg.deploy.tiers;
    let runs = match c.runs(name, ctx.cfg.general.runs_per_repo).await {
        Ok(r) => r,
        Err(e) => {
            ctx.err(e);
            return;
        }
    };
    let sig = run_signature(&runs, tiers);
    ctx.send(Msg::Runs(name.to_string(), runs.clone()));
    let fresh = ctx.deploy_at.get(name).is_some_and(|t| t.elapsed() < DEPLOY_TTL);
    if !force && fresh && ctx.deploy_sigs.get(name) == Some(&sig) {
        return;
    }
    let need_env = tiers.iter().any(|t| !t.environments.is_empty());
    let need_rel = tiers.iter().any(|t| t.release.is_some());
    let (deps, rels) = tokio::join!(async { if need_env { c.deployments(name).await } else { Ok(vec![]) } }, async {
        if need_rel { c.releases(name).await } else { Ok(vec![]) }
    },);
    let deps = deps.unwrap_or_default();
    let rels = rels.unwrap_or_default();

    let mut out = Vec::with_capacity(tiers.len());
    for tier in tiers {
        let mut entries = Vec::new();
        if let Some(d) = deps.iter().find(|d| tier.environments.iter().any(|p| deploy::glob(p, &d.environment))) {
            let st = c.deployment_status(name, d.id).await.ok().flatten();
            entries.push(Entry {
                source: Source::Environment,
                label: d.environment.clone(),
                git_ref: if d.git_ref.len() == 40 { d.git_ref[..7].to_string() } else { d.git_ref.clone() },
                state: st.as_ref().map(|s| deploy::deployment_state(&s.state)).unwrap_or(DState::Unknown),
                at: st.as_ref().map(|s| s.created_at).unwrap_or(d.created_at),
                url: st
                    .as_ref()
                    .and_then(|s| s.environment_url.clone().filter(|u| !u.is_empty()).or(s.log_url.clone()))
                    .unwrap_or_else(|| format!("{}/{name}/deployments", c.web)),
                actor: d.creator.as_ref().map(|u| u.login.clone()),
            });
        }
        if let Some(r) = runs.iter().find(|r| deploy::run_matches(&tier.workflows, r)) {
            entries.push(Entry {
                source: Source::Workflow,
                label: r.workflow().to_string(),
                git_ref: r.branch().to_string(),
                state: deploy::run_state(r.status.as_deref(), r.conclusion.as_deref()),
                at: r.run_started_at.unwrap_or(r.created_at),
                url: r.html_url.clone(),
                actor: r.actor.as_ref().map(|u| u.login.clone()),
            });
        }
        let rel = match tier.release.as_deref() {
            Some("latest") => rels.iter().find(|r| !r.draft && !r.prerelease),
            Some("prerelease") => rels.iter().find(|r| !r.draft && r.prerelease),
            _ => None,
        };
        if let Some(r) = rel {
            entries.push(Entry {
                source: Source::Release,
                label: r.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| r.tag_name.clone()),
                git_ref: r.tag_name.clone(),
                state: DState::Success,
                at: r.published_at.unwrap_or(r.created_at),
                url: r.html_url.clone(),
                actor: r.author.as_ref().map(|u| u.login.clone()),
            });
        }
        out.push(TierStatus { tier: tier.name.clone(), entries });
    }
    ctx.send(Msg::Deploys(name.to_string(), out, sig, Instant::now()));
}

async fn searches(ctx: &Ctx, login: &str) {
    let c = &ctx.client;
    let jobs = [SectionKind::Pulls, SectionKind::Issues].into_iter().flat_map(|kind| {
        let secs = match kind {
            SectionKind::Pulls => &ctx.cfg.sections.pulls,
            SectionKind::Issues => &ctx.cfg.sections.issues,
        };
        secs.iter().enumerate().map(move |(i, s)| async move {
            let q = expand_query(&s.filter.replace("@me", login));
            match c.search_issues(&q, s.limit).await {
                Ok(r) => ctx.send(Msg::Section(kind, i, r.total_count, r.items)),
                Err(e) => ctx.err(e),
            }
        })
    });
    join_all(jobs).await;
}

async fn notifications(ctx: &Ctx) {
    match ctx.client.notifications().await {
        Ok(n) => ctx.send(Msg::Notifications(n)),
        Err(e) => ctx.err(e),
    }
}

async fn explore(ctx: &Ctx) {
    let c = &ctx.client;
    let e = &ctx.cfg.explore;
    let tq = expand_query(ctx.hcfg.trending_query.as_deref().unwrap_or(&e.trending_query));
    let pq = expand_query(ctx.hcfg.top_query.as_deref().unwrap_or(&e.top_query));
    let (t, p) = tokio::join!(c.search_repos(&tq, e.limit), c.search_repos(&pq, e.limit));
    match t {
        Ok(r) => ctx.send(Msg::Trending(r.items)),
        Err(e) => ctx.err(e),
    }
    match p {
        Ok(r) => ctx.send(Msg::Top(r.items)),
        Err(e) => ctx.err(e),
    }
}

pub async fn all_repos(ctx: &Ctx) {
    match ctx.client.all_repos(10).await {
        Ok(v) => ctx.send(Msg::AllRepos(v)),
        Err(e) => ctx.err(e),
    }
}

async fn contributions(ctx: &Ctx) {
    // Not every Enterprise Server exposes the calendar; stay quiet if it is missing.
    if let Ok(cal) = ctx.client.contributions().await {
        ctx.send(Msg::Contributions(cal));
    }
}

pub async fn detail(ctx: Arc<Ctx>, name: String) {
    let _b = Busy::new(&ctx.busy);
    let c = &ctx.client;
    let (langs, part, rels) = tokio::join!(c.languages(&name), c.participation(&name), c.releases(&name));
    let mut weekly = part.ok().flatten().map(|p| p.all).unwrap_or_default();
    if weekly.is_empty() {
        // Stats are computed on demand; GitHub answers 202 until they are ready.
        tokio::time::sleep(Duration::from_secs(3)).await;
        weekly = c.participation(&name).await.ok().flatten().map(|p| p.all).unwrap_or_default();
    }
    ctx.send(Msg::Detail(
        name,
        RepoDetail { languages: langs.unwrap_or_default(), weekly, releases: rels.unwrap_or_default() },
    ));
}

pub async fn jobs(ctx: Arc<Ctx>, repo: String, run: u64) {
    let _b = Busy::new(&ctx.busy);
    match ctx.client.jobs(&repo, run).await {
        Ok(j) => ctx.send(Msg::Jobs(run, j)),
        Err(e) => ctx.err(e),
    }
}

pub async fn statuses(ctx: Arc<Ctx>, repos: Vec<String>) {
    let _b = Busy::new(&ctx.busy);
    let c = &ctx;
    stream::iter(repos).for_each_concurrent(ctx.conc(), |r| async move { repo_status(c, &r, false).await }).await;
}

pub async fn index(ctx: Arc<Ctx>) {
    let _b = Busy::new(&ctx.busy);
    all_repos(&ctx).await;
}

#[derive(Debug, Clone, Copy)]
pub enum RunAction {
    RerunFailed,
    RerunAll,
    Cancel,
}

pub async fn run_action(ctx: Arc<Ctx>, repo: String, run: u64, action: RunAction) {
    let _b = Busy::new(&ctx.busy);
    let c = &ctx.client;
    let res = match action {
        RunAction::RerunFailed => c.rerun(&repo, run, true).await,
        RunAction::RerunAll => c.rerun(&repo, run, false).await,
        RunAction::Cancel => c.cancel(&repo, run).await,
    };
    match res {
        Ok(()) => {
            let verb = match action {
                RunAction::Cancel => "cancel",
                _ => "re-run",
            };
            ctx.send(Msg::Info(format!("{verb} requested for run {run}")));
            tokio::time::sleep(Duration::from_secs(3)).await;
            repo_status(&ctx, &repo, false).await;
        }
        Err(e) => ctx.err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as D, Utc};
    use serde_json::json;

    #[test]
    fn polls_only_repos_that_may_have_changed() {
        let now = Utc::now();
        let old = Some(now - D::days(3));
        // Idle repo with runs loaded: wait for the slow refresh.
        assert!(!should_poll(old, old, true, false, now));
        // Never loaded, active runs, a new push, or a push within the window: poll.
        assert!(should_poll(old, old, false, false, now));
        assert!(should_poll(old, old, true, true, now));
        assert!(should_poll(Some(now - D::days(1)), old, true, false, now));
        assert!(should_poll(Some(now - D::minutes(5)), Some(now - D::minutes(5)), true, false, now));
        assert!(!should_poll(Some(now - D::minutes(30)), Some(now - D::minutes(30)), true, false, now));
    }

    #[test]
    fn run_signature_ignores_ci_progress_but_tracks_deploys() {
        let run = |id: u64, name: &str, status: &str| -> WorkflowRun {
            let conclusion = if status == "completed" { json!("success") } else { json!(null) };
            serde_json::from_value(json!({
                "id": id, "name": name, "status": status, "conclusion": conclusion, "html_url": "",
                "run_number": 1, "event": "push", "created_at": "2026-09-01T00:00:00Z",
                "updated_at": "2026-09-01T00:00:00Z"
            }))
            .unwrap()
        };
        let tiers: Vec<crate::config::Tier> =
            vec![toml::from_str("name = 'prod'\nworkflows = ['*deploy*prod*']").unwrap()];
        let sig = |runs: &[WorkflowRun]| run_signature(runs, &tiers);
        // CI moving from queued to running does not trigger a deploy refetch; finishing does.
        assert_eq!(sig(&[run(1, "CI", "queued")]), sig(&[run(1, "CI", "in_progress")]));
        assert_ne!(sig(&[run(1, "CI", "in_progress")]), sig(&[run(1, "CI", "completed")]));
        // Deploy workflows are tracked through every state.
        assert_ne!(sig(&[run(2, "Deploy prod", "queued")]), sig(&[run(2, "Deploy prod", "in_progress")]));
    }
}
