//! Shared subprocess-with-timeout primitives, used by both tool probing
//! (`--help`/`--version`) and the install path (`git clone`, `gh release
//! download`, `baby`). Nothing in this crate spawns a process any other
//! way — a hung external tool must never hang Orbweaver.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ProcessOutput {
    /// Whether the process exited with status 0. A tool that doesn't
    /// recognise a flag (e.g. `--version` on a tool that never wired it
    /// up) exits nonzero and prints clap's own "unexpected argument"
    /// error — that error text must never be mistaken for real output,
    /// which is exactly why callers check this before trusting `text`.
    pub success: bool,
    pub text: String,
}

pub fn which(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(binary_name))
        .find(|candidate| candidate.is_file())
}

/// Run `binary args...` with a hard timeout, killing it if it doesn't
/// exit in time. Returns stdout, falling back to stderr if stdout was
/// empty (some tools print help/errors to stderr).
pub fn run_with_timeout(
    binary: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Option<ProcessOutput> {
    let mut command = Command::new(binary);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    let mut child = command.spawn().ok()?;
    let start = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let text = if !stdout.trim().is_empty() {
        stdout
    } else {
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    Some(ProcessOutput { success, text })
}

/// The last `n` characters of `text`, on a UTF-8 boundary — for putting
/// a usable amount of a captured build/clone log into an error message
/// without dragging the whole thing along.
pub fn tail(text: &str, n: usize) -> &str {
    if text.len() <= n {
        return text;
    }
    let mut start = text.len() - n;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
