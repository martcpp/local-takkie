//! Dev commands for this repo, run with `cargo xtask <command>`.
//!
//! Plain Rust instead of shell scripts, so it works the same on Windows, macOS
//! and Linux.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, bail};

const USAGE: &str = "usage: cargo xtask <command>

commands:
  ci    run the checks CI runs: fmt, clippy, tests and cargo-deny
  fmt   format the whole workspace";

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("ci") => ci(),
        Some("fmt") => cargo(&["fmt", "--all"]),
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
    } else if !cargo_deny_installed() {
        eprintln!(
            "\nxtask: skipping cargo deny check (install it: cargo install --locked cargo-deny)"
        );
    } else {
        cargo(&["deny", "check"])?;
    }
    eprintln!("\nxtask: all checks passed");
    Ok(())
}

fn cargo(args: &[&str]) -> Result<()> {
    eprintln!("\n> cargo {}", args.join(" "));
    let status = Command::new(cargo_bin())
        .args(args)
        .current_dir(workspace_root())
        .status()
        .context("couldn't start cargo")?;
    if !status.success() {
        bail!("`cargo {}` failed ({status})", args.join(" "));
    }
    Ok(())
}

fn cargo_deny_installed() -> bool {
    Command::new(cargo_bin())
        .args(["deny", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn workspace_root() -> PathBuf {
    let xtask_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask_dir.parent().unwrap_or(xtask_dir).to_path_buf()
}

// Use the cargo that launched us, so every step runs on the same toolchain.
fn cargo_bin() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}
