//! Phase I ingestion pipeline: discover candidate repositories under a
//! root, extract deterministic facts from their manifests and git history,
//! and resolve internal dependency edges between the repositories found in
//! the same scan. Every claim produced here carries an [`Evidence`] record
//! (directive section 10) — nothing is asserted without a source.

mod capabilities;
mod discover;
mod interfaces;
mod manifests;
mod rust_scan;
mod schemas;

pub use discover::discover_repositories;

use orbweaver_core::{Capability, Interface, ManifestKind, Repository, Schema};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ScanConfig {
    pub root: PathBuf,
    /// Cap on commits walked per repository (see `orbweaver_git::inspect`).
    pub max_commits: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            max_commits: 20_000,
        }
    }
}

pub struct ScanResult {
    pub repositories: Vec<Repository>,
    pub capabilities: Vec<Capability>,
    pub interfaces: Vec<Interface>,
    pub schemas: Vec<Schema>,
    pub evidence: Vec<Evidence>,
}

pub fn scan(config: &ScanConfig) -> std::io::Result<ScanResult> {
    let paths = discover_repositories(&config.root)?;

    let mut repositories = Vec::with_capacity(paths.len());
    let mut evidence = Vec::new();

    for path in &paths {
        let (repo, mut repo_evidence) = ingest_repository(path, config.max_commits);
        evidence.append(&mut repo_evidence);
        repositories.push(repo);
    }

    resolve_internal_dependencies(&mut repositories, &mut evidence);

    let mut all_capabilities = Vec::new();
    let mut all_interfaces = Vec::new();
    let mut all_schemas = Vec::new();
    for repo in &repositories {
        let (mut caps, mut cap_evidence) = capabilities::extract(repo);
        all_capabilities.append(&mut caps);
        evidence.append(&mut cap_evidence);

        let (mut ifaces, mut iface_evidence) = interfaces::extract(repo);
        all_interfaces.append(&mut ifaces);
        evidence.append(&mut iface_evidence);

        let (mut repo_schemas, mut schema_evidence) = schemas::extract(repo);
        all_schemas.append(&mut repo_schemas);
        evidence.append(&mut schema_evidence);
    }

    Ok(ScanResult {
        repositories,
        capabilities: all_capabilities,
        schemas: all_schemas,
        interfaces: all_interfaces,
        evidence,
    })
}

