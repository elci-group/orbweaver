//! Static extraction of serde-derived data structures from Rust source
//! (directive section 9/12: schemas as evidence for what a capability
//! consumes/produces). Same text-pattern approach as `interfaces`, and
//! built on the same shared scanning primitives — no proc-macro
//! expansion, no execution.
//!
//! What it deliberately can't see: fields contributed by other derives
//! or macros, `#[serde(flatten)]` semantics, and tuple/unit structs
//! (only `struct Name { field: Type, ... }` bodies are recognised).
//! Every finding is `ProbabilisticInference`.

use crate::rust_scan;
use orbweaver_core::{ManifestKind, Repository, Schema, SchemaField};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use std::collections::BTreeSet;
use std::fs;

const MAX_FILES: usize = 400;

pub fn extract(repo: &Repository) -> (Vec<Schema>, Vec<Evidence>) {
    if !repo.manifests.contains(&ManifestKind::Cargo) {
        return (Vec::new(), Vec::new());
    }

    let mut schemas = Vec::new();
    let mut evidence = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    for file in rust_scan::rust_files(&repo.path, MAX_FILES) {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        let content = rust_scan::strip_test_modules(&content);
        let structs = rust_scan::find_annotated_blocks(&content, "struct", |line| {
            line.contains("Serialize") || line.contains("Deserialize")
        });

        for block in structs {
            if !seen_names.insert(block.name.clone()) {
                continue;
            }

            let fields: Vec<SchemaField> = rust_scan::split_top_level(&block.body)
                .iter()
                .filter_map(|chunk| field_from_chunk(chunk))
                .collect();

            if fields.is_empty() {
                // Either a tuple/unit-shaped body we can't parse, or a
                // genuinely empty struct — either way there's nothing to
                // report as a schema.
                continue;
            }

            let field_count = fields.len();
            schemas.push(Schema {
                id: format!("{}:{}", repo.id, block.name),
                repository: repo.id.clone(),
                name: block.name.clone(),
                description: block.doc,
                fields,
            });
            evidence.push(Evidence::new(
                file.display().to_string(),
                SourceType::Filesystem,
                repo.id.clone(),
                "schema_extractor",
                Confidence::ProbabilisticInference(0.7),
                format!("struct {}", block.name),
                format!(
                    "{} declares schema `{}` with {field_count} field(s) (heuristic: found in \
                     #[derive(Serialize/Deserialize)] struct; not verified against actual \
                     serialized output)",
                    repo.id, block.name
                ),
            ));
        }
    }

    (schemas, evidence)
}

/// Pull `name: Type` out of a struct-body chunk, skipping leading doc
/// comments and `#[...]` field attributes (e.g. `#[serde(rename = "...")]`).
fn field_from_chunk(chunk: &str) -> Option<SchemaField> {
    for raw_line in chunk.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("///") || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        return parse_name_type(line);
    }
    None
}

fn parse_name_type(line: &str) -> Option<SchemaField> {
    let line = line.strip_prefix("pub(crate) ").unwrap_or(line);
    let line = line.strip_prefix("pub ").unwrap_or(line);

    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != ':' {
            continue;
        }
        let prev_is_colon = i > 0 && chars[i - 1] == ':';
        let next_is_colon = chars.get(i + 1) == Some(&':');
        if prev_is_colon || next_is_colon {
            continue;
        }

        let name: String = chars[..i].iter().collect::<String>().trim().to_string();
        let type_repr: String = chars[i + 1..]
            .iter()
            .collect::<String>()
            .trim()
            .trim_end_matches(',')
            .trim()
            .to_string();

        if name.is_empty() || type_repr.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        return Some(SchemaField { name, type_repr });
    }
    None
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
    fn extracts_fields_and_struct_doc_from_realistic_struct() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            r#"
/// A single tracked repository, discovered under the scan root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: String,
    /// Human-readable name, defaulting to the directory name.
    pub name: String,
    pub tags: Vec<String>,
    dependencies: Vec<DependencyRef>,
}

fn main() {}
"#,
        )
        .unwrap();

        let r = repo(dir.path());
        let (schemas, evidence) = extract(&r);

        assert_eq!(schemas.len(), 1);
        let schema = &schemas[0];
        assert_eq!(schema.name, "Repository");
        assert_eq!(
            schema.description.as_deref(),
            Some("A single tracked repository, discovered under the scan root.")
        );

        let names: Vec<_> = schema.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["id", "name", "tags", "dependencies"]);
        let name_field = schema.fields.iter().find(|f| f.name == "name").unwrap();
        assert_eq!(name_field.type_repr, "String");
        let deps_field = schema.fields.iter().find(|f| f.name == "dependencies").unwrap();
        assert_eq!(deps_field.type_repr, "Vec<DependencyRef>");

        assert_eq!(evidence.len(), 1);
    }

    #[test]
    fn ignores_structs_without_serde_derive() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            "#[derive(Debug, Clone)]\npub struct Internal { x: u32 }\nfn main() {}\n",
        )
        .unwrap();

        let r = repo(dir.path());
        let (schemas, _) = extract(&r);
        assert!(schemas.is_empty());
    }

    #[test]
    fn does_not_extract_schemas_declared_only_inside_test_modules() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"widget\"").unwrap();
        fs::write(
            dir.path().join("src/main.rs"),
            r####"
#[derive(Serialize, Deserialize)]
pub struct Real {
    pub id: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn something() {
        let fixture = r###"
#[derive(Serialize, Deserialize)]
pub struct Fake {
    pub id: String,
}
"###;
        assert!(fixture.contains("Fake"));
    }
}
"####,
        )
        .unwrap();

        let r = repo(dir.path());
        let (schemas, _) = extract(&r);

        let names: BTreeSet<_> = schemas.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, BTreeSet::from(["Real".to_string()]));
    }
}
