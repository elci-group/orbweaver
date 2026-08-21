//! `.orb` — a project's own opt-in declaration that it's ELCI tooling
//! Orbweaver should discover, probe, and (if `install.installable`) build
//! or install on demand. This replaces treating every repository in the
//! GitHub org as a connector candidate: scope is decided by each
//! project's own maintainers via a file *in that project*, not by a list
//! hardcoded in Orbweaver or by guessing from repo names.
//!
//! Only checked locally (in an already-known repository's working
//! directory — see `orbweaver_storage`'s repository paths from the last
//! scan). A repo that exists on GitHub but has never been cloned locally
//! can't be checked for `.orb` without cloning it first, which this
//! deliberately doesn't do speculatively for every one of a hundred-plus
//! org repos; it's a known limitation, not an oversight.
//!
//! # Format (TOML)
//!
//! ```toml
//! version = 1
//!
//! [tool]
//! binaries = ["kaptaind", "kaptaind-cli"]  # optional; defaults to [repo-name]
//! group = "release-management"             # optional, free-form
//!
//! [install]
//! installable = true              # may orbweaver acquire this when missing?
//! prefer_release_binary = true    # try a matching GitHub release asset first
//! ```
//!
//! `group` is accepted and carried through but not yet used for anything
//! — reserved for grouping/hierarchy features raised when this format
//! was proposed, not implemented until there's a concrete consumer.
//! Cross-project compatibility/concurrency conditions are not part of
//! this format yet at all; adding them speculatively before any feature
//! reads them would just be unused schema.

use serde::Deserialize;
use std::fs;
use std::path::Path;

pub const ORB_FILENAME: &str = ".orb";

#[derive(Debug, Clone, Deserialize)]
pub struct OrbFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub tool: OrbTool,
    #[serde(default)]
    pub install: OrbInstall,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OrbTool {
    /// Explicit binary name hints, in priority order. Empty means "use
    /// the repo name and `{repo name}-cli`" (the same default every tool
    /// got before `.orb` existed).
    #[serde(default)]
    pub binaries: Vec<String>,
    /// Reserved for future grouping/hierarchy features — carried through
    /// but not read by anything yet.
    pub group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrbInstall {
    #[serde(default)]
    pub installable: bool,
    #[serde(default = "default_true")]
    pub prefer_release_binary: bool,
}

impl Default for OrbInstall {
    fn default() -> Self {
        Self {
            installable: false,
            prefer_release_binary: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Load and parse `.orb` from a repository's root, if present. `None`
/// covers both "no `.orb` file" and "`.orb` present but malformed" —
/// callers treat both the same way (out of scope), but a malformed file
/// should still be visible to whoever is debugging it, hence the eprintln
/// rather than silently swallowing a real syntax error.
pub fn load_local(repo_path: &Path) -> Option<OrbFile> {
    let content = fs::read_to_string(repo_path.join(ORB_FILENAME)).ok()?;
    match toml::from_str(&content) {
        Ok(orb) => Some(orb),
        Err(e) => {
            eprintln!(
                "warning: {} exists but failed to parse: {e}",
                repo_path.join(ORB_FILENAME).display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_orb_file() {
        let toml = r#"
version = 1

[tool]
binaries = ["kaptaind", "kaptaind-cli"]
group = "release-management"

[install]
installable = true
prefer_release_binary = true
"#;
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".orb"), toml).unwrap();

        let orb = load_local(dir.path()).unwrap();
        assert_eq!(orb.version, 1);
        assert_eq!(orb.tool.binaries, vec!["kaptaind", "kaptaind-cli"]);
        assert_eq!(orb.tool.group.as_deref(), Some("release-management"));
        assert!(orb.install.installable);
        assert!(orb.install.prefer_release_binary);
    }

    #[test]
    fn defaults_are_conservative_when_sections_are_omitted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".orb"), "version = 1\n").unwrap();

        let orb = load_local(dir.path()).unwrap();
        assert!(orb.tool.binaries.is_empty());
        // Not installable unless explicitly opted in — a bare `.orb`
        // marks a repo as a connector to probe, not as something
        // Orbweaver may build and install unattended.
        assert!(!orb.install.installable);
    }

    #[test]
    fn missing_file_is_none_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_local(dir.path()).is_none());
    }

    #[test]
    fn malformed_file_is_none_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".orb"), "this is not valid toml {{{").unwrap();
        assert!(load_local(dir.path()).is_none());
    }
}
