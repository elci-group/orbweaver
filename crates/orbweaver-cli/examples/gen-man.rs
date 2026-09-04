// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT
//! Regenerates the man pages under `man/` from the real `clap` definitions
//! in `src/cli.rs`, so the shipped man pages can never drift from the actual
//! CLI surface. Run after changing the `Cli`/`Command` structs:
//!
//!   cargo run -p orbweaver-cli --example gen-man
//!
//! One page is written per binary and per subcommand path
//! (e.g. `man/orbweaver-scan.1`).
use clap::CommandFactory;
use orbweaver_cli::cli::Cli;
use std::path::Path;

fn write_man(cmd: clap::Command, title: &str, dest: &Path) -> std::io::Result<()> {
    let man = clap_mangen::Man::new(cmd).title(title);
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    std::fs::write(dest, buf)
}

fn gen(cmd: &clap::Command, prefix: &str, dir: &Path) -> std::io::Result<()> {
    write_man(cmd.clone(), prefix, &dir.join(format!("{prefix}.1")))?;
    for sub in cmd.get_subcommands() {
        let name = format!("{prefix}-{}", sub.get_name());
        gen(sub, &name, dir)?;
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let man_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("man");
    std::fs::create_dir_all(&man_dir)?;
    gen(&Cli::command(), "orbweaver", &man_dir)?;
    println!("Wrote man pages to {}", man_dir.display());
    Ok(())
}
