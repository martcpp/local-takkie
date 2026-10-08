//! Dev commands for this repo, run with `cargo xtask <command>`.
//!
//! Plain Rust instead of shell scripts, so it works the same on Windows, macOS
//! and Linux.

mod release;

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};

const USAGE: &str = "usage: cargo xtask <command>

commands:
  ci                  run the checks CI runs: fmt, clippy, tests and cargo-deny
  fmt                 format the whole workspace
  release <version>   set the version and regenerate CHANGELOG.md, e.g. release 0.2.0";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ci") => ci(),
        Some("fmt") => cargo(&["fmt", "--all"]),
        Some("release") => release::release(args.get(1).map(String::as_str)),
        Some("help" | "-h" | "--help") => {
            eprintln!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("\nxtask: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn ci() -> Result<()> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    if !workspace_root().join("deny.toml").exists() {
        eprintln!("\nxtask: skipping cargo deny check (no deny.toml yet)");
    } else if !succeeds(cargo_bin(), &["deny", "--version"]) {
        eprintln!(
            "\nxtask: skipping cargo deny check (install it: cargo install --locked cargo-deny)"
        );
    } else {
        cargo(&["deny", "check"])?;
    }
    eprintln!("\nxtask: all checks passed");
    Ok(())
}

pub(crate) fn cargo(args: &[&str]) -> Result<()> {
    run(cargo_bin(), "cargo", args)
}

pub(crate) fn git(args: &[&str]) -> Result<()> {
    run("git", "git", args)
}

fn run(program: impl AsRef<OsStr>, name: &str, args: &[&str]) -> Result<()> {
    eprintln!("\n> {name} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .status()
        .with_context(|| format!("couldn't start {name}"))?;
    if !status.success() {
        bail!("`{name} {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

/// Runs git quietly and returns its trimmed stdout.
pub(crate) fn git_output(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root())
        .stderr(Stdio::null())
        .output()
        .context("couldn't start git")?;
    if !output.status.success() {
        bail!("`git {}` failed ({})", args.join(" "), output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Whether the command runs and exits successfully, with its output hidden.
pub(crate) fn succeeds(program: impl AsRef<OsStr>, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn workspace_root() -> PathBuf {
    let xtask_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask_dir.parent().unwrap_or(xtask_dir).to_path_buf()
}

// Use the cargo that launched us, so every step runs on the same toolchain.
fn cargo_bin() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
