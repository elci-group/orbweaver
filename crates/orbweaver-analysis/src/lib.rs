//! Cross-repository analysis over an already-loaded snapshot (directive
//! section 13.C, and the "duplicate detection" deliverable listed under
//! Phase II). Everything here is Tier 0 (directive section 16): no
//! network access, no code execution, no LLM — just aggregating facts
//! `orbweaver-ingest` already collected.
//!
//! The first pass: independent repositories that pull in the same
//! external dependency are a *candidate* signal for shared/duplicate
//! infrastructure — not proof of it. Two repos both depending on `serde`
//! says nothing; two repos both depending on `git2` for the same kind of
//! introspection is worth a human or Tier-2 reasoning pass looking closer.
//! This module produces the candidates and their evidence trail; it does
//! not itself decide which ones matter.

use orbweaver_core::{ManifestKind, RepoId, Repository};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCandidate {
    pub dependency_name: String,
    pub manifest: ManifestKind,
    pub repositories: Vec<RepoId>,
    /// How many discovered repositories declare a manifest of this kind
    /// at all — the denominator for judging whether this dependency is a
    /// deliberate, narrow choice or just ecosystem-wide boilerplate.
    pub ecosystem_total: usize,
}

/// Find external dependencies shared by more than one repository, filtered
/// to exclude both one-offs (`< min_repos`) and dependencies so common
/// within their ecosystem that sharing them says nothing (more than
/// `max_ubiquity_fraction` of all repositories using that manifest kind —
/// e.g. the default 0.2 excludes `tokio`/`serde`-class foundational
/// crates while keeping narrower shared choices like `git2` or `ratatui`
/// visible).
pub fn shared_dependency_candidates(
    repositories: &[Repository],
    min_repos: usize,
    max_ubiquity_fraction: f64,
) -> Vec<DuplicateCandidate> {
    let mut totals_by_kind: BTreeMap<ManifestKind, usize> = BTreeMap::new();
    for repo in repositories {
        for kind in &repo.manifests {
            *totals_by_kind.entry(*kind).or_insert(0) += 1;
        }
    }

    let mut groups: BTreeMap<(String, ManifestKind), BTreeSet<RepoId>> = BTreeMap::new();
    for repo in repositories {
        for dep in &repo.dependencies {
            if dep.resolved_repo.is_some() {
                // Internal dependency — already visible as a depends_on
                // graph edge, not a duplication candidate.
                continue;
            }
            groups
                .entry((dep.name.clone(), dep.manifest))
                .or_default()
                .insert(repo.id.clone());
        }
    }

    let mut candidates: Vec<DuplicateCandidate> = groups
        .into_iter()
        .filter_map(|((name, manifest), repos)| {
            let ecosystem_total = *totals_by_kind.get(&manifest).unwrap_or(&repos.len());
            let max_repos =
                ((ecosystem_total as f64 * max_ubiquity_fraction) as usize).max(min_repos);
            if repos.len() >= min_repos && repos.len() <= max_repos {
                Some(DuplicateCandidate {
                    dependency_name: name,
                    manifest,
                    repositories: repos.into_iter().collect(),
                    ecosystem_total,
                })
            } else {
                None
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.repositories
            .len()
            .cmp(&a.repositories.len())
            .then_with(|| a.dependency_name.cmp(&b.dependency_name))
    });
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbweaver_core::DependencyRef;
    use std::path::PathBuf;

    fn repo(id: &str, manifests: Vec<ManifestKind>, deps: Vec<(&str, ManifestKind)>) -> Repository {
        Repository {
            id: id.to_string(),
            name: id.to_string(),
            path: PathBuf::from(id),
            description: None,
            primary_language: None,
            manifests,
            license: None,
            readme_present: false,
            is_git_repo: false,
            default_branch: None,
            last_commit_at: None,
            commit_count: None,
            contributor_count: None,
            dependencies: deps
                .into_iter()
                .map(|(name, manifest)| DependencyRef {
                    name: name.to_string(),
                    version_req: None,
                    manifest,
                    is_path_dependency: false,
                    path_hint: None,
                    resolved_repo: None,
                })
                .collect(),
        }
    }

    #[test]
    fn excludes_singletons_and_ubiquitous_dependencies() {
        // 10 Cargo repos: "rare" shared by 2 (a real candidate), "everywhere"
        // shared by all 10 (ubiquitous — excluded at the default 20% cap).
        let mut repos = vec![
            repo("a", vec![ManifestKind::Cargo], vec![("rare", ManifestKind::Cargo), ("everywhere", ManifestKind::Cargo)]),
            repo("b", vec![ManifestKind::Cargo], vec![("rare", ManifestKind::Cargo), ("everywhere", ManifestKind::Cargo)]),
            repo("c", vec![ManifestKind::Cargo], vec![("only-here", ManifestKind::Cargo), ("everywhere", ManifestKind::Cargo)]),
        ];
        for name in ["d", "e", "f", "g", "h", "i", "j"] {
            repos.push(repo(
                name,
                vec![ManifestKind::Cargo],
                vec![("everywhere", ManifestKind::Cargo)],
            ));
        }

        let candidates = shared_dependency_candidates(&repos, 2, 0.2);
        let names: Vec<_> = candidates.iter().map(|c| c.dependency_name.as_str()).collect();

        assert!(names.contains(&"rare"));
        assert!(!names.contains(&"only-here")); // singleton, below min_repos
        assert!(!names.contains(&"everywhere")); // 9/10 repos, above 20% ubiquity cap
    }

    #[test]
    fn internal_resolved_dependencies_are_never_candidates() {
        let mut a = repo("a", vec![ManifestKind::Cargo], vec![]);
        a.dependencies.push(DependencyRef {
            name: "b".to_string(),
            version_req: None,
            manifest: ManifestKind::Cargo,
            is_path_dependency: true,
            path_hint: None,
            resolved_repo: Some("b".to_string()),
        });
        let mut c = repo("c", vec![ManifestKind::Cargo], vec![]);
        c.dependencies.push(DependencyRef {
            name: "b".to_string(),
            version_req: None,
            manifest: ManifestKind::Cargo,
            is_path_dependency: true,
            path_hint: None,
            resolved_repo: Some("b".to_string()),
        });
        let b = repo("b", vec![ManifestKind::Cargo], vec![]);

        let candidates = shared_dependency_candidates(&[a, b, c], 2, 1.0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn ecosystem_denominator_is_scoped_to_manifest_kind() {
        // 2 npm repos both pulling in "react" — scoped per-kind, its
        // denominator must be the npm repo count (2), not diluted by 20
        // unrelated Cargo repos also in the snapshot. The `.max(min_repos)`
        // floor means it stays a valid candidate even though 2/2 = 100%
        // ubiquity within its own (tiny) ecosystem.
        let mut repos = vec![
            repo("web-a", vec![ManifestKind::Npm], vec![("react", ManifestKind::Npm)]),
            repo("web-b", vec![ManifestKind::Npm], vec![("react", ManifestKind::Npm)]),
        ];
        for i in 0..20 {
            repos.push(repo(&format!("rust-{i}"), vec![ManifestKind::Cargo], vec![]));
        }

        let candidates = shared_dependency_candidates(&repos, 2, 0.2);
        let react = candidates
            .iter()
            .find(|c| c.dependency_name == "react")
            .expect("react should survive the min_repos floor despite 100% npm ubiquity");
        assert_eq!(react.ecosystem_total, 2);
    }
}
