use clap::{Parser, Subcommand};

mod cli;
mod core;
mod mcp;
mod shell;
mod tui;

fn main() -> anyhow::Result<()> {
    let cli_args = Cli::parse();
    match cli_args.command {
        None => {
            // No args → TUI dashboard
            let mut app = tui::app::App::new()?;
            cli::run_tui_and_emit_cd(&mut app)
        }
        Some(cmd) => cli::dispatch(cmd),
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
    /// Generate shell completions (completion function only, for manual install)
    Completions {
        /// Shell name (only 'zsh' is supported)
        shell: String,
    },
    /// Start the MCP server (stdio transport)
    Mcp,
    /// Output shell init script (wrapper function + completions) for eval in .zshrc
    Init {
        /// Shell name (only 'zsh' is supported)
        shell: String,
    },
    /// Internal: emit completion data for the shell
    #[command(name = "__complete", hide = true)]
    Complete {
        #[command(subcommand)]
        what: CompleteTarget,
    },
}

#[doc(hidden)]
#[derive(Subcommand)]
pub enum CompleteTarget {
    /// Workspace names with context
    Workspaces,
    /// Repo basenames with paths
    Repos,
    /// Repos not yet in a workspace
    AvailableRepos { workspace: String },
}