fn ingest_repository(path: &Path, max_commits: usize) -> (Repository, Vec<Evidence>) {
    let dir_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let id = dir_name.clone();

    let mut evidence = Vec::new();
    let mut manifests_found = Vec::new();
    let mut name = None;
    let mut description = None;
    let mut license = None;
    let mut dependencies = Vec::new();

    let manifest_candidates: [(&str, ManifestKind); 4] = [
        ("Cargo.toml", ManifestKind::Cargo),
        ("package.json", ManifestKind::Npm),
        ("pyproject.toml", ManifestKind::PyprojectPoetry),
        ("go.mod", ManifestKind::Go),
    ];

    for (file, kind) in manifest_candidates {
        let manifest_path = path.join(file);
        if !manifest_path.exists() {
            continue;
        }

        let facts = match kind {
            ManifestKind::Cargo => manifests::parse_cargo_toml(&manifest_path),
            ManifestKind::Npm => manifests::parse_package_json(&manifest_path),
            ManifestKind::PyprojectPoetry => manifests::parse_pyproject_toml(&manifest_path),
            ManifestKind::Go => manifests::parse_go_mod(&manifest_path),
            ManifestKind::PyprojectPep621 => None, // resolved dynamically inside parse_pyproject_toml
        };

        let Some(facts) = facts else { continue };

        manifests_found.push(kind);
        if name.is_none() {
            name = facts.name.clone();
        }
        if description.is_none() {
            description = facts.description.clone();
        }
        if license.is_none() {
            license = facts.license.clone();
        }
        let dep_count = facts.dependencies.len();
        dependencies.extend(facts.dependencies);

        evidence.push(Evidence::new(
            manifest_path.display().to_string(),
            SourceType::Manifest,
            id.clone(),
            "manifest_scanner",
            Confidence::Observed,
            file,
            format!("{file} declares {dep_count} dependencies"),
        ));
    }

    let readme_present = ["README.md", "README", "README.rst", "readme.md"]
        .iter()
        .any(|f| path.join(f).exists());
    if readme_present {
        evidence.push(Evidence::new(
            path.display().to_string(),
            SourceType::Readme,
            id.clone(),
            "filesystem_scanner",
            Confidence::Observed,
            "README present",
            "Repository has a README".to_string(),
        ));
    }

    let is_git_repo = orbweaver_git::is_git_repo(path);
    let git_info = if is_git_repo {
        orbweaver_git::inspect(path, max_commits)
    } else {
        None
    };
    if let Some(info) = &git_info {
        evidence.push(Evidence::new(
            path.display().to_string(),
            SourceType::GitHistory,
            id.clone(),
            "git_history_scanner",
            Confidence::Observed,
            "HEAD revwalk",
            format!(
                "{} commits by {} contributor(s), last commit {}",
                info.commit_count.unwrap_or(0),
                info.contributor_count.unwrap_or(0),
                info.last_commit_at
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
        ));
    }

    let primary_language = manifests_found.first().map(|k| {
        match k {
            ManifestKind::Cargo => "Rust",
            ManifestKind::Npm => "JavaScript/TypeScript",
            ManifestKind::PyprojectPoetry | ManifestKind::PyprojectPep621 => "Python",
            ManifestKind::Go => "Go",
        }
        .to_string()
    });

    let repo = Repository {
        id,
        name: name.unwrap_or(dir_name),
        path: path.to_path_buf(),
        description,
        primary_language,
        manifests: manifests_found,
        license,
        readme_present,
        is_git_repo,
        default_branch: git_info.as_ref().and_then(|g| g.default_branch.clone()),
        last_commit_at: git_info.as_ref().and_then(|g| g.last_commit_at),
        commit_count: git_info.as_ref().and_then(|g| g.commit_count),
        contributor_count: git_info.as_ref().and_then(|g| g.contributor_count),
        dependencies,
    };

    (repo, evidence)
}

/// Match each repository's declared dependencies against the other
/// repositories found in the same scan. A path dependency that resolves to
/// another discovered repository's directory is the strongest signal
/// (`DeterministicInference`); a bare name match is weaker but still
/// deterministic, since it is a fixed string-equality rule, not a guess.
fn resolve_internal_dependencies(repositories: &mut [Repository], evidence: &mut Vec<Evidence>) {
    let by_path: HashMap<PathBuf, String> = repositories
        .iter()
        .filter_map(|r| r.path.canonicalize().ok().map(|p| (p, r.id.clone())))
        .collect();
    let by_name: HashMap<String, String> = repositories
        .iter()
        .map(|r| (r.name.to_lowercase(), r.id.clone()))
        .collect();

    let mut resolutions: Vec<(String, usize, String, bool)> = Vec::new();

    for repo in repositories.iter() {
        for (dep_idx, dep) in repo.dependencies.iter().enumerate() {
            if let Some(hint) = &dep.path_hint {
                if let Some(target_id) = repo
                    .path
                    .join(hint)
                    .canonicalize()
                    .ok()
                    .and_then(|p| by_path.get(&p))
                {
                    if target_id != &repo.id {
                        resolutions.push((repo.id.clone(), dep_idx, target_id.clone(), true));
                        continue;
                    }
                }
            }
            if let Some(target_id) = by_name.get(&dep.name.to_lowercase()) {
                if target_id != &repo.id {
                    resolutions.push((repo.id.clone(), dep_idx, target_id.clone(), false));
                }
            }
        }
    }

    let index: HashMap<String, usize> = repositories
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();

    for (repo_id, dep_idx, target_id, via_path) in resolutions {
        let Some(&repo_pos) = index.get(repo_id.as_str()) else {
            continue;
        };
        let dep_name = repositories[repo_pos].dependencies[dep_idx].name.clone();
        repositories[repo_pos].dependencies[dep_idx].resolved_repo = Some(target_id.clone());

        evidence.push(Evidence::new(
            repo_id.clone(),
            SourceType::Manifest,
            repo_id.clone(),
            "dependency_resolver",
            Confidence::DeterministicInference,
            dep_name.clone(),
            format!(
                "{repo_id} depends_on {target_id} (dependency `{dep_name}` resolved via {})",
                if via_path { "path match" } else { "name match" }
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbweaver_core::DependencyRef;

    fn bare_repo(id: &str, path: std::path::PathBuf) -> Repository {
        Repository {
            id: id.to_string(),
            name: id.to_string(),
            path,
            description: None,
            primary_language: None,
            manifests: vec![],
            license: None,
            readme_present: false,
            is_git_repo: false,
            default_branch: None,
            last_commit_at: None,
            commit_count: None,
            contributor_count: None,
            dependencies: vec![],
        }
    }

    #[test]
    fn resolves_path_dependency_over_name_match_and_ignores_self_references() {
        let root = tempfile::tempdir().unwrap();
        let a_path = root.path().join("a");
        let b_path = root.path().join("b");
        std::fs::create_dir(&a_path).unwrap();
        std::fs::create_dir(&b_path).unwrap();

        let mut a = bare_repo("a", a_path);
        a.dependencies.push(DependencyRef {
            name: "b-crate".to_string(),
            version_req: None,
            manifest: ManifestKind::Cargo,
            is_path_dependency: true,
            path_hint: Some("../b".to_string()),
            resolved_repo: None,
        });
        // A dependency on itself by name must never resolve to itself.
        a.dependencies.push(DependencyRef {
            name: "a".to_string(),
            version_req: None,
            manifest: ManifestKind::Cargo,
            is_path_dependency: false,
            path_hint: None,
            resolved_repo: None,
        });

        let b = bare_repo("b", b_path);

        let mut repos = vec![a, b];
        let mut evidence = Vec::new();
        resolve_internal_dependencies(&mut repos, &mut evidence);

        let a = &repos[0];
        assert_eq!(a.dependencies[0].resolved_repo.as_deref(), Some("b"));
        assert_eq!(a.dependencies[1].resolved_repo, None);
        assert_eq!(evidence.len(), 1);
    }

    #[test]
    fn resolves_by_case_insensitive_name_match_when_no_path_hint() {
        let root = tempfile::tempdir().unwrap();
        let c_path = root.path().join("c");
        let d_path = root.path().join("d");
        std::fs::create_dir(&c_path).unwrap();
        std::fs::create_dir(&d_path).unwrap();

        let mut c = bare_repo("c", c_path);
        c.dependencies.push(DependencyRef {
            name: "D".to_string(),
            version_req: None,
            manifest: ManifestKind::Npm,
            is_path_dependency: false,
            path_hint: None,
            resolved_repo: None,
        });
        let d = bare_repo("d", d_path);

        let mut repos = vec![c, d];
        let mut evidence = Vec::new();
        resolve_internal_dependencies(&mut repos, &mut evidence);

        assert_eq!(repos[0].dependencies[0].resolved_repo.as_deref(), Some("d"));
    }
}
