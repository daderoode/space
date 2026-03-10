use clap::{CommandFactory, Parser, Subcommand};

mod cli;
mod core;
mod shell;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(cmd) => cli::dispatch(cmd),
        None => {
            // TUI deferred to v0.2.0 — print help for now
            Cli::command().print_help()?;
            Ok(())
        }
    }
}

#[derive(Parser)]
#[command(
    name = "space",
    about = "Workspace manager for multi-repo git worktrees",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List workspaces
    #[command(alias = "list")]
    Ls {
        #[arg(short, long)]
        verbose: bool,
    },
    /// Show workspace detail
    #[command(alias = "st")]
    Status { name: String },
    /// Create a new workspace
    Create { repos: Vec<String> },
    /// Add repos to an existing workspace
    Add {
        workspace: String,
        repos: Vec<String>,
    },
    /// Remove a workspace
    #[command(alias = "remove")]
    Rm {
        name: String,
        #[arg(short, long)]
        force: bool,
    },
    /// cd into a workspace (prints __SPACE_CD__ marker for shell wrapper)
    Go { name: Option<String> },
    /// List discoverable repos
    Repos {
        #[arg(short, long)]
        refresh: bool,
    },
    /// Edit configuration interactively
    Config,
    /// Generate shell completions
    Completions {
        /// Shell name (e.g. bash, zsh, fish)
        shell: String,
    },
}
