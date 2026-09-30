pub mod models;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{StatusCode, header::HeaderMap};
use serde::{Deserialize, de::DeserializeOwned};
use tokio::sync::OnceCell;

use crate::{config::HostConfig, metrics::Metrics};
use models::*;

type Body = Arc<[u8]>;

/// Validators and body of a cached response.
#[derive(Clone)]
struct Cached {
    etag: Option<String>,
    last_modified: Option<String>,
    body: Body,
}

/// A REST + GraphQL client for one GitHub host. Responses are cached by ETag / Last-Modified
/// so that unchanged resources come back as 304s, which do not count against the rate limit.
pub struct Client {
    pub host: String,
    pub api: String,
    pub graphql: String,
    pub web: String,
    http: reqwest::Client,
    token_env: Option<String>,
    token_cmd: Option<String>,
    token: OnceCell<String>,
    cache: Mutex<HashMap<String, Cached>>,
    rates: Mutex<Rates>,
    pub metrics: Metrics,
}

pub fn endpoints(host: &str, api_override: Option<&str>) -> (String, String, String) {
    let web = format!("https://{host}");
    if let Some(api) = api_override {
        let api = api.trim_end_matches('/').to_string();
        let gql = match api.strip_suffix("/api/v3") {
            Some(base) => format!("{base}/api/graphql"),
            None => format!("{api}/graphql"),
        };
        return (api, gql, web);
    }
    if host == "github.com" {
        ("https://api.github.com".into(), "https://api.github.com/graphql".into(), web)
    } else if host.ends_with(".ghe.com") {
        (format!("https://api.{host}"), format!("https://api.{host}/graphql"), web)
    } else {
        (format!("https://{host}/api/v3"), format!("https://{host}/api/graphql"), web)
    }
}

pub fn enc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

async fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(program).args(args).output().await.ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

