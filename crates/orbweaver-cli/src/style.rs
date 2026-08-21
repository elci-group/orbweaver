//! Terminal presentation: color, emoji, and animated progress for the
//! human-readable output paths only. Every `--json` path stays plain,
//! parseable JSON with nothing from this module touching it — piping
//! `orbweaver scan --json` into `jq` must keep working.
//!
//! Colors and emoji degrade automatically: `console` detects non-tty
//! output (a pipe, a redirected file) and strips ANSI codes, and honours
//! `NO_COLOR`/`CLICOLOR_FORCE`. Every [`Emoji`] carries a plain-text
//! fallback for terminals that can't render it.

use console::{style, Emoji};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub static SCAN: Emoji<'_, '_> = Emoji("🔍 ", "");
pub static STATUS: Emoji<'_, '_> = Emoji("📊 ", "");
pub static SNAPSHOTS: Emoji<'_, '_> = Emoji("📸 ", "");
pub static GRAPH: Emoji<'_, '_> = Emoji("🕸️  ", "");
pub static CAPABILITIES: Emoji<'_, '_> = Emoji("🧩 ", "");
pub static DUPLICATES: Emoji<'_, '_> = Emoji("🔁 ", "");
pub static INTERFACES: Emoji<'_, '_> = Emoji("🔌 ", "");
pub static SCHEMAS: Emoji<'_, '_> = Emoji("📐 ", "");
pub static INTEGRATIONS: Emoji<'_, '_> = Emoji("🔗 ", "");
pub static DOCTOR: Emoji<'_, '_> = Emoji("🩺 ", "");

pub static OK: Emoji<'_, '_> = Emoji("✅", "[ok]");
pub static MISSING: Emoji<'_, '_> = Emoji("⚪", "[--]");
pub static WARN: Emoji<'_, '_> = Emoji("⚠️ ", "[!]");
pub static FAIL: Emoji<'_, '_> = Emoji("🔴", "[x]");

/// A bold, coloured section header with a leading emoji, e.g.
/// `🔍 ORBWEAVER SCAN`.
pub fn header(emoji: Emoji, title: &str) -> String {
    format!("{emoji}{}", style(title).bold().cyan())
}

/// A muted caveat/disclaimer line — used for the ProbabilisticInference
/// notices so they read as "worth knowing" rather than alarming.
pub fn note(text: &str) -> String {
    style(text).dim().to_string()
}

/// A short-lived spinner for an operation with no natural progress count
/// (a single blocking call, e.g. the whole ingestion pass). Draws to
/// stderr, so it never interferes with `--json` output on stdout.
pub fn spinner(message: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"]),
    );
    pb.set_message(message.into());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}
