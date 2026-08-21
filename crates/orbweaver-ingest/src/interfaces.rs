//! Static extraction of clap-style CLI subcommand surfaces from Rust
//! source (directive section 12/9: what does a capability actually
//! expose). This is a text-pattern heuristic over `.rs` files — no
//! proc-macro expansion, no `cargo build`, no code execution. It looks
//! for `#[derive(Subcommand)]` immediately preceding an `enum`, and reads
//! off each variant's identifier (converted to the kebab-case name clap
//! derives by default) and its `///` doc comment.
//!
//! What it deliberately can't see: builder-style clap (`Command::new(...)
//! .subcommand(...)`), non-Rust CLIs, and `#[command(name = "...")]`
//! renames. Every finding is `ProbabilisticInference`, never asserted as
//! fact — this is a lead for a human or a Tier-2 reasoning pass to
//! confirm, not a verified interface contract.

use crate::rust_scan::{self, leading_doc_and_identifier};
use orbweaver_core::{Interface, ManifestKind, Repository};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use std::collections::BTreeSet;
use std::fs;

const MAX_FILES: usize = 400;

pub fn extract(repo: &Repository) -> (Vec<Interface>, Vec<Evidence>) {
    if !repo.manifests.contains(&ManifestKind::Cargo) {
        return (Vec::new(), Vec::new());
    }

    let mut interfaces = Vec::new();
    let mut evidence = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    for file in rust_scan::rust_files(&repo.path, MAX_FILES) {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        let content = rust_scan::strip_test_modules(&content);
        let enums = rust_scan::find_annotated_blocks(&content, "enum", |line| line.contains("Subcommand"));

        for enum_block in enums {
            for chunk in rust_scan::split_top_level(&enum_block.body) {
                let (doc, identifier) = leading_doc_and_identifier(&chunk);
                let Some(identifier) = identifier else { continue };
                let name = to_kebab_case(&identifier);
                if name.is_empty() || !seen_names.insert(name.clone()) {
                    continue;
                }

                interfaces.push(Interface {
                    id: format!("{}:{}", repo.id, name),
                    repository: repo.id.clone(),
                    name: name.clone(),
                    description: doc,
                });
                evidence.push(Evidence::new(
                    file.display().to_string(),
                    SourceType::Filesystem,
                    repo.id.clone(),
                    "cli_interface_extractor",
                    Confidence::ProbabilisticInference(0.7),
                    format!("enum {} variant {identifier}", enum_block.name),
                    format!(
                        "{} exposes CLI subcommand `{name}` (heuristic: found in \
                         #[derive(Subcommand)] enum {}; not verified against runtime --help)",
                        repo.id, enum_block.name
                    ),
                ));
            }
        }
    }

    (interfaces, evidence)
}

/// Clap's default subcommand rename: `PascalCase` -> `kebab-case`.
fn to_kebab_case(ident: &str) -> String {
    let chars: Vec<char> = ident.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            out.push('-');
            continue;
        }
        if c.is_uppercase() {
            let prev_lower_or_digit = i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
            if i > 0 && (prev_lower_or_digit || next_lower) {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn repo(path: &Path) -> Repository {
        Repository {
            id: "widget".to_string(),
            name: "widget".to_string(),
            path: path.to_path_buf(),
            description: None,
            primary_language: None,
            manifests: vec![ManifestKind::Cargo],
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
    fn kebab_case_matches_clap_defaults() {
        assert_eq!(to_kebab_case("Scan"), "scan");
        assert_eq!(to_kebab_case("Snapshots"), "snapshots");
        assert_eq!(to_kebab_case("MaxUbiquity"), "max-ubiquity");
        assert_eq!(to_kebab_case("HTTPServer"), "http-server");
    }

    #[test]
    fn extracts_variants_with_doc_comments_from_realistic_enum() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"widget\"",
        )
        .unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            r#"
use clap::Subcommand;

#[derive(Subcommand)]
enum Command {
    /// Discover repositories and persist a snapshot.
    Scan {
        #[arg(long, default_value = ".")]
        root: PathBuf,
        json: bool,
    },
    /// Show the most recent snapshot.
    Status,
    ListSnapshots,
}

fn main() {}
"#,
        )
        .unwrap();

        let r = repo(dir.path());
        let (interfaces, evidence) = extract(&r);

        let names: BTreeSet<_> = interfaces.iter().map(|i| i.name.clone()).collect();
        assert_eq!(
            names,
            BTreeSet::from([
                "scan".to_string(),
                "status".to_string(),
                "list-snapshots".to_string()
            ])
        );

        let scan = interfaces.iter().find(|i| i.name == "scan").unwrap();
        assert_eq!(
            scan.description.as_deref(),
            Some("Discover repositories and persist a snapshot.")
        );
        assert_eq!(evidence.len(), 3);
    }

    #[test]
    fn ignores_enums_without_subcommand_derive() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "#[derive(Debug)]\nenum Color { Red, Green, Blue }\nfn main() {}\n",
        )
        .unwrap();

        let r = repo(dir.path());
        let (interfaces, _) = extract(&r);
        assert!(interfaces.is_empty());
    }

    #[test]
    fn non_cargo_repo_yields_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = repo(dir.path());
        r.manifests = vec![ManifestKind::Npm];
        let (interfaces, evidence) = extract(&r);
        assert!(interfaces.is_empty());
        assert!(evidence.is_empty());
    }

    /// Regression test: this exact bug shipped once — scanning our own
    /// interfaces.rs picked up a `ListSnapshots` variant that only ever
    /// existed inside a `#[cfg(test)]` fixture's string literal, not in
    /// any real CLI. A file's real subcommand enum must survive; anything
    /// declared only inside a test module must not appear at all.
    #[test]
    fn does_not_extract_subcommand_enums_declared_only_inside_test_modules() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            r####"
#[derive(Subcommand)]
enum Command {
    /// The real, user-facing subcommand.
    Scan,
}

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn something() {
        let fixture = r###"
#[derive(Subcommand)]
enum Command {
    Scan,
    ListSnapshots,
}
"###;
        assert!(fixture.contains("ListSnapshots"));
    }
}
"####,
        )
        .unwrap();

        let r = repo(dir.path());
        let (interfaces, _) = extract(&r);

        let names: BTreeSet<_> = interfaces.iter().map(|i| i.name.clone()).collect();
        assert_eq!(names, BTreeSet::from(["scan".to_string()]));
    }
}