async fn resolve_token(host: &str, token_env: Option<&str>, token_cmd: Option<&str>) -> Result<String> {
    let from_env = |k: &str| std::env::var(k).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if let Some(t) = token_env.and_then(from_env) {
        return Ok(t);
    }
    if let Some(cmd) = token_cmd {
        #[cfg(windows)]
        let t = run_capture("cmd", &["/C", cmd]).await;
        #[cfg(not(windows))]
        let t = run_capture("sh", &["-c", cmd]).await;
        if let Some(t) = t {
            return Ok(t);
        }
    }
    if let Some(t) = run_capture("gh", &["auth", "token", "--hostname", host]).await {
        return Ok(t);
    }
    let fallbacks: &[&str] = if host == "github.com" {
        &["GH_TOKEN", "GITHUB_TOKEN"]
    } else {
        &["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
    };
    fallbacks
        .iter()
        .find_map(|k| from_env(k))
        .ok_or_else(|| anyhow!("{host}: no token (run `gh auth login -h {host}` or set token_env)"))
}

impl Client {
    pub fn new(h: &HostConfig) -> Result<Self> {
        let (api, graphql, web) = endpoints(&h.host, h.api_url.as_deref());
        let http = reqwest::Client::builder()
            .user_agent(concat!("ghdeck/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("build http client")?;
        Ok(Self {
            host: h.host.clone(),
            api,
            graphql,
            web,
            http,
            token_env: h.token_env.clone(),
            token_cmd: h.token_cmd.clone(),
            token: OnceCell::new(),
            cache: Mutex::new(HashMap::new()),
            rates: Mutex::new(Rates::default()),
            metrics: Metrics::default(),
        })
    }

    pub fn rates(&self) -> Rates {
        *self.rates.lock().unwrap()
    }

    async fn token(&self) -> Result<String> {
        self.token
            .get_or_try_init(|| resolve_token(&self.host, self.token_env.as_deref(), self.token_cmd.as_deref()))
            .await
            .cloned()
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http") { path.to_string() } else { format!("{}{}", self.api, path) }
    }

    fn track_rate(&self, h: &HeaderMap) {
        let num = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok());
        let (Some(limit), Some(remaining), Some(_)) =
            (num("x-ratelimit-limit"), num("x-ratelimit-remaining"), num("x-ratelimit-reset"))
        else {
            return;
        };
        let rate = Rate { limit: limit as u64, remaining: remaining as u64 };
        let mut r = self.rates.lock().unwrap();
        match h.get("x-ratelimit-resource").and_then(|v| v.to_str().ok()) {
            Some("search") => r.search = Some(rate),
            Some("core") | None => r.core = Some(rate),
            _ => {}
        }
    }

    fn describe(&self, status: StatusCode, headers: &HeaderMap, body: &[u8], path: &str) -> String {
        #[derive(Deserialize)]
        struct Msg {
            message: Option<String>,
        }
        let msg = serde_json::from_slice::<Msg>(body).ok().and_then(|m| m.message).unwrap_or_default();
        let remaining = headers.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok());
        match status.as_u16() {
            401 => format!("{}: bad credentials, run `gh auth login -h {}`", self.host, self.host),
            403 | 429 if remaining == Some("0") || msg.contains("rate limit") => {
                let reset = headers
                    .get("x-ratelimit-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<i64>().ok())
                    .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                    .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
                    .unwrap_or_else(|| "?".into());
                format!("{}: rate limited until {reset}", self.host)
            }
            404 => format!("{path}: not found or no access"),
            _ => format!("{} {path}: {msg}", status.as_u16()),
        }
    }

    /// Counts a finished request and logs it at debug level.
    fn observe(&self, method: &str, url: &str, status: u16, bytes: usize, t0: Instant) {
        let took = t0.elapsed();
        self.metrics.record(method, url, status, bytes, took);
        let remaining = self.rates().core.map(|r| r.remaining);
        tracing::debug!(
            host = %self.host, %method, path = %url.strip_prefix(&self.api).unwrap_or(url),
            status, bytes, ms = took.as_millis() as u64, remaining, "http"
        );
    }

    /// GET with ETag / Last-Modified revalidation. Returns the status (202 for stats still
    /// computing, 304 when the cached body is still current) and the body.
    pub async fn get_raw(&self, path: &str) -> Result<(u16, Body)> {
        let url = self.url(path);
        let token = self.token().await?;
        let cached = self.cache.lock().unwrap().get(&url).cloned();
        let mut req = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        // Prefer Last-Modified: with gzip, GitHub's weak ETags rarely match (measured on
        // ghe.com: /repos/x and /user only answer 304 to If-Modified-Since), and when both
        // headers are sent If-None-Match wins, so a mismatching ETag spoils the 304.
        match cached.as_ref().map(|c| (&c.last_modified, &c.etag)) {
            Some((Some(lm), _)) => req = req.header("If-Modified-Since", lm.as_str()),
            Some((None, Some(etag))) => req = req.header("If-None-Match", etag.as_str()),
            _ => {}
        }
        let t0 = Instant::now();
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                self.observe("GET", &url, 0, 0, t0);
                return Err(anyhow!(e).context(format!("GET {url}")));
            }
        };
        let headers = resp.headers().clone();
        self.track_rate(&headers);
        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED
            && let Some(c) = cached
        {
            self.observe("GET", &url, 304, 0, t0);
            return Ok((304, c.body));
        }
        let body: Body = resp.bytes().await.with_context(|| format!("read {url}"))?.to_vec().into();
        self.observe("GET", &url, status.as_u16(), body.len(), t0);
        if !status.is_success() {
            let msg = self.describe(status, &headers, &body, path);
            if matches!(status.as_u16(), 403 | 429) {
                tracing::warn!(host = %self.host, status = status.as_u16(), "{msg}");
            }
            bail!(msg);
        }
        if status == StatusCode::OK {
            let header = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            let (etag, last_modified) = (header("etag"), header("last-modified"));
            if etag.is_some() || last_modified.is_some() {
                self.cache.lock().unwrap().insert(url, Cached { etag, last_modified, body: body.clone() });
            }
        }
        Ok((status.as_u16(), body))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let (_, body) = self.get_raw(path).await?;
        serde_json::from_slice(&body).with_context(|| format!("decode {path}"))
    }

    pub async fn get_opt<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        let (status, body) = self.get_raw(path).await?;
        if status == 202 || status == 204 || body.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&body).with_context(|| format!("decode {path}"))?))
    }

    pub async fn post(&self, path: &str) -> Result<()> {
        let url = self.url(path);
        let token = self.token().await?;
        let t0 = Instant::now();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        self.track_rate(&headers);
        self.observe("POST", &url, status.as_u16(), 0, t0);
        tracing::info!(host = %self.host, path, status = status.as_u16(), "run action");
        if !status.is_success() {
            let body = resp.bytes().await.unwrap_or_default();
            bail!(self.describe(status, &headers, &body, path));
        }
        Ok(())
    }

    pub async fn graphql<T: DeserializeOwned>(&self, query: &str) -> Result<T> {
        #[derive(Deserialize)]
        struct Resp<T> {
            data: Option<T>,
            errors: Option<Vec<GqlErr>>,
        }
        #[derive(Deserialize)]
        struct GqlErr {
            message: String,
        }
        let token = self.token().await?;
        let t0 = Instant::now();
        let resp = self
            .http
            .post(&self.graphql)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .context("POST graphql")?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.bytes().await?;
        self.observe("POST", &self.graphql, status.as_u16(), body.len(), t0);
        if !status.is_success() {
            bail!(self.describe(status, &headers, &body, "/graphql"));
        }
        let r: Resp<T> = serde_json::from_slice(&body).context("decode graphql")?;
        match (r.data, r.errors) {
            (Some(d), _) => Ok(d),
            (None, Some(e)) => bail!("graphql: {}", e.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ")),
            _ => bail!("graphql: empty response"),
        }
    }

    // ── Endpoints ────────────────────────────────────────────────────────────

    pub async fn viewer(&self) -> Result<User> {
        self.get("/user").await
    }

    pub async fn repo(&self, full: &str) -> Result<Repo> {
        self.get(&format!("/repos/{full}")).await
    }

    pub async fn user_repos(&self, n: u32) -> Result<Vec<Repo>> {
        self.get(&format!("/user/repos?sort=pushed&per_page={}", n.clamp(1, 100))).await
    }

    pub async fn org_repos(&self, org: &str, n: u32) -> Result<Vec<Repo>> {
        self.get(&format!("/orgs/{org}/repos?sort=pushed&per_page={}", n.clamp(1, 100))).await
    }

    /// Every repo the viewer can access, most recently pushed first (for the fuzzy finder).
    pub async fn all_repos(&self, max_pages: u32) -> Result<Vec<Repo>> {
        let mut out = Vec::new();
        for page in 1..=max_pages {
            let batch: Vec<Repo> = self.get(&format!("/user/repos?sort=pushed&per_page=100&page={page}")).await?;
            let done = batch.len() < 100;
            out.extend(batch);
            if done {
                break;
            }
        }
        Ok(out)
    }

    pub async fn runs(&self, full: &str, n: u32) -> Result<Vec<WorkflowRun>> {
        let page: RunsPage = self.get(&format!("/repos/{full}/actions/runs?per_page={}", n.clamp(1, 100))).await?;
        Ok(page
            .workflow_runs
            .into_iter()
            .map(|mut r| {
                r.repo = full.to_string();
                r
            })
            .collect())
    }

    pub async fn jobs(&self, full: &str, run: u64) -> Result<Vec<Job>> {
        let page: JobsPage = self.get(&format!("/repos/{full}/actions/runs/{run}/jobs?per_page=100")).await?;
        Ok(page.jobs)
    }

    pub async fn deployments(&self, full: &str) -> Result<Vec<Deployment>> {
        self.get(&format!("/repos/{full}/deployments?per_page=100")).await
    }

    pub async fn deployment_status(&self, full: &str, id: u64) -> Result<Option<DeploymentStatus>> {
        let v: Vec<DeploymentStatus> = self.get(&format!("/repos/{full}/deployments/{id}/statuses?per_page=1")).await?;
        Ok(v.into_iter().next())
    }

    pub async fn releases(&self, full: &str) -> Result<Vec<Release>> {
        self.get(&format!("/repos/{full}/releases?per_page=15")).await
    }

    pub async fn search_issues(&self, q: &str, n: u32) -> Result<Search<Issue>> {
        self.get(&format!("/search/issues?q={}&sort=updated&order=desc&per_page={}", enc(q), n.clamp(1, 100))).await
    }

    pub async fn search_repos(&self, q: &str, n: u32) -> Result<Search<Repo>> {
        self.get(&format!("/search/repositories?q={}&sort=stars&order=desc&per_page={}", enc(q), n.clamp(1, 100))).await
    }

    pub async fn notifications(&self) -> Result<Vec<Notification>> {
        self.get("/notifications?per_page=50").await
    }

    pub async fn languages(&self, full: &str) -> Result<Vec<(String, u64)>> {
        let map: serde_json::Map<String, serde_json::Value> = self.get(&format!("/repos/{full}/languages")).await?;
        let mut v: Vec<(String, u64)> = map.into_iter().map(|(k, v)| (k, v.as_u64().unwrap_or(0))).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        Ok(v)
    }

    pub async fn participation(&self, full: &str) -> Result<Option<Participation>> {
        self.get_opt(&format!("/repos/{full}/stats/participation")).await
    }

    pub async fn refresh_rates(&self) -> Result<()> {
        #[derive(Deserialize)]
        struct R {
            resources: Res,
        }
        #[derive(Deserialize)]
        struct Res {
            core: RawRate,
            search: Option<RawRate>,
        }
        #[derive(Deserialize)]
        struct RawRate {
            limit: u64,
            remaining: u64,
        }
        let r: R = self.get("/rate_limit").await?;
        let conv = |r: RawRate| Rate { limit: r.limit, remaining: r.remaining };
        let mut rates = self.rates.lock().unwrap();
        rates.core = Some(conv(r.resources.core));
        rates.search = r.resources.search.map(conv);
        Ok(())
    }

    pub async fn contributions(&self) -> Result<Calendar> {
        #[derive(Deserialize)]
        struct D {
            viewer: V,
        }
        #[derive(Deserialize)]
        struct V {
            #[serde(rename = "contributionsCollection")]
            cc: Cc,
        }
        #[derive(Deserialize)]
        struct Cc {
            #[serde(rename = "contributionCalendar")]
            cal: Calendar,
        }
        let d: D = self
            .graphql(
                "query { viewer { contributionsCollection { contributionCalendar { totalContributions \
                 weeks { contributionDays { contributionCount weekday } } } } } }",
            )
            .await?;
        Ok(d.viewer.cc.cal)
    }

    pub async fn rerun(&self, full: &str, run: u64, failed_only: bool) -> Result<()> {
        let suffix = if failed_only { "rerun-failed-jobs" } else { "rerun" };
        self.post(&format!("/repos/{full}/actions/runs/{run}/{suffix}")).await
    }

    pub async fn cancel(&self, full: &str, run: u64) -> Result<()> {
        self.post(&format!("/repos/{full}/actions/runs/{run}/cancel")).await
    }

    /// Maps an API URL (e.g. a notification subject) to its web page.
    pub fn web_url(&self, api_url: &str) -> String {
        let prefix = format!("{}/repos/", self.api);
        match api_url.strip_prefix(&prefix) {
            Some(rest) => {
                format!("{}/{}", self.web, rest.replace("/pulls/", "/pull/").replace("/commits/", "/commit/"))
            }
            None => api_url.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_derivation() {
        assert_eq!(endpoints("github.com", None).0, "https://api.github.com");
        assert_eq!(endpoints("acme.ghe.com", None).0, "https://api.acme.ghe.com");
        let (api, gql, _) = endpoints("git.corp.local", None);
        assert_eq!(api, "https://git.corp.local/api/v3");
        assert_eq!(gql, "https://git.corp.local/api/graphql");
        assert_eq!(endpoints("x", Some("https://h/api/v3/")).1, "https://h/api/graphql");
    }

    #[test]
    fn encodes_queries() {
        assert_eq!(enc("is:open author:me"), "is%3Aopen%20author%3Ame");
    }
}
