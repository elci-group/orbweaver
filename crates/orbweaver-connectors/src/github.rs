//! GitHub org repository discovery — replaces a hardcoded tool-name list
//! with the actual current repository inventory. Uses the `gh` CLI
//! (already authenticated in this environment) for the metadata query;
//! any actual git operation (clone, for installs) uses the `sshUrl` this
//! returns and *only* that field — never an `https://` URL is
//! constructed, per the SSH-only requirement.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

use crate::process::run_with_timeout;

pub const ELCI_GITHUB_ORG: &str = "elci-group";
const GH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRepo {
    pub name: String,
    #[serde(rename = "sshUrl")]
    pub ssh_url: String,
    #[serde(rename = "isPrivate")]
    pub is_private: bool,
}

/// List every repository in `org`. Errors (network, auth, `gh` missing)
/// are surfaced explicitly rather than falling back to stale/guessed
/// data — directive section 33: "unavailable" must never collapse into
/// an empty-but-successful result.
pub fn discover_org_repos(org: &str) -> Result<Vec<GithubRepo>, String> {
    let gh_path = crate::process::which("gh")
        .ok_or_else(|| "`gh` not found on PATH — install the GitHub CLI to enable discovery".to_string())?;

    let output = run_with_timeout(
        &gh_path,
        &["repo", "list", org, "--limit", "1000", "--json", "name,sshUrl,isPrivate"],
        None,
        GH_TIMEOUT,
    )
    .ok_or_else(|| format!("`gh repo list {org}` did not return within {}s", GH_TIMEOUT.as_secs()))?;

    if !output.success {
        return Err(format!("`gh repo list {org}` failed: {}", output.text.trim()));
    }

    serde_json::from_str(&output.text).map_err(|e| format!("failed to parse `gh repo list {org}` output: {e}"))
}

/// The GitHub release asset naming convention observed across every real
/// ELCI release checked (kaptaind, padagonia, dreamseq): a Rust target
/// triple somewhere in the filename. Only Linux/macOS/Windows on
/// x86_64/aarch64 are covered — anything else reports no match, which
/// correctly falls through to a source build.
pub fn current_target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseAsset {
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseView {
    #[serde(rename = "tagName")]
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

/// The exact filename suffixes GitHub attaches to every release built by
/// the same pipeline that produces the binaries themselves (checksums,
/// signatures, SBOMs) — never the binary/archive we actually want.
const NON_BINARY_SUFFIXES: &[&str] = &[".sha256", ".sig", ".cert", ".spdx.json"];

/// Find a release asset matching the current platform for `org/repo`'s
/// latest release, if one exists. Returns the release tag and the
/// matched asset filename — downloading is a separate step so the
/// caller can decide where the file goes.
pub fn find_release_asset(org: &str, repo: &str) -> Option<(String, String)> {
    let gh_path = crate::process::which("gh")?;
    let triple = current_target_triple()?;

    let output = run_with_timeout(
        &gh_path,
        &["release", "view", "--repo", &format!("{org}/{repo}"), "--json", "tagName,assets"],
        None,
        GH_TIMEOUT,
    )?;
    if !output.success {
        return None; // no releases, or repo doesn't exist — not an error, just nothing to use
    }

    let view: ReleaseView = serde_json::from_str(&output.text).ok()?;
    let asset = view
        .assets
        .iter()
        .find(|a| a.name.contains(triple) && !NON_BINARY_SUFFIXES.iter().any(|s| a.name.ends_with(s)))?;

    Some((view.tag_name, asset.name.clone()))
}

/// Download one named asset from a release into `dest_dir` via `gh
/// release download`. `dest_dir` must already exist.
pub fn download_release_asset(
    org: &str,
    repo: &str,
    tag: &str,
    asset_name: &str,
    dest_dir: &Path,
) -> Result<std::path::PathBuf, String> {
    let gh_path = crate::process::which("gh").ok_or("`gh` not found on PATH")?;
    let output = run_with_timeout(
        &gh_path,
        &[
            "release",
            "download",
            tag,
            "--repo",
            &format!("{org}/{repo}"),
            "--pattern",
            asset_name,
            "--dir",
            dest_dir.to_str().ok_or("dest dir is not valid UTF-8")?,
            "--clobber",
        ],
        None,
        Duration::from_secs(120),
    )
    .ok_or("`gh release download` timed out")?;

    if !output.success {
        return Err(format!("`gh release download` failed: {}", output.text.trim()));
    }

    Ok(dest_dir.join(asset_name))
}
