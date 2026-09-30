//! Counters for API traffic and render work, so the cost of running ghdeck can be measured
//! (stats overlay `S`, `ghdeck bench`, and the periodic summary in the log file).

use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Which quota a request is charged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quota {
    Core,
    Search,
    Graphql,
    Free,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stat {
    pub calls: u64,
    pub not_modified: u64,
    pub errors: u64,
    pub bytes: u64,
    pub millis: u64,
}

impl Stat {
    /// Requests that used up rate limit (304s are free).
    pub fn cost(&self) -> u64 {
        self.calls - self.not_modified
    }
}

#[derive(Default)]
pub struct Metrics {
    by_endpoint: Mutex<BTreeMap<(Quota, String), Stat>>,
}

impl Metrics {
    pub fn record(&self, method: &str, path: &str, status: u16, bytes: usize, took: Duration) {
        let (quota, kind) = classify(method, path);
        let mut m = self.by_endpoint.lock().unwrap();
        let s = m.entry((quota, kind)).or_default();
        s.calls += 1;
        s.not_modified += u64::from(status == 304);
        s.errors += u64::from(status >= 400 || status == 0);
        s.bytes += bytes as u64;
        s.millis += took.as_millis() as u64;
    }

    pub fn snapshot(&self) -> Vec<(Quota, String, Stat)> {
        self.by_endpoint.lock().unwrap().iter().map(|((q, k), s)| (*q, k.clone(), *s)).collect()
    }

    pub fn total(&self, quota: Quota) -> Stat {
        let m = self.by_endpoint.lock().unwrap();
        m.iter().filter(|((q, _), _)| *q == quota).fold(Stat::default(), |mut a, (_, s)| {
            a.calls += s.calls;
            a.not_modified += s.not_modified;
            a.errors += s.errors;
            a.bytes += s.bytes;
            a.millis += s.millis;
            a
        })
    }
}

/// Maps a request to its quota and a path template without owner, repo or ids, e.g.
/// `/repos/acme/x/actions/runs/42/jobs?per_page=100` → `/repos/:repo/actions/runs/:id/jobs`.
pub fn classify(method: &str, path: &str) -> (Quota, String) {
    let path = path.split('?').next().unwrap_or(path);
    let path = path.find("://").map_or(path, |i| {
        let rest = &path[i + 3..];
        rest.find('/').map_or("/", |j| &rest[j..])
    });
    let path = path.strip_prefix("/api/v3").unwrap_or(path);
    if path.ends_with("graphql") {
        return (Quota::Graphql, "graphql".into());
    }
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::with_capacity(seg.len());
    let mut i = 0;
    while i < seg.len() {
        let s = seg[i];
        match (i, s) {
            (0, "repos") if seg.len() >= 3 => {
                out.push("repos");
                out.push(":repo");
                i += 3;
                continue;
            }
            (1, _) if matches!(seg[0], "orgs" | "users") => out.push(":owner"),
            _ if s.chars().all(|c| c.is_ascii_digit()) => out.push(":id"),
            _ => out.push(s),
        }
        i += 1;
    }
    let kind = format!("{} /{}", method, out.join("/"));
    let quota = match seg.first() {
        Some(&"search") => Quota::Search,
        Some(&"rate_limit") => Quota::Free,
        _ => Quota::Core,
    };
    (quota, kind)
}

// ── Render and process counters ─────────────────────────────────────────────

pub static DRAWS: AtomicU64 = AtomicU64::new(0);
pub static DRAW_MICROS: AtomicU64 = AtomicU64::new(0);

pub fn record_draw(took: Duration) {
    DRAWS.fetch_add(1, Ordering::Relaxed);
    DRAW_MICROS.fetch_add(took.as_micros() as u64, Ordering::Relaxed);
}

/// CPU seconds used by this process and its resident memory in bytes (Linux only).
pub fn process() -> Option<(f64, u64)> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the parenthesised command name; utime and stime are fields 14 and 15.
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks: u64 = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rss_kb: u64 = status
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())?;
    // USER_HZ is 100 on every mainstream Linux configuration.
    Some((ticks as f64 / 100.0, rss_kb * 1024))
}

/// A point-in-time reading used to compute rates over an interval.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub at: Instant,
    pub core: u64,
    pub search: u64,
    pub graphql: u64,
    pub calls: u64,
    pub not_modified: u64,
    pub draws: u64,
    pub draw_micros: u64,
    pub cpu: f64,
    pub rss: u64,
}

