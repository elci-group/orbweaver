//! Immutable snapshot persistence (directive section 22) on SQLite.
//! Local mode only; PostgreSQL/enterprise mode is Phase VIII.

use chrono::{DateTime, Utc};
use orbweaver_core::{
    Capability, CapabilityKind, DependencyRef, ManifestKind, OrbweaverError, Repository, Result,
};
use orbweaver_evidence::{Confidence, Evidence, SourceType};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub fn default_db_path() -> PathBuf {
    let base = dirs_home().join(".local/share/orbweaver");
    base.join("orbweaver.db")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn open(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| OrbweaverError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let conn = Connection::open(db_path)
        .map_err(|e| OrbweaverError::Storage(format!("failed to open {}: {e}", db_path.display())))?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            root TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS repositories (
            snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
            id TEXT NOT NULL,
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            description TEXT,
            primary_language TEXT,
            manifests_json TEXT NOT NULL,
            license TEXT,
            readme_present INTEGER NOT NULL,
            is_git_repo INTEGER NOT NULL,
            default_branch TEXT,
            last_commit_at TEXT,
            commit_count INTEGER,
            contributor_count INTEGER,
            PRIMARY KEY (snapshot_id, id)
        );

        CREATE TABLE IF NOT EXISTS dependencies (
            snapshot_id TEXT NOT NULL,
            repo_id TEXT NOT NULL,
            name TEXT NOT NULL,
            version_req TEXT,
            manifest TEXT NOT NULL,
            is_path_dependency INTEGER NOT NULL,
            path_hint TEXT,
            resolved_repo TEXT
        );

        CREATE TABLE IF NOT EXISTS capabilities (
            snapshot_id TEXT NOT NULL,
            id TEXT NOT NULL,
            repository TEXT NOT NULL,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            description TEXT,
            evidence_sources INTEGER NOT NULL,
            PRIMARY KEY (snapshot_id, id)
        );

        CREATE TABLE IF NOT EXISTS evidence (
            snapshot_id TEXT NOT NULL,
            id TEXT NOT NULL,
            source TEXT NOT NULL,
            source_type TEXT NOT NULL,
            repository TEXT NOT NULL,
            commit_hash TEXT,
            timestamp TEXT NOT NULL,
            extractor TEXT NOT NULL,
            confidence_json TEXT NOT NULL,
            raw_reference TEXT NOT NULL,
            derived_claim TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_deps_snapshot ON dependencies(snapshot_id, repo_id);
        CREATE INDEX IF NOT EXISTS idx_evidence_snapshot ON evidence(snapshot_id, repository);
        CREATE INDEX IF NOT EXISTS idx_capabilities_snapshot ON capabilities(snapshot_id, repository);
        "#,
    )
    .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    Ok(())
}

pub fn save_snapshot(
    conn: &mut Connection,
    snapshot_id: &str,
    root: &Path,
    repositories: &[Repository],
    capabilities: &[Capability],
    evidence: &[Evidence],
) -> Result<()> {
    let tx = conn
        .transaction()
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    tx.execute(
        "INSERT OR REPLACE INTO snapshots (id, created_at, root) VALUES (?1, ?2, ?3)",
        params![snapshot_id, Utc::now().to_rfc3339(), root.display().to_string()],
    )
    .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    for repo in repositories {
        let manifests_json = serde_json::to_string(&repo.manifests)
            .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

        tx.execute(
            "INSERT OR REPLACE INTO repositories
                (snapshot_id, id, name, path, description, primary_language, manifests_json,
                 license, readme_present, is_git_repo, default_branch, last_commit_at,
                 commit_count, contributor_count)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                snapshot_id,
                repo.id,
                repo.name,
                repo.path.display().to_string(),
                repo.description,
                repo.primary_language,
                manifests_json,
                repo.license,
                repo.readme_present as i64,
                repo.is_git_repo as i64,
                repo.default_branch,
                repo.last_commit_at.map(|t| t.to_rfc3339()),
                repo.commit_count.map(|c| c as i64),
                repo.contributor_count.map(|c| c as i64),
            ],
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

        for dep in &repo.dependencies {
            let manifest_str = serde_json::to_string(&dep.manifest)
                .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
            tx.execute(
                "INSERT INTO dependencies
                    (snapshot_id, repo_id, name, version_req, manifest, is_path_dependency,
                     path_hint, resolved_repo)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    snapshot_id,
                    repo.id,
                    dep.name,
                    dep.version_req,
                    manifest_str,
                    dep.is_path_dependency as i64,
                    dep.path_hint,
                    dep.resolved_repo,
                ],
            )
            .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
        }
    }

    for cap in capabilities {
        let kind_str = match cap.kind {
            CapabilityKind::Cli => "Cli",
            CapabilityKind::Library => "Library",
        };
        tx.execute(
            "INSERT OR REPLACE INTO capabilities
                (snapshot_id, id, repository, name, kind, description, evidence_sources)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                snapshot_id,
                cap.id,
                cap.repository,
                cap.name,
                kind_str,
                cap.description,
                cap.evidence_sources,
            ],
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    }

    for ev in evidence {
        let confidence_json = serde_json::to_string(&ev.confidence)
            .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
        tx.execute(
            "INSERT INTO evidence
                (snapshot_id, id, source, source_type, repository, commit_hash, timestamp,
                 extractor, confidence_json, raw_reference, derived_claim)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                snapshot_id,
                ev.id.to_string(),
                ev.source,
                format!("{:?}", ev.source_type),
                ev.repository,
                ev.commit,
                ev.timestamp.to_rfc3339(),
                ev.extractor,
                confidence_json,
                ev.raw_reference,
                ev.derived_claim,
            ],
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    }

    tx.commit().map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    Ok(())
}

