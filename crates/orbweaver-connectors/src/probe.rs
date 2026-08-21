//! Probing a single tool once a candidate binary has been found: runs
//! only read-only, well-known-safe invocations (`--help`, `--version`,
//! and — only if `--help` itself listed a self-description command like
//! `capabilities` — that command). Nothing here ever runs a command that
//! wasn't first confirmed to exist by inspecting `--help` output.

use crate::process::{self, ProcessOutput};
use orbweaver_evidence::Availability;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const SELF_DESCRIBE_NAMES: &[&str] = &["capabilities", "capability", "manifest", "describe"];

#[derive(Debug, Clone, Serialize)]
pub struct ConnectorReport {
    pub tool: String,
    pub binary: String,
    pub availability: Availability<ConnectorDetails>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectorDetails {
    pub binary_path: std::path::PathBuf,
    pub version: Option<String>,
    pub commands: Vec<DiscoveredCommand>,
    pub discovery_method: DiscoveryMethod,
    /// Present only when a self-description command (found in the
    /// `commands` list) was invoked and returned parseable JSON.
    pub capability_manifest: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredCommand {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DiscoveryMethod {
    /// The tool advertised a machine-readable capability manifest and we
    /// successfully parsed it as JSON.
    JsonCapabilityManifest,
    /// Parsed from the `Commands:`/`COMMANDS:` section of `--help` text —
    /// a heuristic, not a verified contract.
    HelpTextHeuristic,
}

/// Try each candidate binary name in order, probing every one actually
/// found on PATH — robust matching can turn up more than the
/// traditional `{tool}`/`{tool}-cli` pair (e.g. a repo whose real
/// declared bin name differs from the repo name, per `.orb` hints or
/// locally-known capability data). If none are found, returns a single
/// `Unavailable` report summarising what was tried, rather than one per
/// candidate — a wall of "not found" lines for every name variant isn't
/// useful.
pub fn probe_candidates(tool: &str, candidates: &[String], repo_path: Option<&Path>) -> Vec<ConnectorReport> {
    let mut seen = HashSet::new();
    let ordered: Vec<&String> = candidates.iter().filter(|c| seen.insert(c.as_str())).collect();

    let mut found = Vec::new();
    for candidate in &ordered {
        if process::which(candidate).is_some() {
            found.push(probe_binary(tool, candidate, repo_path));
        }
    }

    if found.is_empty() {
        let tried = ordered.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        found.push(ConnectorReport {
            tool: tool.to_string(),
            binary: tool.to_string(),
            availability: Availability::Unavailable {
                reason: format!("no matching binary found on PATH (tried: {tried})"),
            },
        });
    }
    found
}

pub fn probe_binary(tool: &str, binary_name: &str, repo_path: Option<&Path>) -> ConnectorReport {
    let Some(binary_path) = process::which(binary_name) else {
        return ConnectorReport {
            tool: tool.to_string(),
            binary: binary_name.to_string(),
            availability: Availability::Unavailable {
                reason: format!("no `{binary_name}` binary found on PATH"),
            },
        };
    };

    // --help is accepted even on a nonzero exit: several real tools here
    // (cambrian, goglz) require a positional/required arg and so exit
    // nonzero even for --help, but still print genuine help text.
    let Some(help_output) =
        process::run_with_timeout(&binary_path, &["--help"], repo_path, process::DEFAULT_TIMEOUT)
    else {
        return ConnectorReport {
            tool: tool.to_string(),
            binary: binary_name.to_string(),
            availability: Availability::Unavailable {
                reason: format!(
                    "`{binary_name} --help` did not return within {}s or failed to run",
                    process::DEFAULT_TIMEOUT.as_secs()
                ),
            },
        };
    };

    let (mut version, commands) = parse_help_text(&help_output.text);
    if version.is_none() {
        version = accept_version_output(process::run_with_timeout(
            &binary_path,
            &["--version"],
            repo_path,
            process::DEFAULT_TIMEOUT,
        ));
    }

    let mut discovery_method = DiscoveryMethod::HelpTextHeuristic;
    let mut capability_manifest = None;

    if let Some(repo_path) = repo_path {
        if let Some(cmd) = commands
            .iter()
            .find(|c| SELF_DESCRIBE_NAMES.contains(&c.name.to_lowercase().as_str()))
        {
            if let Some(output) = process::run_with_timeout(
                &binary_path,
                &[cmd.name.as_str()],
                Some(repo_path),
                process::DEFAULT_TIMEOUT,
            ) {
                if output.success && output.text.len() <= MAX_MANIFEST_BYTES {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&output.text) {
                        capability_manifest = Some(json);
                        discovery_method = DiscoveryMethod::JsonCapabilityManifest;
                    }
                }
            }
        }
    }

    ConnectorReport {
        tool: tool.to_string(),
        binary: binary_name.to_string(),
        availability: Availability::Known(ConnectorDetails {
            binary_path,
            version,
            commands,
            discovery_method,
            capability_manifest,
        }),
    }
}

/// Decide whether a `--version` invocation's output is trustworthy: it
/// must have exited successfully (a tool that doesn't support the flag —
/// cambrian, goglz, hellhound in this estate — exits nonzero and prints
/// an "unexpected argument" error, which is not a version) and look like
/// a short, single-purpose string rather than a stray usage dump.
fn accept_version_output(output: Option<ProcessOutput>) -> Option<String> {
    let output = output?;
    if !output.success {
        return None;
    }
    let text = output.text.trim();
    if text.is_empty() || text.len() >= 200 {
        return None;
    }
    Some(text.to_string())
}

/// Strip ANSI escape sequences (`\x1b[...m` etc.) — some ELCI tools use
/// hand-styled `--help` output with color codes that would otherwise
/// land in the middle of command names/descriptions.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Parse a `--help` transcript for a version line and a `Commands:` /
/// `COMMANDS:` section. Heuristic — real help text from real tools, not
/// a spec. Handles both plain clap-default output and hand-styled
/// output with a `VERSION` line and section headers ending in `:`.
fn parse_help_text(text: &str) -> (Option<String>, Vec<DiscoveredCommand>) {
    let clean = strip_ansi(text);

    let mut version = None;
    for line in clean.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("VERSION") {
            let v = rest.trim();
            if !v.is_empty() {
                version = Some(v.to_string());
            }
        }
    }

    let mut commands = Vec::new();
    let mut in_commands = false;
    for line in clean.lines() {
        let trimmed = line.trim();
        if !in_commands {
            if trimmed.eq_ignore_ascii_case("commands:") {
                in_commands = true;
            }
            continue;
        }

        if trimmed.is_empty() {
            break;
        }
        // A new top-level section (no leading whitespace, ends with ':')
        // ends the commands list.
        if !line.starts_with(char::is_whitespace) && trimmed.ends_with(':') {
            break;
        }

        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let Some(name) = parts.next() else { continue };
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        let description = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
        commands.push(DiscoveredCommand {
            name: name.to_string(),
            description,
        });
    }

    (version, commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_clap_default_help_text() {
        let text = "\
Deckhand keeps build artifacts clean.

Usage: deckhand [OPTIONS] <COMMAND>

Commands:
  clean   Run cargo clean across the workspace
  status  Report workspace sea-state (disk usage)
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
";
        let (version, commands) = parse_help_text(text);
        assert_eq!(version, None);
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].name, "clean");
        assert_eq!(commands[0].description.as_deref(), Some("Run cargo clean across the workspace"));
    }

