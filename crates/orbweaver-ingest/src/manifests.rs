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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn cargo_toml_extracts_package_facts_and_path_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "Cargo.toml",
            r#"
                [package]
                name = "widget"
                description = "does widget things"
                license = "MIT"

                [dependencies]
                serde = "1"
                orbweaver-core = { path = "../orbweaver-core" }
            "#,
        );

        let facts = parse_cargo_toml(&path).unwrap();
        assert_eq!(facts.name.as_deref(), Some("widget"));
        assert_eq!(facts.description.as_deref(), Some("does widget things"));
        assert_eq!(facts.license.as_deref(), Some("MIT"));
        assert_eq!(facts.dependencies.len(), 2);

        let serde_dep = facts.dependencies.iter().find(|d| d.name == "serde").unwrap();
        assert!(!serde_dep.is_path_dependency);
        assert_eq!(serde_dep.version_req.as_deref(), Some("1"));

        let path_dep = facts
            .dependencies
            .iter()
            .find(|d| d.name == "orbweaver-core")
            .unwrap();
        assert!(path_dep.is_path_dependency);
        assert_eq!(path_dep.path_hint.as_deref(), Some("../orbweaver-core"));
    }

    #[test]
    fn package_json_distinguishes_registry_workspace_and_file_deps() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "package.json",
            r#"{
                "name": "webapp",
                "description": "a webapp",
                "dependencies": {
                    "react": "^18.0.0",
                    "local-lib": "file:../local-lib",
                    "internal-pkg": "workspace:*"
                }
            }"#,
        );

        let facts = parse_package_json(&path).unwrap();
        assert_eq!(facts.name.as_deref(), Some("webapp"));
        assert_eq!(facts.dependencies.len(), 3);

        let react = facts.dependencies.iter().find(|d| d.name == "react").unwrap();
        assert!(!react.is_path_dependency);

        let local = facts
            .dependencies
            .iter()
            .find(|d| d.name == "local-lib")
            .unwrap();
        assert!(local.is_path_dependency);
        assert_eq!(local.path_hint.as_deref(), Some("../local-lib"));

        let workspace = facts
            .dependencies
            .iter()
            .find(|d| d.name == "internal-pkg")
            .unwrap();
        assert!(workspace.is_path_dependency);
        assert_eq!(workspace.path_hint, None);
    }

    #[test]
    fn pyproject_pep621_parses_dependency_names_from_version_specifiers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "pyproject.toml",
            r#"
                [project]
                name = "toolkit"
                description = "a toolkit"
                dependencies = ["requests>=2.0,<3", "click"]
            "#,
        );

        let facts = parse_pyproject_toml(&path).unwrap();
        assert_eq!(facts.name.as_deref(), Some("toolkit"));
        let names: Vec<_> = facts.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["requests", "click"]);
    }

    #[test]
    fn pyproject_poetry_parses_table_and_string_dependencies_and_skips_python() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "pyproject.toml",
            r#"
                [tool.poetry]
                name = "toolkit"

                [tool.poetry.dependencies]
                python = "^3.11"
                requests = "^2.0"
                local-lib = { path = "../local-lib" }
            "#,
        );

        let facts = parse_pyproject_toml(&path).unwrap();
        let names: Vec<_> = facts.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(!names.contains(&"python"));

        let local = facts
            .dependencies
            .iter()
            .find(|d| d.name == "local-lib")
            .unwrap();
        assert!(local.is_path_dependency);
    }

    #[test]
    fn go_mod_parses_require_block_and_local_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "go.mod",
            "module github.com/elci-group/widget\n\
             \n\
             go 1.21\n\
             \n\
             require (\n\
             \tgithub.com/spf13/cobra v1.8.0\n\
             \tgithub.com/elci-group/other v0.1.0\n\
             )\n\
             \n\
             replace github.com/elci-group/other => ../other\n",
        );

        let facts = parse_go_mod(&path).unwrap();
        assert_eq!(facts.name.as_deref(), Some("widget"));
        assert_eq!(facts.dependencies.len(), 2);

        let cobra = facts
            .dependencies
            .iter()
            .find(|d| d.name == "github.com/spf13/cobra")
            .unwrap();
        assert!(!cobra.is_path_dependency);

        let other = facts
            .dependencies
            .iter()
            .find(|d| d.name == "github.com/elci-group/other")
            .unwrap();
        assert!(other.is_path_dependency);
        assert_eq!(other.path_hint.as_deref(), Some("../other"));
    }
}
