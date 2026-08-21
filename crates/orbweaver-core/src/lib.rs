//! Core ecosystem data model.
//!
//! Phase I: repositories, their manifests, and the dependency edges
//! deterministically observable from those manifests. Phase II adds
//! `Capability` — a conservative first cut at "what does this repository
//! actually do", built only from evidence already collected during
//! ingestion (declared binaries/scripts, manifest/README descriptions),
//! never from executing anything. Interface, Opportunity, Intervention
//! etc. (directive section 9) still wait on later phases — see
//! docs/ROADMAP.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type RepoId = String;

/// A dependency manifest format recognised during ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// The kind of capability inferred from a repository's declared entry
/// points. `Cli` means the repository declares at least one runnable
/// binary/script; `Library` means it has a manifest but no such entry
/// point — a much weaker claim, since we haven't examined its public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityKind {
    Cli,
    Library,
}

/// A first, conservative cut at "what does this repository do" (directive
/// section 12). Built only by aggregating evidence already gathered during
/// ingestion — never invented, never requires running the repository's
/// code. `evidence_sources` counts how many independent signals agree
/// (e.g. a declared bin target + a manifest description); a capability
/// backed by only one signal is a weaker claim than one backed by two,
/// and callers should treat the count as part of the claim, not discard it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub id: String,
    pub repository: RepoId,
    pub name: String,
    pub kind: CapabilityKind,
    pub description: Option<String>,
    pub evidence_sources: u32,
}

/// A single CLI subcommand surfaced by a repository, statically detected
/// from a `#[derive(Subcommand)]` enum in its Rust source (directive
/// section 9: `Capability --exposes--> Interface`). This is text-pattern
/// heuristic, not a real parse — it can miss non-derive-style CLIs and
/// can't see `#[command(name = "...")]` overrides — so every `Interface`
/// is backed by `Confidence::ProbabilisticInference`, never treated as
/// certain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interface {
    pub id: String,
    pub repository: RepoId,
    pub name: String,
    pub description: Option<String>,
}

/// A single field of a [`Schema`], as literally declared in source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaField {
    pub name: String,
    /// The type exactly as written (`Option<String>`, `Vec<RepoId>`,
    /// ...) — not resolved against its actual definition.
    pub type_repr: String,
}

/// A serde-derived data structure statically detected from Rust source
/// (directive section 9: schemas as evidence for what a capability
/// consumes/produces). Text-pattern heuristic like [`Interface`] — no
/// macro expansion, so it can't see fields added by other derives or
/// `#[serde(flatten)]` semantics, and it only recognises `struct { ... }`
/// bodies (tuple and unit structs are invisible to it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub id: String,
    pub repository: RepoId,
    pub name: String,
    pub description: Option<String>,
    pub fields: Vec<SchemaField>,
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
