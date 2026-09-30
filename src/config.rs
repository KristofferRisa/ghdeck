use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

pub const TEMPLATE: &str = include_str!("../assets/config.example.toml");
pub const APP: &str = "ghdeck";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub hosts: Vec<HostConfig>,
    pub deploy: DeployConfig,
    pub sections: Sections,
    pub explore: Explore,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct General {
    pub refresh_secs: u64,
    pub fast_refresh_secs: u64,
    pub slow_refresh_secs: u64,
    pub theme: String,
    pub icons: String,
    pub transparent: bool,
    pub concurrency: usize,
    pub runs_per_repo: u32,
    pub default_host: Option<String>,
    pub editor: Option<String>,
    /// error | warn | info | debug | trace (GHDECK_LOG overrides it)
    pub log_level: String,
}

impl Default for General {
    fn default() -> Self {
        Self {
            refresh_secs: 60,
            fast_refresh_secs: 15,
            slow_refresh_secs: 600,
            theme: "tokyonight".into(),
            icons: "nerd".into(),
            transparent: false,
            concurrency: 8,
            runs_per_repo: 20,
            default_host: None,
            editor: None,
            log_level: "info".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct HostConfig {
    pub name: String,
    pub host: String,
    pub api_url: Option<String>,
    pub token_env: Option<String>,
    pub token_cmd: Option<String>,
    pub repos: Vec<String>,
    pub orgs: Vec<String>,
    /// Folders with clones (e.g. "~/code"); repos whose remote is on this host are watched.
    pub local_dirs: Vec<String>,
    pub auto_repos: u32,
    pub org_repos: u32,
    pub trending_query: Option<String>,
    pub top_query: Option<String>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            name: "github".into(),
            host: "github.com".into(),
            api_url: None,
            token_env: None,
            token_cmd: None,
            repos: vec![],
            orgs: vec![],
            local_dirs: vec![],
            auto_repos: 10,
            org_repos: 10,
            trending_query: None,
            top_query: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct DeployConfig {
    pub tiers: Vec<Tier>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Tier {
    pub name: String,
    pub environments: Vec<String>,
    pub workflows: Vec<String>,
    pub release: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Sections {
    pub pulls: Vec<Section>,
    pub issues: Vec<Section>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Section {
    pub title: String,
    pub filter: String,
    pub limit: u32,
}

impl Default for Section {
    fn default() -> Self {
        Self { title: String::new(), filter: String::new(), limit: 30 }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Explore {
    pub trending_query: String,
    pub top_query: String,
    pub limit: u32,
}

impl Default for Explore {
    fn default() -> Self {
        Self { trending_query: "created:>{since:7d} stars:>5".into(), top_query: "stars:>1000".into(), limit: 30 }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP)
}

/// Log folder: $XDG_STATE_HOME/ghdeck, else ~/.local/state/ghdeck.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join(APP)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Writes the commented template if no config exists. Returns true when a file was written.
pub fn ensure(path: &PathBuf, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    fs::write(path, TEMPLATE).with_context(|| format!("write {}", path.display()))?;
    Ok(true)
}

pub fn load(path: &PathBuf) -> Result<Config> {
    ensure(path, false)?;
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut cfg: Config = toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    if cfg.hosts.is_empty() {
        cfg.hosts.push(HostConfig::default());
    }
    for (i, h) in cfg.hosts.iter_mut().enumerate() {
        if h.name.is_empty() {
            h.name = if i == 0 { h.host.clone() } else { format!("{}-{i}", h.host) };
        }
    }
    cfg.general.concurrency = cfg.general.concurrency.clamp(1, 32);
    cfg.general.refresh_secs = cfg.general.refresh_secs.max(10);
    cfg.general.fast_refresh_secs = cfg.general.fast_refresh_secs.max(5);
    Ok(cfg)
}

/// Expands `{since:7d}` style tokens into an ISO date (units: d, w, m).
pub fn expand_query(q: &str) -> String {
    let mut out = String::new();
    let mut rest = q;
    while let Some(start) = rest.find("{since:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 7..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let spec = &after[..end];
        let (num, unit) = spec.split_at(spec.len().saturating_sub(1));
        let n: i64 = num.parse().unwrap_or(7);
        let days = match unit {
            "w" => n * 7,
            "m" => n * 30,
            _ => n,
        };
        out.push_str(&(Utc::now() - Duration::days(days)).format("%Y-%m-%d").to_string());
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Per-user state that ghdeck writes itself (watch list, theme), kept apart from the
/// hand-edited config so comments there survive.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct StateFile {
    pub watch: HashMap<String, Vec<String>>,
    pub theme: Option<String>,
    pub host: Option<String>,
}

impl StateFile {
    fn path() -> PathBuf {
        config_dir().join("state.json")
    }

    pub fn load() -> Self {
        fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let cfg: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(cfg.deploy.tiers.len(), 2);
        assert_eq!(cfg.sections.pulls.len(), 3);
        assert_eq!(cfg.hosts[0].host, "github.com");
    }

    #[test]
    fn expands_since() {
        let q = expand_query("created:>{since:7d} stars:>5");
        assert!(q.starts_with("created:>20"));
        assert!(q.ends_with(" stars:>5"));
        assert_eq!(expand_query("plain"), "plain");
    }
}
