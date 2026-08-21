use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use orbweaver_core::Repository;
use orbweaver_evidence::Evidence;
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

    let snapshot_id = format!("snapshot-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"));

    let db_path = orbweaver_storage::default_db_path();
    let mut conn =
        orbweaver_storage::open(&db_path).with_context(|| "failed to open local snapshot store")?;
    orbweaver_storage::save_snapshot(
        &mut conn,
        &snapshot_id,
        &root,
        &result.repositories,
        &result.evidence,
    )?;

    if json {
        let export = serde_json::json!({
            "snapshot": snapshot_id,
            "root": root,
            "repositories": result.repositories,
        });
        println!("{}", serde_json::to_string_pretty(&export)?);
    } else {
        print_scan_summary(&snapshot_id, &root, &result.repositories, &result.evidence);
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
    let (repositories, evidence) = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?;
    let summary = orbweaver_storage::list_snapshots(&conn)?
        .into_iter()
        .find(|s| s.id == snapshot_id)
        .expect("just-loaded snapshot must be listed");
    print_scan_summary(&snapshot_id, &PathBuf::from(&summary.root), &repositories, &evidence);
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
    let (repositories, _evidence) = orbweaver_storage::load_snapshot(&conn, &snapshot_id)?;
    let graph = EcosystemGraph::from_repositories(&repositories);

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

fn print_scan_summary(snapshot_id: &str, root: &Path, repositories: &[Repository], evidence: &[Evidence]) {
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
    println!("Evidence records: {}", evidence.len());
    println!("Dependency edges resolved: {}", graph.edge_count());

    let top = graph.most_depended_on(5);
    if !top.is_empty() {
        println!("\nMost depended-on repositories:");
        for (i, (name, count)) in top.into_iter().enumerate() {
            println!("  {:>2}. {name}  ({count} consumer(s))", i + 1);
        }
    }
}
