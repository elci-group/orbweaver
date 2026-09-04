//! Command-line surface of the `orbweaver` binary.
//!
//! Kept in the library target so `examples/gen-man.rs` can regenerate the
//! man pages from these exact definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "orbweaver",
    version,
    about = "🕸️  Ecosystem intelligence and leverage engine for ELCI-group"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
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
        /// GitHub org to discover repositories from.
        #[arg(long)]
        org: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 🩺 Check that Orbweaver's local environment is healthy.
    Doctor,
}
