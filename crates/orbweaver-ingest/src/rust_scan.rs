//! Shared text-pattern scanning primitives for Rust source, used by both
//! `interfaces` (CLI subcommand extraction) and `schemas` (serde struct
//! extraction). No proc-macro expansion, no `cargo build`, no execution —
//! and deliberately no attempt at a full lexer either. What's here is the
//! minimum needed to not be fooled by the things that actually showed up
//! while dogfooding `interfaces` on this codebase: raw strings, doc
//! comments containing commas, and `#[cfg(test)]` fixtures containing
//! lookalike source as string literals.

use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "vendor", "dist", "build"];

pub fn rust_files(root: &Path, max_files: usize) -> Vec<PathBuf> {
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
/// correct), so later scanning never sees inside test modules. Test
/// fixtures routinely embed lookalike source as string literals — this
/// crate's own tests do exactly that — and scanning through them produces
/// confident-looking garbage.
pub fn strip_test_modules(content: &str) -> String {
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

pub struct AnnotatedBlock {
    pub name: String,
    /// The item's own `///` doc comment, if any (not a field/variant doc
    /// — see [`leading_doc_and_identifier`] for those).
    pub doc: Option<String>,
    pub body: String,
}

/// Scan `content` line by line for a `#[derive(...)]` attribute where
/// `derive_matches` returns true on the attribute line, then look up to
/// 10 lines ahead (skipping blank/comment/attribute lines) for the
/// `<keyword> Name {` it applies to (`keyword` is `"enum"` or
/// `"struct"`), and extract the balanced-brace body.
pub fn find_annotated_blocks(
    content: &str,
    keyword: &str,
    derive_matches: impl Fn(&str) -> bool,
) -> Vec<AnnotatedBlock> {
    let chars: Vec<char> = content.chars().collect();

    let mut line_starts = vec![0usize];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            line_starts.push(i + 1);
        }
    }
    let line_text = |from: usize, to: usize| -> String { chars[from..to.min(chars.len())].iter().collect() };

    let prefix_pub = format!("pub {keyword} ");
    let prefix = format!("{keyword} ");

    let mut results = Vec::new();
    for li in 0..line_starts.len() {
        let start = line_starts[li];
        let end = *line_starts.get(li + 1).unwrap_or(&chars.len());
        let line = line_text(start, end);
        if !(line.contains("derive(") && derive_matches(&line)) {
            continue;
        }

        let mut item_name = None;
        let mut item_line_start = None;
        for lj in (li + 1)..line_starts.len().min(li + 11) {
            let s = line_starts[lj];
            let e = *line_starts.get(lj + 1).unwrap_or(&chars.len());
            let trimmed = line_text(s, e).trim().to_string();
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if let Some(rest) = trimmed
                .strip_prefix(prefix_pub.as_str())
                .or_else(|| trimmed.strip_prefix(prefix.as_str()))
            {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    item_name = Some(name);
                    item_line_start = Some(s);
                }
            }
            break;
        }

        if let (Some(name), Some(from)) = (item_name, item_line_start) {
            if let Some(open_rel) = chars[from..].iter().position(|&c| c == '{') {
                let open_idx = from + open_rel;
                if let Some(body) = extract_balanced_block(&chars, open_idx) {
                    let doc = item_doc_comment(&line_starts, &chars, li, &line_text);
                    results.push(AnnotatedBlock { name, doc, body });
                }
            }
        }
    }
    results
}

/// The item's own `///` doc comment, scanning backward from the line
/// carrying the matched `#[derive(...)]` attribute: skip over any other
/// stacked `#[...]` attribute lines directly above it, then collect the
/// contiguous run of `///` lines above that, in source order.
fn item_doc_comment(
    line_starts: &[usize],
    chars: &[char],
    derive_line: usize,
    line_text: &impl Fn(usize, usize) -> String,
) -> Option<String> {
    let mut lj = derive_line;
    while lj > 0 {
        let s = line_starts[lj - 1];
        let e = *line_starts.get(lj).unwrap_or(&chars.len());
        if !line_text(s, e).trim().starts_with('#') {
            break;
        }
        lj -= 1;
    }

    let mut doc_rev = Vec::new();
    while lj > 0 {
        let s = line_starts[lj - 1];
        let e = *line_starts.get(lj).unwrap_or(&chars.len());
        let trimmed = line_text(s, e).trim().to_string();
        let Some(rest) = trimmed.strip_prefix("///") else {
            break;
        };
        doc_rev.push(rest.trim().to_string());
        lj -= 1;
    }

    if doc_rev.is_empty() {
        None
    } else {
        doc_rev.reverse();
        Some(doc_rev.join(" "))
    }
}

/// `chars[open_idx]` must be `{`. Returns the text strictly between the
/// matching `{`/`}` pair.
fn extract_balanced_block(chars: &[char], open_idx: usize) -> Option<String> {
    find_matching_brace(chars, open_idx).map(|close_idx| chars[open_idx + 1..close_idx].iter().collect())
}

/// `chars[open_idx]` must be `{`. Returns the index of the matching `}`,
/// tracking string/char literals, raw strings, and line comments so none
/// of their contents (quotes, braces, commas) throw off the depth count.
pub fn find_matching_brace(chars: &[char], open_idx: usize) -> Option<usize> {
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
            // nearby. Without this, `&'a str` would desync the depth
            // counter for the rest of the file.
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

/// Split a block body into top-level (brace/paren/bracket-depth-0) comma
/// separated chunks (enum variants, struct fields, ...), aware of string
/// literals, raw strings, char-literal-vs-lifetime, and line comments —
/// in particular, a comma *inside a `///` doc comment* (routine in real
/// code, including this codebase's own) must not split a chunk.
pub fn split_top_level(body: &str) -> Vec<String> {
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

/// Pull a leading run of `///` doc-comment lines and the following bare
/// identifier (stopping at the first non-alphanumeric/underscore char) out
/// of a chunk produced by [`split_top_level`], skipping blank lines,
/// plain `//` comments, and `#[...]` attribute lines in between.
pub fn leading_doc_and_identifier(chunk: &str) -> (Option<String>, Option<String>) {
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

    let doc = if doc_lines.is_empty() { None } else { Some(doc_lines.join(" ")) };
    (doc, identifier)
}
