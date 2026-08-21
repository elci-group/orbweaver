//! Deterministic manifest parsing. Every function here reads exactly what
//! is written in the file — no inference about what a dependency "really"
//! means, that happens one layer up when edges get resolved against other
//! discovered repositories.

use orbweaver_core::{DependencyRef, ManifestKind};
use std::fs;
use std::path::Path;

pub struct ManifestFacts {
    pub name: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub dependencies: Vec<DependencyRef>,
}

pub fn parse_cargo_toml(path: &Path) -> Option<ManifestFacts> {
    let content = fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;

    let package = value.get("package");
    let name = package
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let description = package
        .and_then(|p| p.get("description"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let license = package
        .and_then(|p| p.get("license"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut dependencies = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(toml::Value::Table(table)) = value.get(section) {
            dependencies.extend(cargo_deps_from_table(table));
        }
    }

    Some(ManifestFacts {
        name,
        description,
        license,
        dependencies,
    })
}

fn cargo_deps_from_table(table: &toml::value::Table) -> Vec<DependencyRef> {
    table
        .iter()
        .filter_map(|(name, spec)| match spec {
            toml::Value::String(version) => Some(DependencyRef {
                name: name.clone(),
                version_req: Some(version.clone()),
                manifest: ManifestKind::Cargo,
                is_path_dependency: false,
                path_hint: None,
                resolved_repo: None,
            }),
            toml::Value::Table(t) => {
                let path_hint = t.get("path").and_then(|v| v.as_str()).map(String::from);
                let version_req = t.get("version").and_then(|v| v.as_str()).map(String::from);
                Some(DependencyRef {
                    name: name.clone(),
                    version_req,
                    manifest: ManifestKind::Cargo,
                    is_path_dependency: path_hint.is_some(),
                    path_hint,
                    resolved_repo: None,
                })
            }
            _ => None,
        })
        .collect()
}

pub fn parse_package_json(path: &Path) -> Option<ManifestFacts> {
    let content = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;

    let name = value.get("name").and_then(|v| v.as_str()).map(String::from);
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);
    let license = value
        .get("license")
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut dependencies = Vec::new();
    for section in ["dependencies", "devDependencies"] {
        if let Some(serde_json::Value::Object(map)) = value.get(section) {
            for (name, version_value) in map {
                let Some(version) = version_value.as_str() else {
                    continue;
                };
                let (is_path, path_hint) = if let Some(rest) = version.strip_prefix("file:") {
                    (true, Some(rest.to_string()))
                } else if version.starts_with("workspace:") || version.starts_with("link:") {
                    (true, None)
                } else {
                    (false, None)
                };
                dependencies.push(DependencyRef {
                    name: name.clone(),
                    version_req: Some(version.to_string()),
                    manifest: ManifestKind::Npm,
                    is_path_dependency: is_path,
                    path_hint,
                    resolved_repo: None,
                });
            }
        }
    }

    Some(ManifestFacts {
        name,
        description,
        license,
        dependencies,
    })
}

pub fn parse_pyproject_toml(path: &Path) -> Option<ManifestFacts> {
    let content = fs::read_to_string(path).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;

    if let Some(project) = value.get("project") {
        let name = project.get("name").and_then(|v| v.as_str()).map(String::from);
        let description = project
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let license = project
            .get("license")
            .and_then(|l| l.as_str().map(String::from).or_else(|| {
                l.get("text").and_then(|v| v.as_str()).map(String::from)
            }));

        let dependencies = project
            .get("dependencies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|spec| DependencyRef {
                        name: pep508_package_name(spec),
                        version_req: Some(spec.to_string()),
                        manifest: ManifestKind::PyprojectPep621,
                        is_path_dependency: false,
                        path_hint: None,
                        resolved_repo: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        return Some(ManifestFacts {
            name,
            description,
            license,
            dependencies,
        });
    }

    if let Some(poetry) = value.get("tool").and_then(|t| t.get("poetry")) {
        let name = poetry.get("name").and_then(|v| v.as_str()).map(String::from);
        let description = poetry
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        let license = poetry.get("license").and_then(|v| v.as_str()).map(String::from);

        let mut dependencies = Vec::new();
        if let Some(toml::Value::Table(table)) = poetry.get("dependencies") {
            for (name, spec) in table {
                if name == "python" {
                    continue;
                }
                match spec {
                    toml::Value::String(version) => dependencies.push(DependencyRef {
                        name: name.clone(),
                        version_req: Some(version.clone()),
                        manifest: ManifestKind::PyprojectPoetry,
                        is_path_dependency: false,
                        path_hint: None,
                        resolved_repo: None,
                    }),
                    toml::Value::Table(t) => {
                        let path_hint = t.get("path").and_then(|v| v.as_str()).map(String::from);
                        dependencies.push(DependencyRef {
                            name: name.clone(),
                            version_req: t.get("version").and_then(|v| v.as_str()).map(String::from),
                            manifest: ManifestKind::PyprojectPoetry,
                            is_path_dependency: path_hint.is_some(),
                            path_hint,
                            resolved_repo: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        return Some(ManifestFacts {
            name,
            description,
            license,
            dependencies,
        });
    }

    None
}

/// Extract the package name from a PEP 508 requirement string, e.g.
/// `"requests>=2.0,<3"` -> `"requests"`.
fn pep508_package_name(spec: &str) -> String {
    spec.chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

pub fn parse_go_mod(path: &Path) -> Option<ManifestFacts> {
    let content = fs::read_to_string(path).ok()?;

    let mut name = None;
    let mut dependencies: Vec<DependencyRef> = Vec::new();
    let mut in_require_block = false;

    for line in content.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("module ") {
            name = Some(rest.trim().rsplit('/').next().unwrap_or(rest.trim()).to_string());
            continue;
        }

        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if in_require_block && line == ")" {
            in_require_block = false;
            continue;
        }

        let require_line = if in_require_block {
            Some(line)
        } else {
            line.strip_prefix("require ")
        };

        if let Some(spec) = require_line {
            let mut parts = spec.split_whitespace();
            if let (Some(mod_path), Some(version)) = (parts.next(), parts.next()) {
                dependencies.push(DependencyRef {
                    name: mod_path.to_string(),
                    version_req: Some(version.to_string()),
                    manifest: ManifestKind::Go,
                    is_path_dependency: false,
                    path_hint: None,
                    resolved_repo: None,
                });
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("replace ") {
            if let Some((target, replacement)) = rest.split_once("=>") {
                let target = target.split_whitespace().next().unwrap_or("").trim();
                let replacement = replacement.trim();
                let local_path = replacement.split_whitespace().next().unwrap_or("");
                if local_path.starts_with('.') || local_path.starts_with('/') {
                    if let Some(existing) = dependencies.iter_mut().find(|d| d.name == target) {
                        existing.is_path_dependency = true;
                        existing.path_hint = Some(local_path.to_string());
                    } else {
                        dependencies.push(DependencyRef {
                            name: target.to_string(),
                            version_req: None,
                            manifest: ManifestKind::Go,
                            is_path_dependency: true,
                            path_hint: Some(local_path.to_string()),
                            resolved_repo: None,
                        });
                    }
                }
            }
        }
    }

    Some(ManifestFacts {
        name,
        description: None,
        license: None,
        dependencies,
    })
}
