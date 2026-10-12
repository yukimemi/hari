mod install;
#[cfg(feature = "self-update")]
mod updater;

use std::path::Path;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Run native Git pre-push hooks for jj git push")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install and register a native Git helper in jj configuration.
    Install(install::Options),
    /// Update hari to the latest GitHub release.
    #[cfg(feature = "self-update")]
    SelfUpdate {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Report availability without installing an update.
        #[arg(long)]
        check: bool,
        /// Fail rather than prompt if an update requires confirmation.
        #[arg(long)]
        non_interactive: bool,
    },
}

fn run() -> Result<i32> {
    let mut arguments = std::env::args_os();
    let invoked_as = arguments.next().unwrap_or_default();
    let executable = std::env::current_exe()?;
    if install::is_helper(Path::new(&invoked_as)) || install::is_helper(&executable) {
        let real_git = install::real_git_for_proxy(&executable)?;
        return hari::proxy::run(&real_git, &arguments.collect::<Vec<_>>());
    }

    match Cli::parse().command {
        Commands::Install(options) => install::run(options)?,
        #[cfg(feature = "self-update")]
        Commands::SelfUpdate {
            yes,
            check,
            non_interactive,
        } => updater::run(yes, check, non_interactive)?,
    }
    Ok(0)
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("hari: {error:#}");
            1
        }
    };
    std::process::exit(code);
}
