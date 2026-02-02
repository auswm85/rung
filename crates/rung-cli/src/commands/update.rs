//! `rung update` command - Update rung to the latest version.
//!
//! This module is excluded from test coverage as it performs self-update
//! operations that cannot be meaningfully tested in isolation.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use crate::output;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const CRATE_NAME: &str = "rung-cli";

/// Run the update command.
pub fn run(check_only: bool) -> Result<()> {
    output::info(&format!("Current version: {CURRENT_VERSION}"));

    // Check latest version from crates.io
    let latest_version = fetch_latest_version()?;

    if latest_version == CURRENT_VERSION {
        output::success("Already up to date!");
        return Ok(());
    }

    output::info(&format!("Latest version: {latest_version}"));

    if check_only {
        output::warn(&format!(
            "Update available: {CURRENT_VERSION} → {latest_version}"
        ));
        output::info("Run `rung update` to install");
        return Ok(());
    }

    // Warn if binary is not in ~/.cargo/bin (cargo install won't replace it)
    check_install_location();

    // Try cargo-binstall first (fast, pre-built binary), fall back to cargo install
    if has_cargo_binstall() {
        output::info("Updating via cargo-binstall...");
        run_cargo_binstall()?;
    } else {
        output::info("Updating via cargo install (this may take a minute)...");
        run_cargo_install()?;
    }

    output::success(&format!("Updated: {CURRENT_VERSION} → {latest_version}"));
    Ok(())
}

/// Check if the current binary is in ~/.cargo/bin and warn if not.
fn check_install_location() {
    let Some(current_exe) = std::env::current_exe().ok() else {
        return;
    };

    let Some(cargo_bin) = cargo_bin_dir() else {
        return;
    };

    // Resolve symlinks to compare actual locations
    let current_exe = current_exe.canonicalize().unwrap_or(current_exe);
    let cargo_bin = cargo_bin.canonicalize().unwrap_or(cargo_bin);

    if !current_exe.starts_with(&cargo_bin) {
        output::warn(&format!("Current binary is at: {}", current_exe.display()));
        output::warn(&format!(
            "Update will install to: {}/rung",
            cargo_bin.display()
        ));
        output::warn("You may need to update your PATH or manually replace the binary");
    }
}

/// Get the cargo bin directory (~/.cargo/bin).
fn cargo_bin_dir() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|p| p.join("bin"))
}

/// Fetch the latest version from crates.io.
fn fetch_latest_version() -> Result<String> {
    let url = format!("https://crates.io/api/v1/crates/{CRATE_NAME}");

    let output = Command::new("curl")
        .args(["-sf", &url])
        .output()
        .context("Failed to run curl")?;

    if !output.status.success() {
        anyhow::bail!("Failed to fetch crate info from crates.io");
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Failed to parse crates.io response")?;

    json["crate"]["max_version"]
        .as_str()
        .map(String::from)
        .context("Could not find version in crates.io response")
}

/// Check if cargo-binstall is available.
fn has_cargo_binstall() -> bool {
    Command::new("cargo")
        .args(["binstall", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Update using cargo-binstall (fast, downloads pre-built binary).
fn run_cargo_binstall() -> Result<()> {
    let status = Command::new("cargo")
        .args(["binstall", CRATE_NAME, "-y", "--force"])
        .status()
        .context("Failed to run cargo binstall")?;

    if !status.success() {
        anyhow::bail!("cargo binstall failed");
    }
    Ok(())
}

/// Update using cargo install (slower, compiles from source).
fn run_cargo_install() -> Result<()> {
    let status = Command::new("cargo")
        .args(["install", CRATE_NAME, "--force"])
        .status()
        .context("Failed to run cargo install")?;

    if !status.success() {
        anyhow::bail!("cargo install failed");
    }
    Ok(())
}
