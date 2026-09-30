use chrono::{DateTime, Utc};
use serde::Deserialize;

pub type Time = DateTime<Utc>;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct User {
    pub login: String,
    pub name: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Owner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Repo {
    pub full_name: String,
    pub name: String,
    pub owner: Owner,
    pub description: Option<String>,
    pub html_url: String,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
    pub stargazers_count: u64,
    pub forks_count: u64,
    pub open_issues_count: u64,
    pub language: Option<String>,
    pub default_branch: String,
    pub pushed_at: Option<Time>,
    pub created_at: Option<Time>,
    pub topics: Vec<String>,
    pub visibility: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_title: Option<String>,
    #[serde(default)]
    pub head_branch: Option<String>,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: String,
    pub created_at: Time,
    pub updated_at: Time,
    #[serde(default)]
    pub run_started_at: Option<Time>,
    #[serde(default)]
    pub run_number: u64,
    #[serde(default)]
    pub run_attempt: Option<u64>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub actor: Option<User>,
    #[serde(skip)]
    pub repo: String,
}

impl WorkflowRun {
    pub fn workflow(&self) -> &str {
        self.name.as_deref().or_else(|| self.path.as_deref().map(file_name)).unwrap_or("workflow")
    }
    pub fn title(&self) -> &str {
        self.display_title.as_deref().unwrap_or("")
    }
    pub fn branch(&self) -> &str {
        self.head_branch.as_deref().unwrap_or("")
    }
    pub fn is_active(&self) -> bool {
        self.status.as_deref() != Some("completed")
    }
    pub fn duration(&self) -> chrono::Duration {
        let start = self.run_started_at.unwrap_or(self.created_at);
        let end = if self.is_active() { Utc::now() } else { self.updated_at };
        end - start
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RunsPage {
    pub workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<Time>,
    #[serde(default)]
    pub completed_at: Option<Time>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub started_at: Option<Time>,
    #[serde(default)]
    pub completed_at: Option<Time>,
}

pub fn span(start: Option<Time>, end: Option<Time>) -> Option<chrono::Duration> {
    let s = start?;
    Some(end.unwrap_or_else(Utc::now) - s)
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct JobsPage {
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deployment {
    pub id: u64,
    #[serde(default)]
    pub environment: String,
    #[serde(rename = "ref", default)]
    pub git_ref: String,
    pub created_at: Time,
    #[serde(default)]
    pub creator: Option<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentStatus {
    #[serde(default)]
    pub state: String,
    pub created_at: Time,
    #[serde(default)]
    pub environment_url: Option<String>,
    #[serde(default)]
    pub log_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub published_at: Option<Time>,
    pub created_at: Time,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub author: Option<User>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Label {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub comments: u64,
    pub created_at: Time,
    pub updated_at: Time,
    #[serde(default)]
    pub repository_url: String,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    #[serde(default)]
    pub draft: Option<bool>,
    #[serde(default)]
    pub body: Option<String>,
}

impl Issue {
    pub fn repo(&self) -> String {
        let mut parts = self.repository_url.rsplit('/');
        let name = parts.next().unwrap_or_default();
        let owner = parts.next().unwrap_or_default();
        format!("{owner}/{name}")
    }
    pub fn is_pr(&self) -> bool {
        self.pull_request.is_some()
    }
    pub fn author(&self) -> &str {
        self.user.as_ref().map(|u| u.login.as_str()).unwrap_or("")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Search<T> {
    #[serde(default)]
    pub total_count: u64,
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notification {
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub unread: bool,
    pub updated_at: Time,
    pub subject: Subject,
    pub repository: NotifRepo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Subject {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotifRepo {
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Participation {
    pub all: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Calendar {
    #[serde(rename = "totalContributions", default)]
    pub total: u64,
    #[serde(default)]
    pub weeks: Vec<Week>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Week {
    #[serde(rename = "contributionDays", default)]
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Day {
    #[serde(rename = "contributionCount", default)]
    pub count: u32,
    #[serde(default)]
    pub weekday: u8,
}

#[derive(Debug, Clone, Default)]
pub struct RepoDetail {
    pub languages: Vec<(String, u64)>,
    pub weekly: Vec<u32>,
    pub releases: Vec<Release>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rate {
    pub limit: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rates {
    pub core: Option<Rate>,
    pub search: Option<Rate>,
}
