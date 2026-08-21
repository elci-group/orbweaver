//! Repository discovery: find candidate project directories under a root
//! without hard-coding a repository inventory (directive section 4.3 —
//! "the repository inventory should itself be discovered rather than
//! hard-coded").
//!
//! Phase I scans one level deep only. A directory counts as a repository
//! candidate if it directly contains a recognised manifest or a `.git`
//! entry. This deliberately avoids a recursive walk of the whole
//! filesystem: a home directory contains caches, media libraries and
//! unrelated personal files that are not part of the ecosystem graph.

use std::fs;
use std::path::{Path, PathBuf};

const MARKER_FILES: &[&str] = ["Cargo.toml", "package.json", "pyproject.toml", "go.mod"].as_slice();

const SKIP_NAMES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    "vendor",
    "storage",
    "backups",
    "media",
    "cache",
    "Music",
    "Pictures",
    "Videos",
    "Downloads",
    "Documents",
    "Desktop",
    "Android",
];

pub fn discover_repositories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();

        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') || SKIP_NAMES.contains(&file_name) {
            continue;
        }

        // Skip symlinks: they risk scan loops and don't represent a
        // distinct filesystem location worth treating as its own repo.
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }

        if is_repository_candidate(&path) {
            found.push(path);
        }
    }

    found.sort();
    Ok(found)
}

fn is_repository_candidate(path: &Path) -> bool {
    if path.join(".git").exists() {
        return true;
    }
    MARKER_FILES.iter().any(|marker| path.join(marker).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_repos_by_manifest_or_git_and_skips_noise() {
        let root = tempfile::tempdir().unwrap();

        // A real Cargo repo.
        let cargo_repo = root.path().join("widget");
        fs::create_dir(&cargo_repo).unwrap();
        fs::write(cargo_repo.join("Cargo.toml"), "[package]\nname=\"widget\"").unwrap();

        // A real git repo with no manifest.
        let git_repo = root.path().join("some-git-repo");
        fs::create_dir(&git_repo).unwrap();
        fs::create_dir(git_repo.join(".git")).unwrap();

        // A hidden directory — must be skipped even though it has a marker.
        let hidden = root.path().join(".hidden");
        fs::create_dir(&hidden).unwrap();
        fs::write(hidden.join("Cargo.toml"), "[package]\nname=\"hidden\"").unwrap();

        // A denylisted noise directory — must be skipped even with a marker.
        let noise = root.path().join("node_modules");
        fs::create_dir(&noise).unwrap();
        fs::write(noise.join("package.json"), "{}").unwrap();

        // An empty directory with no marker at all.
        let empty = root.path().join("just-a-folder");
        fs::create_dir(&empty).unwrap();

        let found = discover_repositories(root.path()).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, vec!["some-git-repo", "widget"]);
    }

    #[test]
    fn skips_symlinked_directories() {
        let root = tempfile::tempdir().unwrap();

        let real_repo = root.path().join("real-widget");
        fs::create_dir(&real_repo).unwrap();
        fs::write(real_repo.join("Cargo.toml"), "[package]\nname=\"real\"").unwrap();

        let link = root.path().join("linked-widget");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_repo, &link).unwrap();

        let found = discover_repositories(root.path()).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, vec!["real-widget"]);
    }
}
