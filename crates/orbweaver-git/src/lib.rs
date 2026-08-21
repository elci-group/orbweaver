//! Deterministic git history inspection for a single repository path.
//!
//! Everything here is Tier 0 (directive section 16): no network access, no
//! inference — just walking the object database that's already on disk.

use chrono::{DateTime, TimeZone, Utc};
use git2::Repository as GitRepo;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub default_branch: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub commit_count: Option<u64>,
    pub contributor_count: Option<u64>,
}

/// Walk HEAD's first-parent-inclusive history to gather commit count,
/// distinct author count, and the timestamp of the most recent commit.
///
/// `max_commits` bounds the revwalk so a pathologically large history
/// (or a home-directory-scale monorepo) can't stall a scan; when the cap
/// is hit the counts are a lower bound, not a claim of exact history size.
pub fn inspect(path: &Path, max_commits: usize) -> Option<GitInfo> {
    let repo = GitRepo::open(path).ok()?;

    let default_branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(String::from));

    let mut revwalk = repo.revwalk().ok()?;
    if revwalk.push_head().is_err() {
        // Repo exists but has no commits yet.
        return Some(GitInfo {
            default_branch,
            ..Default::default()
        });
    }

    let mut commit_count: u64 = 0;
    let mut authors: HashSet<String> = HashSet::new();
    let mut last_commit_at: Option<DateTime<Utc>> = None;

    for oid in revwalk.take(max_commits) {
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };

        commit_count += 1;

        if let Some(email) = commit.author().email() {
            authors.insert(email.to_string());
        } else if let Some(name) = commit.author().name() {
            authors.insert(name.to_string());
        }

        let ts = commit.time();
        if let Some(dt) = Utc.timestamp_opt(ts.seconds(), 0).single() {
            if last_commit_at.is_none_or(|prev| dt > prev) {
                last_commit_at = Some(dt);
            }
        }
    }

    Some(GitInfo {
        default_branch,
        last_commit_at,
        commit_count: Some(commit_count),
        contributor_count: Some(authors.len() as u64),
    })
}

pub fn is_git_repo(path: &Path) -> bool {
    GitRepo::open(path).is_ok()
}
