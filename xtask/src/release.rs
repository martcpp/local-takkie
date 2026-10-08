//! `cargo xtask release <version>`: get a release ready in one command.

use std::fs;

use anyhow::{Context, Result, bail};
use semver::Version;
use toml_edit::{DocumentMut, value};

use crate::{cargo, git, git_output, succeeds, workspace_root};

/// Sets the workspace version, updates `Cargo.lock` and regenerates
/// `CHANGELOG.md` for the new version.
pub(crate) fn release(arg: Option<&str>) -> Result<()> {
    let Some(arg) = arg else {
        bail!("missing version, for example: cargo xtask release 0.2.0");
    };
    let new = parse_version(arg)?;

    let branch = git_output(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    if matches!(branch.as_str(), "develop" | "main") {
        bail!(
            "{branch} only changes through PRs; create a branch first: git switch -c chore/release-{new}"
        );
    }
    if !succeeds("git", &["cliff", "--version"]) {
        bail!("git-cliff isn't installed: cargo install --locked git-cliff");
    }

    let path = workspace_root().join("Cargo.toml");
    let manifest = fs::read_to_string(&path).context("couldn't read Cargo.toml")?;
    let (updated, current) = set_workspace_version(&manifest, &new)?;
    if new <= current {
        bail!("{new} isn't newer than the current version {current}");
    }
    fs::write(&path, updated).context("couldn't write Cargo.toml")?;
    eprintln!("\nxtask: version {current} -> {new}");

    cargo(&["update", "--workspace"])?;
    git(&["cliff", "--tag", &format!("v{new}"), "-o", "CHANGELOG.md"])?;

    eprintln!(
        "\nxtask: v{new} is ready. Next:
  1. Commit, push and open a PR into develop, titled \"chore(release): v{new}\".
  2. Once it's merged, open a release PR from develop into main and merge it
     with a merge commit.
  3. Tag that commit on main as v{new} and push the tag; the release workflow
     does the rest."
    );
    Ok(())
}

fn parse_version(arg: &str) -> Result<Version> {
    Version::parse(arg.strip_prefix('v').unwrap_or(arg))
        .with_context(|| format!("`{arg}` isn't a version like 0.2.0"))
}

/// Returns the manifest with `[workspace.package] version` set to `new`, and
/// the version it had before. Everything else in the file is left as it was.
fn set_workspace_version(manifest: &str, new: &Version) -> Result<(String, Version)> {
    let mut doc: DocumentMut = manifest.parse().context("couldn't parse Cargo.toml")?;
    let item = doc
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("package"))
        .and_then(|package| package.get_mut("version"))
        .context("Cargo.toml has no [workspace.package] version")?;
    let current = item
        .as_str()
        .context("[workspace.package] version isn't a string")
        .and_then(|text| {
            Version::parse(text).with_context(|| format!("current version `{text}` isn't valid"))
        })?;
    *item = value(new.to_string());
    Ok((doc.to_string(), current))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"

# A comment that has to survive.
[workspace.lints.rust]
unsafe_code = "forbid"
"#;

    #[test]
    fn parses_versions_with_or_without_v() {
        assert_eq!(parse_version("0.2.0").unwrap(), Version::new(0, 2, 0));
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(parse_version("two").is_err());
        assert!(parse_version("1.2").is_err());
    }

    #[test]
    fn sets_only_the_workspace_version() {
        let (updated, current) = set_workspace_version(MANIFEST, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(current, Version::new(0, 1, 0));
        assert_eq!(
            updated,
            MANIFEST.replace(r#"version = "0.1.0""#, r#"version = "0.2.0""#)
        );
    }

    #[test]
    fn errors_without_a_workspace_version() {
        let manifest = "[workspace]\nmembers = []\n";
        assert!(set_workspace_version(manifest, &Version::new(0, 2, 0)).is_err());
    }
}
