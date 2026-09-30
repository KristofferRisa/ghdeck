//! Finds GitHub repos that are cloned on disk, so the watch list can follow what you work on.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Expands a leading `~` to the home directory.
pub fn expand_home(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest.trim_start_matches('/')))
            .unwrap_or_else(|| PathBuf::from(p)),
        _ => PathBuf::from(p),
    }
}

/// Scans each directory (and one level below folders that are not repos themselves, which
/// catches worktree folders) for clones whose remote points at `host`. Hidden folders are
/// skipped. Returns `owner/repo` → first local path, sorted by name.
pub fn discover(dirs: &[String], host: &str) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    for d in dirs {
        let root = expand_home(d);
        for dir in children(&root) {
            if dir.join(".git").exists() {
                add(&mut out, &dir, host);
            } else {
                for sub in children(&dir) {
                    add(&mut out, &sub, host);
                }
            }
        }
    }
    out
}

fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}

fn add(out: &mut BTreeMap<String, PathBuf>, dir: &Path, host: &str) {
    let Some(cfg) = git_config(dir) else { return };
    let Ok(text) = fs::read_to_string(cfg) else { return };
    for url in remote_urls(&text) {
        if let Some((h, name)) = parse_remote(&url)
            && h.eq_ignore_ascii_case(host)
        {
            out.entry(name).or_insert_with(|| dir.to_path_buf());
        }
    }
}

/// Path of the git config for a clone or a linked worktree (`.git` file → commondir).
fn git_config(dir: &Path) -> Option<PathBuf> {
    let dot = dir.join(".git");
    if dot.is_dir() {
        return Some(dot.join("config"));
    }
    let text = fs::read_to_string(&dot).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let gitdir = if gitdir.is_absolute() { gitdir } else { dir.join(gitdir) };
    let common = fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .map(|c| {
            let c = PathBuf::from(c.trim());
            if c.is_absolute() { c } else { gitdir.join(c) }
        })
        .unwrap_or(gitdir);
    Some(common.join("config"))
}

/// Remote URLs from a git config, `origin` first.
pub fn remote_urls(config: &str) -> Vec<String> {
    let mut section = String::new();
    let mut v: Vec<(bool, String)> = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.to_string();
        } else if section.starts_with("[remote ")
            && let Some(url) = line.strip_prefix("url").map(str::trim_start).and_then(|l| l.strip_prefix('='))
        {
            v.push((section != "[remote \"origin\"]", url.trim().to_string()));
        }
    }
    v.sort_by_key(|(not_origin, _)| *not_origin);
    v.into_iter().map(|(_, u)| u).collect()
}

/// Parses `git@host:owner/repo.git`, `ssh://user@host:22/owner/repo` and
/// `https://host/owner/repo` into `(host, "owner/repo")`.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?.to_string(), path.to_string())
    } else {
        let rest = url.rsplit_once('@').map_or(url, |(_, r)| r);
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    let (owner, repo) = (parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((host.to_lowercase(), format!("{owner}/{repo}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_urls() {
        let want = Some(("acme.ghe.com".to_string(), "acme/lens".to_string()));
        assert_eq!(parse_remote("git@acme.ghe.com:acme/lens.git"), want);
        assert_eq!(parse_remote("git@acme.ghe.com:acme/lens"), want);
        assert_eq!(parse_remote("https://acme.ghe.com/acme/lens.git"), want);
        assert_eq!(parse_remote("ssh://git@acme.ghe.com:22/acme/lens.git"), want);
        assert_eq!(parse_remote("https://token@ACME.ghe.com/acme/lens/"), want);
        // Azure DevOps paths are deeper than owner/repo and never match a GitHub host anyway.
        assert_eq!(parse_remote("https://acme@dev.azure.com/acme/Acme/_git/x"), None);
    }

    #[test]
    fn reads_remotes_origin_first() {
        let cfg = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = git@github.com:a/b.git\n\
                   [remote \"origin\"]\n\turl = git@github.com:me/b.git\n\tfetch = +refs/heads/*\n";
        assert_eq!(remote_urls(cfg), vec!["git@github.com:me/b.git", "git@github.com:a/b.git"]);
    }

    #[test]
    fn discovers_clones_and_worktrees() {
        let root = std::env::temp_dir().join(format!("ghdeck-local-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let write = |p: PathBuf, s: &str| {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, s).unwrap();
        };
        write(root.join("app/.git/config"), "[remote \"origin\"]\n\turl = git@h.ghe.com:ORG/app.git\n");
        write(root.join("other/.git/config"), "[remote \"origin\"]\n\turl = git@github.com:x/y.git\n");
        // A worktree folder holding linked worktrees of a clone.
        write(root.join("app-wt/feat/.git"), &format!("gitdir: {}\n", root.join("app/.git/worktrees/feat").display()));
        write(root.join("app/.git/worktrees/feat/commondir"), "../..\n");
        write(root.join("lib-wt/lib/.git/config"), "[remote \"origin\"]\n\turl = https://h.ghe.com/ORG/lib\n");
        write(root.join(".hidden/.git/config"), "[remote \"origin\"]\n\turl = git@h.ghe.com:ORG/hidden.git\n");

        let found = discover(&[root.display().to_string()], "h.ghe.com");
        assert_eq!(found.keys().collect::<Vec<_>>(), vec!["ORG/app", "ORG/lib"]);
        assert_eq!(found["ORG/app"], root.join("app"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn expands_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_home("~/code"), PathBuf::from(&home).join("code"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