pub fn latest_snapshot_id(conn: &Connection) -> Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM snapshots ORDER BY created_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(OrbweaverError::Storage(e.to_string())),
    })
}

pub struct SnapshotSummary {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub root: String,
}

pub fn list_snapshots(conn: &Connection) -> Result<Vec<SnapshotSummary>> {
    let mut stmt = conn
        .prepare("SELECT id, created_at, root FROM snapshots ORDER BY created_at DESC")
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let created_at: String = row.get(1)?;
            Ok(SnapshotSummary {
                id: row.get(0)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                root: row.get(2)?,
            })
        })
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| OrbweaverError::Storage(e.to_string()))
}

pub fn load_snapshot(
    conn: &Connection,
    snapshot_id: &str,
) -> Result<(Vec<Repository>, Vec<Capability>, Vec<Evidence>)> {
    let mut repos = load_repositories(conn, snapshot_id)?;
    let deps = load_dependencies(conn, snapshot_id)?;
    for repo in &mut repos {
        if let Some(list) = deps.get(&repo.id) {
            repo.dependencies = list.clone();
        }
    }
    let capabilities = load_capabilities(conn, snapshot_id)?;
    let evidence = load_evidence(conn, snapshot_id)?;
    Ok((repos, capabilities, evidence))
}

fn load_capabilities(conn: &Connection, snapshot_id: &str) -> Result<Vec<Capability>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, repository, name, kind, description, evidence_sources
             FROM capabilities WHERE snapshot_id = ?1",
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    let rows = stmt
        .query_map(params![snapshot_id], |row| {
            let kind_str: String = row.get(3)?;
            Ok(Capability {
                id: row.get(0)?,
                repository: row.get(1)?,
                name: row.get(2)?,
                kind: if kind_str == "Cli" {
                    CapabilityKind::Cli
                } else {
                    CapabilityKind::Library
                },
                description: row.get(4)?,
                evidence_sources: row.get::<_, i64>(5)? as u32,
            })
        })
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| OrbweaverError::Storage(e.to_string()))
}

