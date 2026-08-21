mod style;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use orbweaver_core::CapabilityKind;
use orbweaver_graph::EcosystemGraph;
use orbweaver_ingest::ScanConfig;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "orbweaver",
    version,
    about = "🕸️  Ecosystem intelligence and leverage engine for ELCI-group"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 🔍 Discover repositories under a root, extract evidence, and save a snapshot.
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
    /// 📊 Show the most recent snapshot at a glance.
    Status,
    /// 📸 List every snapshot taken so far.
    Snapshots,
    /// 🕸️  Export the dependency graph of a snapshot.
    Graph {
        /// Snapshot id; defaults to the latest.
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 🧩 List what each repository in a snapshot can actually do.
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
    /// 🔁 Find external dependencies shared by more than one repository —
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
    /// 🔌 List the CLI subcommands detected in a snapshot.
    Interfaces {
        #[arg(long)]
        snapshot: Option<String>,
        /// Only show interfaces for this repository id.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 📐 List the data schemas detected in a snapshot.
    Schemas {
        #[arg(long)]
        snapshot: Option<String>,
        /// Only show schemas for this repository id.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 🔗 Discover which ELCI tools are actually available and what they
    /// actually expose — probes `--help`/`--version` on PATH, never
    /// assumes an interface (directive sections 26–27).
    Integrations {
        #[arg(long)]
        json: bool,
    },
    /// 🩺 Check that Orbweaver's local environment is healthy.
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
        Command::Integrations { json } => cmd_integrations(json),
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

    let pb = style::spinner(format!("Weaving through {}…", style(root.display()).cyan()));
    let result = orbweaver_ingest::scan(&config)
        .with_context(|| format!("failed to scan {}", root.display()))?;
    pb.finish_and_clear();

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
        println!(
            "{}No snapshots yet — run {} to take your first one.",
            style::STATUS,
            style("orbweaver scan --root <path>").bold()
        );
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
        println!(
            "{}No snapshots yet — run {} to take your first one.",
            style::SNAPSHOTS,
            style("orbweaver scan --root <path>").bold()
        );
        return Ok(());
    }
    println!("{}\n", style::header(style::SNAPSHOTS, "ORBWEAVER SNAPSHOTS"));
    for s in snapshots {
        println!(
            "  {}  {}  {}",
            style(&s.id).bold().cyan(),
            style(s.created_at.to_rfc3339()).dim(),
            s.root
        );
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
        return Ok(());
    }

    println!("{}\n", style::header(style::GRAPH, "ORBWEAVER GRAPH"));
    println!("Snapshot:               {}", style(&snapshot_id).bold());
    println!("Nodes (repositories):   {}", style(graph.node_count()).bold());
    println!("Edges (resolved depends_on): {}", style(graph.edge_count()).bold());

    let top = graph.most_depended_on(10);
    if !top.is_empty() {
        println!("\n{}", style("Most depended-on repositories:").bold());
        for (i, (name, count)) in top.into_iter().enumerate() {
            println!("  {} {}  ({} consumer(s))", rank_medal(i), style(name).cyan(), count);
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

    println!("{}\n", style::header(style::CAPABILITIES, "ORBWEAVER CAPABILITIES"));
    println!(
        "Snapshot: {}   Capabilities: {}\n",
        style(&snapshot_id).bold(),
        style(capabilities.len()).bold()
    );
    for cap in &capabilities {
        let kind = match cap.kind {
            CapabilityKind::Cli => style(" cli ").black().on_cyan(),
            CapabilityKind::Library => style(" lib ").black().on_yellow(),
        };
        let desc = cap.description.as_deref().unwrap_or("(no description)");
        println!(
            "  {kind} {:<28} {:<20} sources={}  {desc}",
            style(&cap.repository).cyan(),
            cap.name,
            cap.evidence_sources
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

    println!("{}\n", style::header(style::DUPLICATES, "ORBWEAVER DUPLICATES"));
    println!(
        "Shared external dependency candidates: {} (min_repos={min_repos})\n",
        style(candidates.len()).bold()
    );
    println!(
        "{}",
        style::note(
            "Confidence: ProbabilisticInference — shared adoption of a dependency is a candidate\n\
             signal for shared/duplicate infrastructure, not proof of it. Review before acting.\n"
        )
    );
    for c in &candidates {
        println!(
            "  {:<28} [{:?}]  {}/{} repos: {}",
            style(&c.dependency_name).cyan(),
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

    println!("{}\n", style::header(style::INTERFACES, "ORBWEAVER INTERFACES"));
    println!(
        "Snapshot: {}   Interfaces: {}\n",
        style(&snapshot_id).bold(),
        style(interfaces.len()).bold()
    );
    println!(
        "{}",
        style::note(
            "Confidence: ProbabilisticInference — statically detected from #[derive(Subcommand)]\n\
             enums, not verified against runtime --help. Misses non-derive-style CLIs.\n"
        )
    );
    for iface in &interfaces {
        let desc = iface.description.as_deref().unwrap_or("(no description)");
        println!("  {:<28} {:<20} {desc}", style(&iface.repository).cyan(), iface.name);
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

    println!("{}\n", style::header(style::SCHEMAS, "ORBWEAVER SCHEMAS"));
    println!(
        "Snapshot: {}   Schemas: {}\n",
        style(&snapshot_id).bold(),
        style(schemas.len()).bold()
    );
    println!(
        "{}",
        style::note(
            "Confidence: ProbabilisticInference — statically detected from #[derive(Serialize/\n\
             Deserialize)] structs, not verified against actual serialized output.\n"
        )
    );
    for schema in &schemas {
        let desc = schema.description.as_deref().unwrap_or("(no description)");
        println!(
            "  {:<20} {:<24} {desc}",
            style(&schema.repository).cyan(),
            style(&schema.name).bold()
        );
        for field in &schema.fields {
            println!("      {:<20} {}", style(&field.name).dim(), field.type_repr);
        }
    }
    Ok(())
}

fn cmd_integrations(json: bool) -> Result<()> {
    let repo_paths = latest_repo_paths().unwrap_or_default();

    let pb = style::spinner("Reaching out across the ELCI estate…");
    let mut reports = Vec::new();
    for tool in orbweaver_connectors::KNOWN_TOOLS {
        pb.set_message(format!("Probing {}…", style(*tool).cyan()));
        let repo_path = repo_paths.get(*tool).map(|p| p.as_path());
        reports.extend(orbweaver_connectors::probe_tool(tool, repo_path));
    }
    pb.finish_and_clear();

    if json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return Ok(());
    }

    println!("{}\n", style::header(style::INTEGRATIONS, "ORBWEAVER INTEGRATIONS"));
    println!(
        "{}\n",
        style::note(
            "Probing PATH for known ELCI binaries. Only --help/--version, and a\n\
             self-description command if --help itself listed one, are ever run."
        )
    );

    let available = reports
        .iter()
        .filter(|r| matches!(r.availability, orbweaver_evidence::Availability::Known(_)))
        .count();
    println!(
        "{} {}/{} candidate binaries found on PATH\n",
        style::OK,
        style(available).bold(),
        reports.len()
    );

    for report in &reports {
        match &report.availability {
            orbweaver_evidence::Availability::Known(details) => {
                let version = details.version.as_deref().unwrap_or("unknown version");
                let method = match details.discovery_method {
                    orbweaver_connectors::DiscoveryMethod::JsonCapabilityManifest => "json-manifest",
                    orbweaver_connectors::DiscoveryMethod::HelpTextHeuristic => "help-text",
                };
                println!(
                    "{} {:<16} {:<14} {} command(s) discovered ({method})",
                    style::OK,
                    style(&report.binary).bold().cyan(),
                    style(version).dim(),
                    details.commands.len(),
                );
            }
            orbweaver_evidence::Availability::Unavailable { reason } => {
                println!("{} {:<16} {}", style::MISSING, report.binary, style(reason).dim());
            }
            orbweaver_evidence::Availability::Unknown => {
                println!("{} {:<16} (unable to determine)", style::WARN, report.binary);
            }
        }
    }

    Ok(())
}

fn latest_repo_paths() -> Option<std::collections::HashMap<String, PathBuf>> {
    let db_path = orbweaver_storage::default_db_path();
    let conn = orbweaver_storage::open(&db_path).ok()?;
    let snapshot_id = orbweaver_storage::latest_snapshot_id(&conn).ok()??;
    let data = orbweaver_storage::load_snapshot(&conn, &snapshot_id).ok()?;
    Some(data.repositories.into_iter().map(|r| (r.id, r.path)).collect())
}

fn cmd_doctor() -> Result<()> {
    println!("{}\n", style::header(style::DOCTOR, "ORBWEAVER DOCTOR"));

    let db_path = orbweaver_storage::default_db_path();
    match orbweaver_storage::open(&db_path) {
        Ok(_) => println!(
            "{} local snapshot store writable at {}",
            style::OK,
            db_path.display()
        ),
        Err(e) => println!("{} local snapshot store: {e}", style::FAIL),
    }

    match which_git() {
        Some(path) => println!("{} git available ({path})", style::OK),
        None => println!(
            "{} `git` not found on PATH — git history evidence will be unavailable",
            style::WARN
        ),
    }

    let repo_paths = latest_repo_paths().unwrap_or_default();
    let reports = orbweaver_connectors::probe_all(&repo_paths);
    let available = reports
        .iter()
        .filter(|r| matches!(r.availability, orbweaver_evidence::Availability::Known(_)))
        .count();
    println!(
        "{} ELCI connectors: {available}/{} candidate binaries discovered on PATH (see `orbweaver integrations`)",
        style::OK,
        reports.len()
    );

    println!(
        "{} opportunity engine, leverage scoring: not yet implemented (Phase IV+)",
        style::WARN
    );

    Ok(())
}

fn which_git() -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

fn rank_medal(index: usize) -> String {
    match index {
        0 => "🥇".to_string(),
        1 => "🥈".to_string(),
        2 => "🥉".to_string(),
        _ => format!("{:>2}.", index + 1),
    }
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

    println!("{}\n", style::header(style::SCAN, "ORBWEAVER SCAN"));
    println!("Snapshot:    {}", style(snapshot_id).bold());
    println!("Root:        {}", style(root.display()).cyan());
    println!("Repositories discovered: {}", style(repositories.len()).bold());
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
        "{}Capabilities extracted: {} (cli={cli_count}, library={lib_count})",
        style::CAPABILITIES,
        data.capabilities.len()
    );
    println!(
        "{}CLI interfaces extracted (heuristic): {}",
        style::INTERFACES,
        data.interfaces.len()
    );
    println!(
        "{}Schemas extracted (heuristic): {}",
        style::SCHEMAS,
        data.schemas.len()
    );

    let top = graph.most_depended_on(5);
    if !top.is_empty() {
        println!("\n{}", style("Most depended-on repositories:").bold());
        for (i, (name, count)) in top.into_iter().enumerate() {
            println!("  {} {}  ({count} consumer(s))", rank_medal(i), style(name).cyan());
        }
    }
}