impl Sample {
    pub fn take<'a>(ms: impl IntoIterator<Item = &'a Metrics>) -> Self {
        let ms: Vec<&Metrics> = ms.into_iter().collect();
        let all = [Quota::Core, Quota::Search, Quota::Graphql, Quota::Free].map(|q| {
            ms.iter().map(|m| m.total(q)).fold(Stat::default(), |mut a, s| {
                a.calls += s.calls;
                a.not_modified += s.not_modified;
                a
            })
        });
        let (cpu, rss) = process().unwrap_or((0.0, 0));
        Self {
            at: Instant::now(),
            core: all[0].cost(),
            search: all[1].cost(),
            graphql: all[2].cost(),
            calls: all.iter().map(|s| s.calls).sum(),
            not_modified: all.iter().map(|s| s.not_modified).sum(),
            draws: DRAWS.load(Ordering::Relaxed),
            draw_micros: DRAW_MICROS.load(Ordering::Relaxed),
            cpu,
            rss,
        }
    }
}

/// Rates between two samples.
#[derive(Debug, Clone, Copy)]
pub struct Rates {
    pub secs: f64,
    pub core_per_hour: f64,
    pub search_per_min: f64,
    pub graphql_per_hour: f64,
    pub calls: u64,
    pub free_pct: f64,
    pub draws_per_sec: f64,
    pub ms_per_draw: f64,
    pub cpu_pct: f64,
    pub rss: u64,
}

pub fn rates(a: &Sample, b: &Sample) -> Rates {
    let secs = b.at.duration_since(a.at).as_secs_f64().max(0.001);
    let calls = b.calls - a.calls;
    let draws = b.draws - a.draws;
    Rates {
        secs,
        core_per_hour: (b.core - a.core) as f64 * 3600.0 / secs,
        search_per_min: (b.search - a.search) as f64 * 60.0 / secs,
        graphql_per_hour: (b.graphql - a.graphql) as f64 * 3600.0 / secs,
        calls,
        free_pct: if calls == 0 { 0.0 } else { (b.not_modified - a.not_modified) as f64 * 100.0 / calls as f64 },
        draws_per_sec: draws as f64 / secs,
        ms_per_draw: if draws == 0 { 0.0 } else { (b.draw_micros - a.draw_micros) as f64 / draws as f64 / 1000.0 },
        cpu_pct: (b.cpu - a.cpu) * 100.0 / secs,
        rss: b.rss,
    }
}

impl std::fmt::Display for Rates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "core={:.0}/h search={:.1}/min graphql={:.0}/h calls={} free={:.0}% draws={:.2}/s draw={:.2}ms cpu={:.2}% rss={:.1}MB",
            self.core_per_hour,
            self.search_per_min,
            self.graphql_per_hour,
            self.calls,
            self.free_pct,
            self.draws_per_sec,
            self.ms_per_draw,
            self.cpu_pct,
            self.rss as f64 / 1_048_576.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        assert_eq!(
            classify("GET", "https://api.x.ghe.com/repos/acme/app/actions/runs/42/jobs?per_page=100"),
            (Quota::Core, "GET /repos/:repo/actions/runs/:id/jobs".into())
        );
        assert_eq!(classify("GET", "/repos/o/r?x=1"), (Quota::Core, "GET /repos/:repo".into()));
        assert_eq!(classify("GET", "/orgs/acme/repos?sort=pushed"), (Quota::Core, "GET /orgs/:owner/repos".into()));
        assert_eq!(classify("GET", "/search/issues?q=a").0, Quota::Search);
        assert_eq!(classify("GET", "/rate_limit").0, Quota::Free);
        assert_eq!(classify("POST", "https://h/api/graphql").0, Quota::Graphql);
        assert_eq!(classify("GET", "https://h/api/v3/user"), (Quota::Core, "GET /user".into()));
    }

    #[test]
    fn not_modified_is_free() {
        let m = Metrics::default();
        m.record("GET", "/repos/a/b", 200, 10, Duration::from_millis(5));
        m.record("GET", "/repos/a/b", 304, 0, Duration::from_millis(5));
        m.record("GET", "/repos/c/d", 304, 0, Duration::from_millis(5));
        m.record("GET", "/search/issues?q=x", 200, 10, Duration::ZERO);
        let core = m.total(Quota::Core);
        assert_eq!((core.calls, core.cost()), (3, 1));
        assert_eq!(m.total(Quota::Search).cost(), 1);
    }

    #[test]
    fn rates_project_per_hour() {
        let m = Metrics::default();
        let a = Sample::take([&m]);
        for _ in 0..10 {
            m.record("GET", "/user", 200, 0, Duration::ZERO);
        }
        let mut b = Sample::take([&m]);
        b.at = a.at + Duration::from_secs(60);
        let r = rates(&a, &b);
        assert!((r.core_per_hour - 600.0).abs() < 1e-6);
        assert_eq!(r.calls, 10);
    }
}