fn load_repositories(conn: &Connection, snapshot_id: &str) -> Result<Vec<Repository>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, path, description, primary_language, manifests_json, license,
                    readme_present, is_git_repo, default_branch, last_commit_at,
                    commit_count, contributor_count
             FROM repositories WHERE snapshot_id = ?1",
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    let rows = stmt
        .query_map(params![snapshot_id], |row| {
            let manifests_json: String = row.get(5)?;
            let last_commit_at: Option<String> = row.get(10)?;
            Ok(Repository {
                id: row.get(0)?,
                name: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                description: row.get(3)?,
                primary_language: row.get(4)?,
                manifests: serde_json::from_str::<Vec<ManifestKind>>(&manifests_json)
                    .unwrap_or_default(),
                license: row.get(6)?,
                readme_present: row.get::<_, i64>(7)? != 0,
                is_git_repo: row.get::<_, i64>(8)? != 0,
                default_branch: row.get(9)?,
                last_commit_at: last_commit_at
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|d| d.with_timezone(&Utc)),
                commit_count: row.get::<_, Option<i64>>(11)?.map(|v| v as u64),
                contributor_count: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                dependencies: Vec::new(),
            })
        })
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| OrbweaverError::Storage(e.to_string()))
}

fn load_dependencies(
    conn: &Connection,
    snapshot_id: &str,
) -> Result<std::collections::HashMap<String, Vec<DependencyRef>>> {
    let mut stmt = conn
        .prepare(
            "SELECT repo_id, name, version_req, manifest, is_path_dependency, path_hint, resolved_repo
             FROM dependencies WHERE snapshot_id = ?1",
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    let rows = stmt
        .query_map(params![snapshot_id], |row| {
            let repo_id: String = row.get(0)?;
            let manifest_str: String = row.get(3)?;
            let manifest: ManifestKind =
                serde_json::from_str(&manifest_str).unwrap_or(ManifestKind::Cargo);
            Ok((
                repo_id,
                DependencyRef {
                    name: row.get(1)?,
                    version_req: row.get(2)?,
                    manifest,
                    is_path_dependency: row.get::<_, i64>(4)? != 0,
                    path_hint: row.get(5)?,
                    resolved_repo: row.get(6)?,
                },
            ))
        })
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    let mut map: std::collections::HashMap<String, Vec<DependencyRef>> = std::collections::HashMap::new();
    for row in rows {
        let (repo_id, dep) = row.map_err(|e| OrbweaverError::Storage(e.to_string()))?;
        map.entry(repo_id).or_default().push(dep);
    }
    Ok(map)
}

fn load_evidence(conn: &Connection, snapshot_id: &str) -> Result<Vec<Evidence>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, source, source_type, repository, commit_hash, timestamp, extractor,
                    confidence_json, raw_reference, derived_claim
             FROM evidence WHERE snapshot_id = ?1",
        )
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    let rows = stmt
        .query_map(params![snapshot_id], |row| {
            let id: String = row.get(0)?;
            let source_type_str: String = row.get(2)?;
            let timestamp: String = row.get(5)?;
            let confidence_json: String = row.get(7)?;
            Ok(Evidence {
                id: uuid::Uuid::parse_str(&id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
                source: row.get(1)?,
                source_type: parse_source_type(&source_type_str),
                repository: row.get(3)?,
                commit: row.get(4)?,
                timestamp: DateTime::parse_from_rfc3339(&timestamp)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                extractor: row.get(6)?,
                confidence: serde_json::from_str::<Confidence>(&confidence_json)
                    .unwrap_or(Confidence::Observed),
                raw_reference: row.get(8)?,
                derived_claim: row.get(9)?,
            })
        })
        .map_err(|e| OrbweaverError::Storage(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| OrbweaverError::Storage(e.to_string()))
}

fn parse_source_type(s: &str) -> SourceType {
    match s {
        "GitHistory" => SourceType::GitHistory,
        "Readme" => SourceType::Readme,
        "Filesystem" => SourceType::Filesystem,
        _ => SourceType::Manifest,
    }
}
