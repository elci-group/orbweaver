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

use orbweaver_core::{Interface, ManifestKind, Repository};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "vendor", "dist", "build"];
const MAX_FILES: usize = 400;

pub fn extract(repo: &Repository) -> (Vec<Interface>, Vec<Evidence>) {
    if !repo.manifests.contains(&ManifestKind::Cargo) {
        return (Vec::new(), Vec::new());
    }

    let mut interfaces = Vec::new();
    let mut evidence = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    for file in rust_files(&repo.path, MAX_FILES) {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        // Test fixtures routinely embed lookalike-CLI source as string
        // literals (this crate's own tests do exactly that) — scanning
        // through them produces confident-looking garbage, so cut
        // `#[cfg(test)]` modules out before looking for real enums.
        let content = strip_test_modules(&content);
        for enum_block in find_subcommand_enums(&content) {
            for variant in parse_variants(&enum_block.body) {
                let name = to_kebab_case(&variant.identifier);
                if name.is_empty() || !seen_names.insert(name.clone()) {
                    continue;
                }

                interfaces.push(Interface {
                    id: format!("{}:{}", repo.id, name),
                    repository: repo.id.clone(),
                    name: name.clone(),
                    description: variant.doc,
                });
                evidence.push(Evidence::new(
                    file.display().to_string(),
                    SourceType::Filesystem,
                    repo.id.clone(),
                    "cli_interface_extractor",
                    Confidence::ProbabilisticInference(0.7),
                    format!("enum {} variant {}", enum_block.enum_name, variant.identifier),
                    format!(
                        "{} exposes CLI subcommand `{name}` (heuristic: found in \
                         #[derive(Subcommand)] enum {}; not verified against runtime --help)",
                        repo.id, enum_block.enum_name
                    ),
                ));
            }
        }
    }

    (interfaces, evidence)
}

fn rust_files(root: &Path, max_files: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if found.len() >= max_files {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || SKIP_DIRS.contains(&name) {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                found.push(path);
                if found.len() >= max_files {
                    break;
                }
            }
        }
    }
    found
}

/// Replace the body of every `#[cfg(test)] mod ... { ... }` block with
/// blanks (preserving newlines, so line numbers of everything else stay
/// correct), so later scanning never sees inside test modules.
fn strip_test_modules(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();

    let mut line_starts = vec![0usize];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let line_text = |from: usize, to: usize| -> String { chars[from..to.min(chars.len())].iter().collect() };

    let mut blank_ranges: Vec<(usize, usize)> = Vec::new();

    for li in 0..line_starts.len() {
        let start = line_starts[li];
        let end = *line_starts.get(li + 1).unwrap_or(&chars.len());
        let line = line_text(start, end);
        if !line.contains("cfg(test)") {
            continue;
        }

        for lj in (li + 1)..line_starts.len().min(li + 6) {
            let s = line_starts[lj];
            let e = *line_starts.get(lj + 1).unwrap_or(&chars.len());
            let trimmed = line_text(s, e).trim().to_string();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.contains("mod ") {
                if let Some(open_rel) = chars[s..].iter().position(|&c| c == '{') {
                    let open_idx = s + open_rel;
                    if let Some(close_idx) = find_matching_brace(&chars, open_idx) {
                        blank_ranges.push((start, close_idx + 1));
                    }
                }
            }
            break;
        }
    }

    let mut out = chars;
    let len = out.len();
    for (s, e) in blank_ranges {
        for c in &mut out[s..e.min(len)] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    }
    out.into_iter().collect()
}

struct EnumBlock {
    enum_name: String,
    body: String,
}

/// Scan `content` line by line for `#[derive(...Subcommand...)]`, then
/// look up to 10 lines ahead (skipping blank/comment/attribute lines) for
/// the `enum Name {` it applies to, and extract the balanced-brace body.
fn find_subcommand_enums(content: &str) -> Vec<EnumBlock> {
    let chars: Vec<char> = content.chars().collect();

    let mut line_starts = vec![0usize];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let line_text = |from: usize, to: usize| -> String {
        chars[from..to.min(chars.len())].iter().collect()
    };

    let mut results = Vec::new();
    for li in 0..line_starts.len() {
        let start = line_starts[li];
        let end = *line_starts.get(li + 1).unwrap_or(&chars.len());
        let line = line_text(start, end);
        if !(line.contains("derive(") && line.contains("Subcommand")) {
            continue;
        }

        let mut enum_name = None;
        let mut enum_line_start = None;
        for lj in (li + 1)..line_starts.len().min(li + 11) {
            let s = line_starts[lj];
            let e = *line_starts.get(lj + 1).unwrap_or(&chars.len());
            let trimmed = line_text(s, e).trim().to_string();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed
                .strip_prefix("pub enum ")
                .or_else(|| trimmed.strip_prefix("enum "))
            {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    enum_name = Some(name);
                    enum_line_start = Some(s);
                }
            }
            break;
        }

        if let (Some(name), Some(from)) = (enum_name, enum_line_start) {
            if let Some(open_rel) = chars[from..].iter().position(|&c| c == '{') {
                let open_idx = from + open_rel;
                if let Some(body) = extract_balanced_block(&chars, open_idx) {
                    results.push(EnumBlock { enum_name: name, body });
                }
            }
        }
    }
    results
}

