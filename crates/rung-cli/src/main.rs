//! Rung CLI - The developer's ladder for stacked PRs.

use clap::Parser;

mod commands;
mod output;

use commands::{Cli, Commands};

fn main() {
    let cli = Cli::parse();
    let json = cli.json;

    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Create { name } => commands::create::run(&name),
        Commands::Status { fetch } => commands::status::run(json, fetch),
        Commands::Sync {
            dry_run,
            continue_,
            abort,
            no_push,
            base,
        } => commands::sync::run(json, dry_run, continue_, abort, no_push, base.as_deref()),
        Commands::Submit {
            draft,
            force,
            title,
        } => commands::submit::run(json, draft, force, title.as_deref()),
        Commands::Undo => commands::undo::run(),
        Commands::Merge { method, no_delete } => commands::merge::run(json, &method, no_delete),
        Commands::Nxt => commands::navigate::run_next(),
        Commands::Prv => commands::navigate::run_prev(),
        Commands::Move => commands::mv::run(),
        Commands::Doctor => commands::doctor::run(json),
        Commands::Update { check } => commands::update::run(check),
        Commands::Completions { shell } => commands::completions::run(shell),
        Commands::Log => commands::log::run(),
    };

    if let Err(e) = result {
        output::error(&e.to_string());
        std::process::exit(1);
    }
}
