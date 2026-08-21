//! Capability extraction (directive section 12): "function != capability".
//!
//! Phase II slice — the smallest extractor that produces a real,
//! evidence-backed answer to "what does this repository do" without
//! executing any repository's code:
//!
//!   declared bin/script entry point (Cargo `[[bin]]`/`src/main.rs`,
//!   npm `bin`, PEP 621 `[project.scripts]`, Poetry
//!   `[tool.poetry.scripts]`, a Go `package main`)
//!         +
//!   a description (manifest `description` field, else README presence)
//!         =
//!   a `Cli` capability, with `evidence_sources` reflecting how many of
//!   those independent signals actually agreed.
//!
//! A repository with a manifest but no such entry point still gets a
//! single, weaker `Library` capability — we know it's meant to be
//! consumed by something, we just haven't parsed its public API yet
//! (that needs real source analysis, not this pass).

use orbweaver_core::{Capability, CapabilityKind, ManifestKind, Repository};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use std::fs;
use std::path::Path;

pub fn extract(repo: &Repository) -> (Vec<Capability>, Vec<Evidence>) {
    let bin_names = declared_binaries(repo);

    if bin_names.is_empty() {
        if repo.manifests.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let evidence_sources = 1 + repo.description.is_some() as u32;
        let capability = Capability {
            id: format!("{}:library", repo.id),
            repository: repo.id.clone(),
            name: repo.name.clone(),
            kind: CapabilityKind::Library,
            description: repo.description.clone(),
            evidence_sources,
        };
        let evidence = Evidence::new(
            repo.path.display().to_string(),
            SourceType::Manifest,
            repo.id.clone(),
            "capability_extractor",
            Confidence::DeterministicInference,
            "no bin/script entry point found",
            format!("{} is a library capability (no declared entry point)", repo.id),
        );
        return (vec![capability], vec![evidence]);
    }

    let mut capabilities = Vec::new();
    let mut evidence = Vec::new();

    for name in bin_names {
        let evidence_sources = 1 + repo.description.is_some() as u32 + repo.readme_present as u32;
        capabilities.push(Capability {
            id: format!("{}:{}", repo.id, name),
            repository: repo.id.clone(),
            name: name.clone(),
            kind: CapabilityKind::Cli,
            description: repo.description.clone(),
            evidence_sources,
        });
        evidence.push(Evidence::new(
            repo.path.display().to_string(),
            SourceType::Manifest,
            repo.id.clone(),
            "capability_extractor",
            Confidence::DeterministicInference,
            name.clone(),
            format!("{} exposes a CLI capability named `{name}`", repo.id),
        ));
    }

    (capabilities, evidence)
}

fn declared_binaries(repo: &Repository) -> Vec<String> {
    let mut names = Vec::new();
    for kind in &repo.manifests {
        match kind {
            ManifestKind::Cargo => names.extend(cargo_binaries(&repo.path, &repo.name)),
            ManifestKind::Npm => names.extend(npm_binaries(&repo.path)),
            ManifestKind::PyprojectPoetry | ManifestKind::PyprojectPep621 => {
                names.extend(python_scripts(&repo.path))
            }
            ManifestKind::Go => names.extend(go_binaries(&repo.path)),
        }
    }
    names.sort();
    names.dedup();
    names
}

fn cargo_binaries(path: &Path, package_name: &str) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return Vec::new();
    };

    let mut names: Vec<String> = value
        .get("bin")
        .and_then(|v| v.as_array())
        .map(|bins| {
            bins.iter()
                .filter_map(|b| b.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if names.is_empty() && path.join("src/main.rs").exists() {
        names.push(package_name.to_string());
    }

    names
}

fn npm_binaries(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path.join("package.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };

    match value.get("bin") {
        Some(serde_json::Value::String(_)) => value
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        Some(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn python_scripts(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path.join("pyproject.toml")) else {
        return Vec::new();
    };
    let Ok(value) = toml::from_str::<toml::Value>(&content) else {
        return Vec::new();
    };

    let pep621 = value
        .get("project")
        .and_then(|p| p.get("scripts"))
        .and_then(|v| v.as_table())
        .map(|t| t.keys().cloned().collect::<Vec<_>>());

    let poetry = value
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("scripts"))
        .and_then(|v| v.as_table())
        .map(|t| t.keys().cloned().collect::<Vec<_>>());

    pep621.or(poetry).unwrap_or_default()
}

/// Heuristic only: a top-level `.go` file, or a `cmd/<name>/` directory,
/// containing the literal text `package main`. This is a text search, not
/// a parse — good enough to say "this looks like a Go command", not
/// precise enough to be trusted beyond that.
fn go_binaries(path: &Path) -> Vec<String> {
    let mut names = Vec::new();

    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("go") && contains_package_main(&p) {
                if let Some(stem) = path.file_name().and_then(|n| n.to_str()) {
                    names.push(stem.to_string());
                }
                break;
            }
        }
    }

    let cmd_dir = path.join("cmd");
    if let Ok(entries) = fs::read_dir(&cmd_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let has_main = fs::read_dir(&p)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|f| {
                        f.path().extension().and_then(|e| e.to_str()) == Some("go")
                            && contains_package_main(&f.path())
                    });
                if has_main {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }

    names
}

fn contains_package_main(go_file: &Path) -> bool {
    fs::read_to_string(go_file)
        .map(|c| c.lines().any(|l| l.trim() == "package main"))
        .unwrap_or(false)
}
