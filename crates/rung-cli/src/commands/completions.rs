//! Shell completion generation.

use std::io;

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use super::Cli;

/// Generate shell completions and print to stdout.
#[allow(clippy::unnecessary_wraps)]
pub fn run(shell: Shell) -> anyhow::Result<()> {
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "rung", &mut io::stdout());
    Ok(())
}
