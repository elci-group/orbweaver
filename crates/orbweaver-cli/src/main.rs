use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use orbweaver_core::CapabilityKind;
use orbweaver_graph::EcosystemGraph;
use orbweaver_ingest::ScanConfig;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "orbweaver", version, about = "Ecosystem intelligence and leverage engine for ELCI-group")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover repositories under a root, extract deterministic evidence,
    /// and persist an immutable snapshot.
    Scan {
        /// Directory to scan for repository candidates (one level deep).
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Cap on git commits walked per repository.
        #[arg(long, default_value_t = 20_000)]
        max_commits: usize,
        #[arg(long)]
        json: bool,
    },
    /// Show the most recent snapshot.
    Status,
    /// List all stored snapshots.
    Snapshots,
    /// Export the dependency graph of a snapshot.
    Graph {
        /// Snapshot id; defaults to the latest.
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List extracted capabilities from a snapshot.
    Capabilities {
        /// Snapshot id; defaults to the latest.
        #[arg(long)]
        snapshot: Option<String>,
        /// Only show capabilities for this repository id.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Find external dependencies shared by more than one repository —
    /// candidates for infrastructure-extraction review (not proof of
    /// duplication; see `orbweaver-analysis`).
    Duplicates {
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long, default_value_t = 2)]
        min_repos: usize,
        /// Exclude dependencies used by more than this fraction of
        /// repositories in their ecosystem (filters out foundational
        /// crates like tokio/serde that say nothing when shared).
        #[arg(long, default_value_t = 0.2)]
        max_ubiquity: f64,
        #[arg(long)]
        json: bool,
    },
    /// List extracted CLI interfaces (subcommands) from a snapshot.
    Interfaces {
        #[arg(long)]
        snapshot: Option<String>,
        /// Only show interfaces for this repository id.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List extracted schemas (serde-derived structs) from a snapshot.
    Schemas {
        #[arg(long)]
        snapshot: Option<String>,
        /// Only show schemas for this repository id.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Check that Orbweaver's local environment is healthy.
    Doctor,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Scan {
            root,
            max_commits,
            json,
        } => cmd_scan(root, max_commits, json),
        Command::Status => cmd_status(),
        Command::Snapshots => cmd_snapshots(),
        Command::Graph { snapshot, json } => cmd_graph(snapshot, json),
        Command::Capabilities { snapshot, repo, json } => cmd_capabilities(snapshot, repo, json),
        Command::Duplicates {
            snapshot,
            min_repos,
            max_ubiquity,
            json,
        } => cmd_duplicates(snapshot, min_repos, max_ubiquity, json),
        Command::Interfaces { snapshot, repo, json } => cmd_interfaces(snapshot, repo, json),
        Command::Schemas { snapshot, repo, json } => cmd_schemas(snapshot, repo, json),
        Command::Doctor => cmd_doctor(),
    }
}

fn cmd_scan(root: PathBuf, max_commits: usize, json: bool) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("root path does not exist: {}", root.display()))?;

    let config = ScanConfig {
        root: root.clone(),
        max_commits,
    };
    let result = orbweaver_ingest::scan(&config)
        .with_context(|| format!("failed to scan {}", root.display()))?;
    let data = orbweaver_storage::SnapshotData {
        repositories: result.repositories,
        capabilities: result.capabilities,
        interfaces: result.interfaces,
        schemas: result.schemas,
        evidence: result.evidence,
    };

    let snapshot_id = format!("snapshot-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    let db_path = orbweaver_storage::default_db_path();
    let mut conn =
        orbweaver_storage::open(&db_path).with_context(|| "failed to open local snapshot store")?;
    orbweaver_storage::save_snapshot(&mut conn, &snapshot_id, &root, &data)?;

    if json {
        let export = serde_json::json!({
            "snapshot": snapshot_id,
            "root": root,
            "repositories": data.repositories,
            "capabilities": data.capabilities,
            "interfaces": data.interfaces,
            "schemas": data.schemas,
        });
        println!("{}", serde_json::to_string_pretty(&export)?);
    } else {
        print_scan_summary(&snapshot_id, &root, &data);
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let Some(snapshot_id) = orbweaver_storage::latest_snapshot_id(&conn)? else {
        println!("No snapshots yet. Run `orbweaver scan --root <path>` first.");
        return Ok(());
    };
    let data = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?;
    let summary = orbweaver_storage::list_snapshots(&conn)?
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .expect("just-loaded snapshot must be listed");
    print_scan_summary(&snapshot_id, &PathBuf::from(&summary.root), &data);
    Ok(())
}

fn cmd_snapshots() -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshots = orbweaver_storage::list_snapshots(&conn)?;
    if snapshots.is_empty() {
        println!("No snapshots yet. Run `orbweaver scan --root <path>` first.");
        return Ok(());
    }
    for s in snapshots {
        println!("{}  {}  root={}", s.id, s.created_at.to_rfc3339(), s.root);
    }
    Ok(())
}

