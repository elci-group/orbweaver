//! ELCI tool discovery (directive sections 26–27): connectors discover
//! their actual interface at runtime rather than encoding assumptions
//! like "Kaptaind has command X" — and, as of this module, discover
//! *which tools exist at all* from the real GitHub org rather than a
//! hardcoded name list. For each repository the org actually has, that
//! we already know locally, and that has opted in via a `.orb` file
//! (see [`orb`]), this probes whether a binary exists and — if `.orb`
//! allows it — acquires one on demand when it doesn't (a matching
//! GitHub release binary preferred, a `baby`-built source binary as
//! fallback; see [`install`]).
//!
//! Failure philosophy (directive section 33): a tool that isn't
//! installed is `Unavailable` with a reason, never silently reported as
//! having no capabilities — those are different claims.

pub mod github;
pub mod install;
pub mod orb;
mod probe;
mod process;

pub use github::{GithubRepo, ELCI_GITHUB_ORG};
pub use install::{InstallMethod, InstallOutcome};
pub use orb::{OrbFile, OrbInstall, OrbTool, ORB_FILENAME};
pub use probe::{ConnectorDetails, ConnectorReport, DiscoveredCommand, DiscoveryMethod};

use orbweaver_evidence::Availability;
use std::collections::HashMap;
use std::path::PathBuf;

/// Discover ELCI connectors: query `org` on GitHub for its real
/// repository list, then for each one we already know locally (from
/// `repo_paths`, typically the last `orbweaver scan`'s results) and that
/// carries a `.orb` file opting it in, probe its binaries — acquiring
/// one on demand first if `.orb` marks it installable and none is found.
///
/// Everything else is silently out of scope: a repo the org has but
/// we've never cloned can't be checked for `.orb` without cloning it
/// speculatively (not done here — see `orb` module docs), and a repo
/// without `.orb` at all hasn't opted in to being an Orbweaver
/// connector, full stop. `on_progress` is called with each repo name as
/// it's considered, for callers that want to show live progress across
/// what can be a slow pass (JIT installs run inline).
///
/// `allow_install` gates whether a missing-but-`.orb`-installable tool
/// gets acquired right now: a routine health check (`orbweaver doctor`)
/// should never have the side effect of cloning and building software,
/// so it passes `false` and just reports what's missing; `orbweaver
/// integrations` passes `true`.
pub fn discover_and_probe(
    org: &str,
    repo_paths: &HashMap<String, PathBuf>,
    allow_install: bool,
    mut on_progress: impl FnMut(&str),
) -> Result<Vec<ConnectorReport>, String> {
    let repos = github::discover_org_repos(org)?;
    let cache_dir = install::default_cache_dir();

    let mut reports = Vec::new();
    for repo in &repos {
        let Some(repo_path) = repo_paths.get(&repo.name) else {
            continue;
        };
        let Some(orb) = orb::load_local(repo_path) else {
            continue;
        };

        on_progress(&repo.name);

        let candidates = candidate_binary_names(&repo.name, &orb.tool.binaries);
        let mut tool_reports = probe::probe_candidates(&repo.name, &candidates, Some(repo_path));

        let all_missing = tool_reports
            .iter()
            .all(|r| matches!(r.availability, Availability::Unavailable { .. }));

        if all_missing && allow_install && orb.install.installable {
            let primary_binary = candidates.first().cloned().unwrap_or_else(|| repo.name.clone());
            tool_reports = match install::jit_install(
                org,
                &repo.name,
                &primary_binary,
                orb.install.prefer_release_binary,
                &repo.ssh_url,
                &cache_dir,
                Some(repo_path.as_path()),
            ) {
                Ok(_outcome) => probe::probe_candidates(&repo.name, &candidates, Some(repo_path)),
                Err(e) => vec![ConnectorReport {
                    tool: repo.name.clone(),
                    binary: primary_binary,
                    availability: Availability::Unavailable {
                        reason: format!("not installed, and on-demand install failed: {e}"),
                    },
                }],
            };
        }

        reports.extend(tool_reports);
    }

    Ok(reports)
}

/// Candidate binary names for a repo, in priority order: `.orb`'s
/// explicit hints first (if any — these come from the project's own
/// declaration, the most authoritative source), then the repo name and
/// its `-cli` companion (the pattern kaptaind actually uses in this
/// estate). [`probe::probe_candidates`] deduplicates and tries all of
/// them, not just the first match.
fn candidate_binary_names(repo_name: &str, orb_hints: &[String]) -> Vec<String> {
    let mut names: Vec<String> = orb_hints.to_vec();
    names.push(repo_name.to_string());
    names.push(format!("{repo_name}-cli"));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_names_put_orb_hints_first() {
        let names = candidate_binary_names("kaptaind", &["kaptaind-daemon".to_string()]);
        assert_eq!(names, vec!["kaptaind-daemon", "kaptaind", "kaptaind-cli"]);
    }

    #[test]
    fn candidate_names_fall_back_to_repo_name_pattern_with_no_orb_hints() {
        let names = candidate_binary_names("padagonia", &[]);
        assert_eq!(names, vec!["padagonia", "padagonia-cli"]);
    }
}
