//! Acquiring a missing tool on demand: a matching GitHub release binary
//! is preferred when one exists (no compute cost — just a download), a
//! source build via `baby` is the fallback. Both paths are real
//! subprocess/filesystem operations, gated entirely on `.orb` having
//! opted the repo in (`install.installable = true`) — see `orb.rs`.

use crate::{github, process};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InstallMethod {
    /// Downloaded a prebuilt binary from a matching GitHub release —
    /// avoids the compute/time cost of a source build.
    ReleaseBinary,
    /// Cloned the repo (SSH) and built it locally via `baby --user`.
    SourceBuild,
}

#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub method: InstallMethod,
    pub installed_path: PathBuf,
}

pub fn default_cache_dir() -> PathBuf {
    home_dir().join(".local/share/orbweaver/sources")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn default_install_dir() -> PathBuf {
    home_dir().join(".local/bin")
}

/// Acquire `binary_name` for `repo_name` (a GitHub repo in `org`),
/// preferring a matching release asset over a source build when
/// `prefer_release` is set. Falls back to source automatically if no
/// matching release exists or the download fails — a repo without
/// binary releases isn't an error, it's the common case.
///
/// `local_repo_path`, when given, is a directory this repo is already
/// known to be checked out at (from a prior `orbweaver scan`) — the
/// source build runs there directly rather than cloning a second,
/// redundant copy into `cache_dir`. `.orb` can currently only ever be
/// checked on repos already known locally, so in practice this is
/// always `Some`; `cache_dir`/SSH clone exists for when that stops being
/// true (e.g. a future remote `.orb` check).
pub fn jit_install(
    org: &str,
    repo_name: &str,
    binary_name: &str,
    prefer_release: bool,
    ssh_url: &str,
    cache_dir: &Path,
    local_repo_path: Option<&Path>,
) -> Result<InstallOutcome, String> {
    if prefer_release {
        match install_from_release(org, repo_name, binary_name) {
            Ok(outcome) => return Ok(outcome),
            Err(e) => eprintln!(
                "info: no usable release binary for {repo_name} ({e}) — falling back to a source build via baby"
            ),
        }
    }
    install_from_source(repo_name, ssh_url, cache_dir, local_repo_path)
}

fn install_from_release(org: &str, repo_name: &str, binary_name: &str) -> Result<InstallOutcome, String> {
    let (tag, asset_name) =
        github::find_release_asset(org, repo_name).ok_or("no release asset matches this platform")?;

    let tmp = tempfile::tempdir().map_err(|e| format!("failed to create a temp dir: {e}"))?;
    let downloaded = github::download_release_asset(org, repo_name, &tag, &asset_name, tmp.path())?;
    let binary_path = extract_binary(&downloaded, tmp.path(), binary_name)?;

    let install_dir = default_install_dir();
    fs::create_dir_all(&install_dir).map_err(|e| format!("failed to create {}: {e}", install_dir.display()))?;
    let dest = install_dir.join(binary_name);
    fs::copy(&binary_path, &dest).map_err(|e| format!("failed to install to {}: {e}", dest.display()))?;
    set_executable(&dest)?;

    Ok(InstallOutcome {
        method: InstallMethod::ReleaseBinary,
        installed_path: dest,
    })
}

/// Run a command to completion, capturing its output rather than
/// streaming it live — install operations run inside a spinner-animated
/// CLI, and interleaving raw child stdio with an actively redrawing
/// spinner produces garbled terminal output. On failure the error
/// carries a tail of what the command actually printed, so nothing
/// useful is lost, just deferred until there's something to report.
fn run_captured(binary: &str, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<(), String> {
    let output = process::run_with_timeout(Path::new(binary), args, cwd, timeout)
        .ok_or_else(|| format!("`{binary}` did not finish within {}s", timeout.as_secs()))?;
    if output.success {
        Ok(())
    } else {
        Err(format!("`{binary}` failed:\n{}", process::tail(&output.text, 2000)))
    }
}

fn install_from_source(
    repo_name: &str,
    ssh_url: &str,
    cache_dir: &Path,
    local_repo_path: Option<&Path>,
) -> Result<InstallOutcome, String> {
    let repo_dir: PathBuf = if let Some(local) = local_repo_path {
        local.to_path_buf()
    } else {
        if !ssh_url.starts_with("git@") {
            return Err(format!("refusing to clone a non-SSH remote: {ssh_url}"));
        }
        // Repo names come from GitHub's API and can't structurally
        // contain a path separator, but a filesystem path built from
        // network-sourced data should never trust that without checking.
        if repo_name.contains('/') || repo_name.contains("..") {
            return Err(format!("unsafe repo name, refusing to use it as a path component: {repo_name}"));
        }

        fs::create_dir_all(cache_dir).map_err(|e| format!("failed to create {}: {e}", cache_dir.display()))?;
        let dir = cache_dir.join(repo_name);
        if !dir.exists() {
            run_captured(
                "git",
                &["clone", "--depth", "1", ssh_url, path_str(&dir)?],
                None,
                Duration::from_secs(120),
            )
            .map_err(|e| format!("git clone of {ssh_url} failed: {e}"))?;
        }
        dir
    };

    if !repo_dir.join("Cargo.toml").exists() {
        return Err(format!("{repo_name} has no Cargo.toml — baby only builds Rust projects"));
    }

    let baby_path = process::which("baby").ok_or("`baby` not found on PATH")?;
    run_captured(path_str(&baby_path)?, &["--user"], Some(&repo_dir), Duration::from_secs(600))
        .map_err(|e| format!("`baby --user` failed for {repo_name}: {e}"))?;

    // baby installs a binary named after the crate; if the crate's
    // package name differs from the repo name this legitimately won't
    // be found — surfaced as an error, not assumed away.
    process::which(repo_name)
        .map(|installed_path| InstallOutcome {
            method: InstallMethod::SourceBuild,
            installed_path,
        })
        .ok_or_else(|| {
            format!(
                "baby reported success but no `{repo_name}` binary is on PATH afterward \
                 (check ~/.local/bin is on PATH, and that the crate's binary name matches the repo name)"
            )
        })
}

fn extract_binary(downloaded: &Path, tmp_dir: &Path, binary_name: &str) -> Result<PathBuf, String> {
    let name = downloaded.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extract_dir = tmp_dir.join("extracted");

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
        run_captured(
            "tar",
            &["xzf", path_str(downloaded)?, "-C", path_str(&extract_dir)?],
            None,
            Duration::from_secs(60),
        )
        .map_err(|e| format!("tar extraction failed: {e}"))?;
        find_binary_in_dir(&extract_dir, binary_name)
            .ok_or_else(|| format!("couldn't find `{binary_name}` inside the extracted archive"))
    } else if name.ends_with(".zip") {
        fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
        run_captured(
            "unzip",
            &["-oq", path_str(downloaded)?, "-d", path_str(&extract_dir)?],
            None,
            Duration::from_secs(60),
        )
        .map_err(|e| format!("unzip extraction failed: {e}"))?;
        find_binary_in_dir(&extract_dir, binary_name)
            .ok_or_else(|| format!("couldn't find `{binary_name}` inside the extracted archive"))
    } else {
        // A raw binary asset with no archive extension (e.g. padagonia's
        // release assets are the executable itself, no wrapping tarball).
        Ok(downloaded.to_path_buf())
    }
}

/// Bounded breadth-first search for a file named exactly `target_name`
/// under `dir` — archives from different tools nest their binary at
/// different depths (flat, or inside a `{name}-{version}-{triple}/`
/// directory), so this doesn't assume a layout.
fn find_binary_in_dir(dir: &Path, target_name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    let mut visited = 0u32;
    while let Some(d) = stack.pop() {
        visited += 1;
        if visited > 2000 {
            break;
        }
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(target_name) {
                return Some(path);
            }
        }
    }
    None
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .map_err(|e| format!("failed to read metadata for {}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .map_err(|e| format!("failed to set executable permission on {}: {e}", path.display()))?;
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_binary_asset_passes_through_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let downloaded = tmp.path().join("padagonia-x86_64-unknown-linux-gnu");
        fs::write(&downloaded, b"fake binary contents").unwrap();

        let result = extract_binary(&downloaded, tmp.path(), "padagonia").unwrap();
        assert_eq!(result, downloaded);
    }

    #[test]
    fn finds_binary_nested_inside_extracted_archive_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("kaptaind-10.1.4-x86_64-unknown-linux-gnu");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("kaptaind"), b"fake binary").unwrap();
        fs::write(nested.join("README.md"), b"not the binary").unwrap();

        let found = find_binary_in_dir(tmp.path(), "kaptaind").unwrap();
        assert_eq!(found, nested.join("kaptaind"));
    }

    #[test]
    fn missing_binary_in_extracted_archive_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("README.md"), b"nothing useful here").unwrap();
        assert!(find_binary_in_dir(tmp.path(), "kaptaind").is_none());
    }

    #[test]
    fn refuses_to_clone_a_non_ssh_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install_from_source(
            "widget",
            "https://github.com/elci-group/widget.git",
            tmp.path(),
            None,
        )
        .unwrap_err();
        assert!(err.contains("SSH"));
    }

    #[test]
    fn refuses_unsafe_repo_names() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install_from_source(
            "../../etc",
            "git@github.com:elci-group/widget.git",
            tmp.path(),
            None,
        )
        .unwrap_err();
        assert!(err.contains("unsafe"));
    }

    #[test]
    fn builds_in_place_when_a_local_repo_path_is_already_known() {
        // With a local path given, no clone should be attempted even for
        // a non-SSH/unsafe-looking url — the clone-validation branch is
        // simply not reached, since there's nothing to clone.
        let tmp = tempfile::tempdir().unwrap();
        // No Cargo.toml in the local dir, so this should fail there
        // rather than on SSH/name validation.
        let err = install_from_source(
            "../../etc",
            "https://not-ssh-at-all",
            Path::new("/unused-cache-dir"),
            Some(tmp.path()),
        )
        .unwrap_err();
        assert!(err.contains("Cargo.toml"), "expected a Cargo.toml error, got: {err}");
    }
}