fn cmd_graph(snapshot: Option<String>, json: bool) -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshot_id = match snapshot {
        Some(id) => id,
        None => orbweaver_storage::latest_snapshot_id(&conn)?
            .context("no snapshots yet — run `orbweaver scan` first")?,
    };
    let data = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?;
    let graph = EcosystemGraph::from_repositories(&data.repositories);

    if json {
        println!("{}", serde_json::to_string_pretty(&graph.to_export())?);
    } else {
        println!("Snapshot: {snapshot_id}");
        println!("Nodes (repositories): {}", graph.node_count());
        println!("Edges (resolved depends_on): {}", graph.edge_count());
        println!();
        println!("Most depended-on repositories:");
        for (i, (name, count)) in graph.most_depended_on(10).into_iter().enumerate() {
            println!("  {:>2}. {name}  ({count} consumer(s))", i + 1);
        }
    }
    Ok(())
}

fn cmd_capabilities(snapshot: Option<String>, repo: Option<String>, json: bool) -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshot_id = match snapshot {
        Some(id) => id,
        None => orbweaver_storage::latest_snapshot_id(&conn)?
            .context("no snapshots yet — run `orbweaver scan` first")?,
    };
    let mut capabilities = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?.capabilities;

    if let Some(repo_id) = &repo {
        capabilities.retain(|c| &c.repository == repo_id);
    }
    capabilities.sort_by(|a, b| a.repository.cmp(&b.repository).then(a.name.cmp(&b.name)));

    if json {
        println!("{}", serde_json::to_string_pretty(&capabilities)?);
        return Ok(());
    }

    println!("Snapshot: {snapshot_id}");
    println!("Capabilities: {}\n", capabilities.len());
    for cap in &capabilities {
        let kind = match cap.kind {
            CapabilityKind::Cli => "cli",
            CapabilityKind::Library => "lib",
        };
        let desc = cap.description.as_deref().unwrap_or("(no description)");
        println!(
            "  [{kind}] {:<28} {:<20} sources={}  {desc}",
            cap.repository, cap.name, cap.evidence_sources
        );
    }
    Ok(())
}

fn cmd_duplicates(snapshot: Option<String>, min_repos: usize, max_ubiquity: f64, json: bool) -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshot_id = match snapshot {
        Some(id) => id,
        None => orbweaver_storage::latest_snapshot_id(&conn)?
            .context("no snapshots yet — run `orbweaver scan` first")?,
    };
    let data = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?;
    let candidates = orbweaver_analysis::shared_dependency_candidates(
        &data.repositories,
        min_repos,
        max_ubiquity,
    );

    if json {
        println!("{}", serde_json::to_string_pretty(&candidates)?);
        return Ok(());
    }

    println!("Snapshot: {snapshot_id}");
    println!(
        "Shared external dependency candidates: {} (min_repos={min_repos})",
        candidates.len()
    );
    println!(
        "Confidence: ProbabilisticInference — shared adoption of a dependency is a candidate\n\
         signal for shared/duplicate infrastructure, not proof of it. Review before acting.\n"
    );
    for c in &candidates {
        println!(
            "  {:<28} [{:?}]  {}/{} repos: {}",
            c.dependency_name,
            c.manifest,
            c.repositories.len(),
            c.ecosystem_total,
            c.repositories.join(", ")
        );
    }
    Ok(())
}

fn cmd_interfaces(snapshot: Option<String>, repo: Option<String>, json: bool) -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshot_id = match snapshot {
        Some(id) => id,
        None => orbweaver_storage::latest_snapshot_id(&conn)?
            .context("no snapshots yet — run `orbweaver scan` first")?,
    };
    let mut interfaces = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?.interfaces;

    if let Some(repo_id) = &repo {
        interfaces.retain(|i| &i.repository == repo_id);
    }
    interfaces.sort_by(|a, b| a.repository.cmp(&b.repository).then(a.name.cmp(&b.name)));

    if json {
        println!("{}", serde_json::to_string_pretty(&interfaces)?);
        return Ok(());
    }

    println!("Snapshot: {snapshot_id}");
    println!("Interfaces: {}", interfaces.len());
    println!(
        "Confidence: ProbabilisticInference — statically detected from #[derive(Subcommand)]\n\
         enums, not verified against runtime --help. Misses non-derive-style CLIs.\n"
    );
    for iface in &interfaces {
        let desc = iface.description.as_deref().unwrap_or("(no description)");
        println!("  {:<28} {:<20} {desc}", iface.repository, iface.name);
    }
    Ok(())
}

