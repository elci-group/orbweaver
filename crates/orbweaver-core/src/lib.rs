//! Core ecosystem data model.
//!
//! Phase I scope only: repositories, their manifests, and the dependency
//! edges deterministically observable from those manifests. Capability,
//! Interface, Opportunity, Intervention etc. (directive section 9) are
//! introduced in later phases once there is real extraction logic behind
//! them — see docs/ROADMAP.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type RepoId = String;

/// A dependency manifest format recognised during ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ManifestKind {
    Cargo,
    Npm,
    PyprojectPoetry,
    PyprojectPep621,
    Go,
}

/// A single declared dependency, as literally written in a manifest.
///
/// `resolved_repo` is filled in during graph construction when the
/// dependency name/path matches another discovered repository in the same
/// scan — that is what turns a manifest line into a `depends_on` edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRef {
    pub name: String,
    pub version_req: Option<String>,
    pub manifest: ManifestKind,
    /// True when the manifest points at a local path/workspace member
    /// rather than a registry package — strong evidence of an intentional
    /// internal relationship.
    pub is_path_dependency: bool,
    /// The raw local path as written in the manifest (Cargo `path = `,
    /// npm `file:`, go.mod `replace ... =>`), before resolution.
    pub path_hint: Option<String>,
    pub resolved_repo: Option<RepoId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: RepoId,
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub primary_language: Option<String>,
    pub manifests: Vec<ManifestKind>,
    pub license: Option<String>,
    pub readme_present: bool,
    pub is_git_repo: bool,
    pub default_branch: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub commit_count: Option<u64>,
    pub contributor_count: Option<u64>,
    pub dependencies: Vec<DependencyRef>,
}

/// An immutable point-in-time capture of the ecosystem (directive section 22).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub root: PathBuf,
    pub repositories: Vec<Repository>,
}

#[derive(Debug, thiserror::Error)]
pub enum OrbweaverError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest {path}: {message}")]
    ManifestParse { path: PathBuf, message: String },
    #[error("git error at {path}: {message}")]
    Git { path: PathBuf, message: String },
    #[error("storage error: {0}")]
    Storage(String),
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(String),
}

pub type Result<T> = std::result::Result<T, OrbweaverError>;
