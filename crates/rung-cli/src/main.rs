//! Rung CLI - The developer's ladder for stacked PRs.

use clap::Parser;

mod commands;
mod output;

use commands::{Cli, Commands};

fn main() {
    // Respect NO_COLOR environment variable (https://no-color.org/)
    if std::env::var("NO_COLOR").is_ok() {
        colored::control::set_override(false);
    }

    let cli = Cli::parse();
    output::set_quiet(cli.quiet);
    let json = cli.json;

    let result = match cli.command {
        Commands::Init => commands::init::run(),
        Commands::Create { name, message } => {
            commands::create::run(name.as_deref(), message.as_deref())
        }
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
            dry_run,
            force,
            title,
        } => commands::submit::run(json, dry_run, draft, force, title.as_deref()),
        Commands::Undo => commands::undo::run(),
        Commands::Merge { method, no_delete } => commands::merge::run(json, &method, no_delete),
        Commands::Nxt => commands::navigate::run_next(),
        Commands::Prv => commands::navigate::run_prev(),
        Commands::Move => commands::mv::run(),
        Commands::Doctor => commands::doctor::run(json),
        Commands::Update { check } => commands::update::run(check),
        Commands::Completions { shell } => commands::completions::run(shell),
        Commands::Log => commands::log::run(json),
        Commands::Absorb { dry_run, base } => commands::absorb::run(dry_run, base.as_deref()),
    };

    if let Err(e) = result {
        output::error(&e.to_string());
        std::process::exit(1);
    }
}