fn cmd_schemas(snapshot: Option<String>, repo: Option<String>, json: bool) -> Result<()> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path)?;
    let snapshot_id = match snapshot {
        Some(id) => id,
        None => orbweaver_storage::latest_snapshot_id(&conn)?
            .context("no snapshots yet — run `orbweaver scan` first")?,
    };
    let mut schemas = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?.schemas;

    if let Some(repo_id) = &repo {
        schemas.retain(|s| &s.repository == repo_id);
    }
    schemas.sort_by(|a, b| a.repository.cmp(&b.repository).then(a.name.cmp(&b.name)));

    if json {
        println!("{}", serde_json::to_string_pretty(&schemas)?);
        return Ok(());
    }

    println!("Snapshot: {snapshot_id}");
    println!("Schemas: {}", schemas.len());
    println!(
        "Confidence: ProbabilisticInference — statically detected from #[derive(Serialize/\n\
         Deserialize)] structs, not verified against actual serialized output.\n"
    );
    for schema in &schemas {
        let desc = schema.description.as_deref().unwrap_or("(no description)");
        println!("  {:<20} {:<24} {desc}", schema.repository, schema.name);
        for field in &schema.fields {
            println!("      {:<20} {}", field.name, field.type_repr);
        }
    }
    Ok(())
}

fn cmd_doctor() -> Result<()> {
    println!("ORBWEAVER DOCTOR\n");

    let db_path = orbweaver_storage::default_db_path();
    match orbweaver_storage::open(&db_path) {
        Ok(_) => println!("[ok]      local snapshot store writable at {}", db_path.display()),
        Err(e) => println!("[fail]    local snapshot store: {e}"),
    }

    match which_git() {
        Some(path) => println!("[ok]      git available ({path})"),
        None => println!("[warn]    `git` not found on PATH — git history evidence will be unavailable"),
    }

    println!(
        "[info]    ELCI connectors (Ontism, Padagonia, Kaptaind, Skillastic, ...): not yet implemented (Phase III)"
    );
    println!("[info]    capability extraction, opportunity engine, leverage scoring: not yet implemented (Phase II+)");

    Ok(())
}

fn which_git() -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

fn print_scan_summary(snapshot_id: &str, root: &Path, data: &orbweaver_storage::SnapshotData) {
    let repositories = &data.repositories;
    let graph = EcosystemGraph::from_repositories(repositories);

    let git_tracked = repositories.iter().filter(|r| r.is_git_repo).count();
    let total_commits: u64 = repositories.iter().filter_map(|r| r.commit_count).sum();

    let mut by_language: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for repo in repositories {
        let lang = repo.primary_language.as_deref().unwrap_or("unknown");
        *by_language.entry(lang).or_insert(0) += 1;
    }

    println!("ORBWEAVER SCAN\n");
    println!("Snapshot:    {snapshot_id}");
    println!("Root:        {}", root.display());
    println!("Repositories discovered: {}", repositories.len());
    for (lang, count) in &by_language {
        println!("  {lang:<24} {count}");
    }
    println!("Git-tracked: {git_tracked} / {}", repositories.len());
    println!("Total commits observed (capped per repo): {total_commits}");
    println!("Evidence records: {}", data.evidence.len());
    println!("Dependency edges resolved: {}", graph.edge_count());

    let cli_count = data
        .capabilities
        .iter()
        .filter(|c| c.kind == CapabilityKind::Cli)
        .count();
    let lib_count = data.capabilities.len() - cli_count;
    println!(
        "Capabilities extracted: {} (cli={cli_count}, library={lib_count})",
        data.capabilities.len()
    );
    println!("CLI interfaces extracted (heuristic): {}", data.interfaces.len());
    println!("Schemas extracted (heuristic): {}", data.schemas.len());

    let top = graph.most_depended_on(5);
    if !top.is_empty() {
        println!("\nMost depended-on repositories:");
        for (i, (name, count)) in top.into_iter().enumerate() {
            println!("  {:>2}. {name}  ({count} consumer(s))", i + 1);
        }
    }
}