/// `chars[open_idx]` must be `{`. Returns the text strictly between the
/// matching `{`/`}` pair, tracking string/char literals so braces inside
/// them don't throw off the depth count.
///
/// Known limitation: this does not special-case raw strings (`r#"..."#`)
/// — a `"` inside one is still treated as a literal-string boundary, so a
/// raw string containing example/fixture source code (exactly the shape
/// of a `#[cfg(test)]` fixture) can confuse the depth count. That's why
/// `extract` strips `#[cfg(test)]` modules before scanning rather than
/// relying on this to see through them.
fn extract_balanced_block(chars: &[char], open_idx: usize) -> Option<String> {
    find_matching_brace(chars, open_idx).map(|close_idx| chars[open_idx + 1..close_idx].iter().collect())
}

/// `chars[open_idx]` must be `{`. Returns the index of the matching `}`.
fn find_matching_brace(chars: &[char], open_idx: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut in_char = false;
    let mut in_line_comment = false;
    let mut escape = false;
    let mut raw_string_hashes: Option<usize> = None;
    let mut i = open_idx;

    while i < chars.len() {
        let c = chars[i];

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if let Some(n) = raw_string_hashes {
            if c == '"' && chars[i + 1..].iter().take(n).all(|&h| h == '#') {
                i += 1 + n;
                raw_string_hashes = None;
                continue;
            }
            i += 1;
            continue;
        }

        if escape {
            escape = false;
            i += 1;
            continue;
        }
        if in_string {
            match c {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        if in_char {
            match c {
                '\\' => escape = true,
                '\'' => in_char = false,
                _ => {}
            }
            i += 1;
            continue;
        }

        if c == '/' && chars.get(i + 1) == Some(&'/') {
            in_line_comment = true;
            i += 2;
            continue;
        }

        // `r"..."`, `r#"..."#`, `r##"..."##`, ... — the number of `#`s
        // must match on both ends, and nothing inside (quotes, braces)
        // counts until the matching close is found.
        if c == 'r' {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                raw_string_hashes = Some(hashes);
                i = j + 1;
                continue;
            }
        }

        if c == '\'' {
            // Distinguish a char literal ('a', '\n', '\'') from a
            // lifetime ('a, 'static) — a lifetime has no closing quote
            // nearby. Without this, `&'a str` inside a variant would
            // desync the depth counter for the rest of the file.
            let is_char_literal = if chars.get(i + 1) == Some(&'\\') {
                (2..=4).any(|off| chars.get(i + off) == Some(&'\''))
            } else {
                chars.get(i + 2) == Some(&'\'')
            };
            if is_char_literal {
                in_char = true;
            }
            i += 1;
            continue;
        }

        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

struct Variant {
    identifier: String,
    doc: Option<String>,
}

/// Split an enum body into top-level (brace/paren/bracket-depth-0) comma
/// separated variant chunks, then pull the identifier and any leading
/// `///` doc comment out of each.
fn parse_variants(body: &str) -> Vec<Variant> {
    split_top_level(body)
        .iter()
        .filter_map(|chunk| parse_variant_chunk(chunk))
        .collect()
}

fn split_top_level(body: &str) -> Vec<String> {
    // A comma inside a `///` doc comment (extremely common in real code —
    // even this codebase's own doc comments have them) must not split a
    // variant. Line comments, raw strings, and the char-literal-vs-lifetime
    // ambiguity all need the same handling `find_matching_brace` uses, so
    // this mirrors that state machine rather than the naive char-by-char
    // version this replaced (which broke on exactly that case).
    let chars: Vec<char> = body.chars().collect();
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut in_char = false;
    let mut in_line_comment = false;
    let mut escape = false;
    let mut raw_string_hashes: Option<usize> = None;
    let mut current = String::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if in_line_comment {
            current.push(c);
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }

        if let Some(n) = raw_string_hashes {
            current.push(c);
            if c == '"' && chars[i + 1..].iter().take(n).all(|&h| h == '#') {
                for _ in 0..n {
                    i += 1;
                    current.push(chars[i]);
                }
                raw_string_hashes = None;
            }
            i += 1;
            continue;
        }

        if escape {
            current.push(c);
            escape = false;
            i += 1;
            continue;
        }
        if in_string {
            current.push(c);
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if in_char {
            current.push(c);
            if c == '\\' {
                escape = true;
            } else if c == '\'' {
                in_char = false;
            }
            i += 1;
            continue;
        }

        if c == '/' && chars.get(i + 1) == Some(&'/') {
            in_line_comment = true;
            current.push(c);
            current.push('/');
            i += 2;
            continue;
        }

        if c == 'r' {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                raw_string_hashes = Some(hashes);
                current.push('r');
                for _ in 0..hashes {
                    current.push('#');
                }
                current.push('"');
                i = j + 1;
                continue;
            }
        }

        if c == '\'' {
            let is_char_literal = if chars.get(i + 1) == Some(&'\\') {
                (2..=4).any(|off| chars.get(i + off) == Some(&'\''))
            } else {
                chars.get(i + 2) == Some(&'\'')
            };
            current.push(c);
            if is_char_literal {
                in_char = true;
            }
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                current.push(c);
            }
            '{' | '(' | '[' => {
                depth += 1;
                current.push(c);
            }
            '}' | ')' | ']' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                result.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        result.push(current);
    }
    result
}

fn parse_variant_chunk(chunk: &str) -> Option<Variant> {
    let mut doc_lines = Vec::new();
    let mut identifier = None;

    for raw_line in chunk.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("///") {
            doc_lines.push(rest.trim().to_string());
            continue;
        }
        if line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let name: String = line.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            identifier = Some(name);
        }
        break;
    }

    identifier.map(|id| Variant {
        identifier: id,
        doc: if doc_lines.is_empty() {
            None
        } else {
            Some(doc_lines.join(" "))
        },
    })
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
