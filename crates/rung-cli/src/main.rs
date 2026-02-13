//! Rung CLI - The developer's ladder for stacked PRs.

use clap::Parser;

mod commands;
mod output;
mod services;

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
        Commands::Adopt {
            branch,
            parent,
            dry_run,
        } => commands::adopt::run(branch.as_deref(), parent.as_deref(), dry_run),
        Commands::Create {
            name,
            message,
            dry_run,
        } => commands::create::run(name.as_deref(), message.as_deref(), dry_run),
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
        Commands::Restack {
            branch,
            onto,
            dry_run,
            continue_,
            abort,
            include_children,
            force,
        } => {
            let opts = commands::restack::RestackOptions {
                json,
                branch: branch.as_deref(),
                onto: onto.as_deref(),
                dry_run,
                continue_,
                abort,
                include_children,
                force,
            };
            commands::restack::run(&opts)
        }
        Commands::Doctor => commands::doctor::run(json),
        Commands::Update { check } => commands::update::run(check),
        Commands::Completions { shell } => commands::completions::run(shell),
        Commands::Log => commands::log::run(json),
        Commands::Absorb { dry_run, base } => commands::absorb::run(dry_run, base.as_deref()),
        Commands::Split {
            branch,
            dry_run,
            abort,
        } => {
            let opts = commands::split::SplitOptions {
                json,
                branch: branch.as_deref(),
                dry_run,
                abort,
            };
            commands::split::run(&opts)
        }
        Commands::Fold {
            branches,
            into_parent,
            include_children,
            dry_run,
            abort,
        } => {
            let opts = commands::fold::FoldOptions {
                json,
                branches: branches.iter().map(String::as_str).collect(),
                into_parent,
                include_children,
                dry_run,
                abort,
            };
            commands::fold::run(&opts)
        }
    };

    if let Err(e) = result {
        output::error(&e.to_string());
        std::process::exit(1);
    }
}
