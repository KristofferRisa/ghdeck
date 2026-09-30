use crate::github::models::{Time, WorkflowRun};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DState {
    Success,
    Failure,
    Running,
    Pending,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Environment,
    Workflow,
    Release,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub source: Source,
    pub label: String,
    pub git_ref: String,
    pub state: DState,
    pub at: Time,
    pub url: String,
    pub actor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TierStatus {
    pub tier: String,
    pub entries: Vec<Entry>,
}

impl TierStatus {
    pub fn latest(&self) -> Option<&Entry> {
        self.entries.iter().max_by_key(|e| e.at)
    }
}

/// Case-insensitive glob with `*` and `?`.
pub fn glob(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn run_state(status: Option<&str>, conclusion: Option<&str>) -> DState {
    match (status, conclusion) {
        (Some("completed"), Some("success")) => DState::Success,
        (Some("completed"), Some("failure" | "timed_out" | "startup_failure")) => DState::Failure,
        (Some("completed"), Some("cancelled" | "skipped" | "neutral" | "stale")) => DState::Inactive,
        (Some("completed"), _) => DState::Unknown,
        (Some("in_progress"), _) => DState::Running,
        (Some(_), _) => DState::Pending,
        _ => DState::Unknown,
    }
}

pub fn deployment_state(s: &str) -> DState {
    match s {
        "success" => DState::Success,
        "failure" | "error" => DState::Failure,
        "in_progress" => DState::Running,
        "queued" | "pending" | "waiting" => DState::Pending,
        "inactive" => DState::Inactive,
        _ => DState::Unknown,
    }
}

pub fn run_matches(patterns: &[String], run: &WorkflowRun) -> bool {
    patterns.iter().any(|p| {
        glob(p, run.workflow())
            || run.path.as_deref().is_some_and(|path| glob(p, crate::github::models::file_name(path)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globbing() {
        assert!(glob("prod", "Prod"));
        assert!(glob("*deploy*prod*", "Deploy to production"));
        assert!(glob("prod-*", "prod-westeurope"));
        assert!(!glob("prod", "production"));
        assert!(glob("te?t", "test"));
        assert!(!glob("*deploy*test*", "Build and test"));
    }
}
