//! Rung CLI - The developer's ladder for stacked PRs.

use clap::Parser;

mod commands;
mod output;

use commands::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Create { name } => commands::create::run(&name),
        Commands::Status { json, fetch } => commands::status::run(json, fetch),
        Commands::Sync { dry_run, continue_, abort } => {
            commands::sync::run(dry_run, continue_, abort)
        }
        Commands::Submit { draft, force } => commands::submit::run(draft, force),
        Commands::Undo => commands::undo::run(),
        Commands::Nxt => commands::navigate::run_next(),
        Commands::Prv => commands::navigate::run_prev(),
    };

    if let Err(e) = result {
        output::error(&e.to_string());
        std::process::exit(1);
    }
}