    #[test]
    fn parses_hand_styled_help_with_ansi_and_version_line() {
        let text = "\u{1b}[1mPADAGONIA\u{1b}[0m\n\nVERSION    0.1.60\n\n\u{1b}[1mCOMMANDS:\u{1b}[0m\n    \u{1b}[36mingest  \u{1b}[0m Generate a synthetic graph\n    \u{1b}[36mbfs     \u{1b}[0m Run a breadth-first search\n\n\u{1b}[1mGLOBAL OPTIONS:\u{1b}[0m\n    -h, --help  Print this help message\n";
        let (version, commands) = parse_help_text(text);
        assert_eq!(version.as_deref(), Some("0.1.60"));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name, "ingest");
        assert_eq!(commands[0].description.as_deref(), Some("Generate a synthetic graph"));
        assert_eq!(commands[1].name, "bfs");
    }

    #[test]
    fn missing_binary_is_unavailable_with_reason_not_empty_capabilities() {
        let report = probe_binary("nonexistent-tool-xyz", "nonexistent-tool-xyz", None);
        match report.availability {
            Availability::Unavailable { reason } => assert!(reason.contains("PATH")),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    /// Regression test: this exact bug shipped once — a `--version`
    /// invocation on a tool that doesn't support the flag (cambrian,
    /// goglz, hellhound all reject it) exits nonzero and prints clap's
    /// "unexpected argument" error to stderr, which the timeout runner's
    /// stdout-empty fallback then returned as if it were legitimate
    /// output. That error text was displayed as the tool's "version".
    #[test]
    fn rejects_error_output_from_a_failed_version_flag() {
        let error_text = "error: unexpected argument '--version' found\n\n  tip: a similar argument exists: '--verbose'\n\nUsage: cambrian --repository <REPOSITORY> --verbose\n\nFor more information, try '--help'.".to_string();
        let failed = ProcessOutput {
            success: false,
            text: error_text,
        };
        assert_eq!(accept_version_output(Some(failed)), None);

        let real = ProcessOutput {
            success: true,
            text: "deckhand 0.21.37\n".to_string(),
        };
        assert_eq!(accept_version_output(Some(real)), Some("deckhand 0.21.37".to_string()));

        assert_eq!(accept_version_output(None), None);
    }

    #[test]
    fn probe_candidates_tries_every_match_and_reports_one_line_when_none_found() {
        let candidates = vec!["nonexistent-a".to_string(), "nonexistent-b".to_string()];
        let reports = probe_candidates("ghost-tool", &candidates, None);
        assert_eq!(reports.len(), 1);
        match &reports[0].availability {
            Availability::Unavailable { reason } => {
                assert!(reason.contains("nonexistent-a"));
                assert!(reason.contains("nonexistent-b"));
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }
}
