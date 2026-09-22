use clap::{Parser, Subcommand};
// The library's modules, not copies of them: `cli` reaches them as `crate::`.
use space::{core, mcp, shell, tui};

mod cli;

// Keeps this binary's tests off the invoking user's git config (ticket 43).
#[cfg(test)]
#[path = "../tests/common/git_isolation.rs"]
mod git_isolation;

fn main() -> anyhow::Result<()> {
    let cli_args = Cli::parse();
    // `space mcp` installs its own stderr subscriber inside mcp::run().
    // Initializing the file subscriber first would cause that fmt().init()
    // call to panic (subscriber already set). Skip file logging for Mcp.
    let _log_guard = match &cli_args.command {
        Some(Commands::Mcp) => None,
        _ => space::logging::init(),
    };
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
    Create {
        /// Repos to pre-select, by exact directory name as `space repos` lists them (case-sensitive)
        repos: Vec<String>,
    },
    /// Add repos to an existing workspace
    Add {
        workspace: String,
        /// Repos to pre-select, by exact directory name as `space repos` lists them (case-sensitive)
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

#[cfg(test)]
mod tests {
    use std::any::{Any, TypeId};

    /// The `TypeId` of `value`'s type, which for a function is its own item type.
    fn type_of<T: Any>(_value: &T) -> TypeId {
        TypeId::of::<T>()
    }

    /// The binary takes `core`, `mcp`, `shell` and `tui` from the library
    /// rather than compiling its own copies (ticket 29). A module compiled
    /// twice gives each of its items two identities, so one item of each
    /// module, reached as `crate::` and as `space::`, must be one type.
    #[test]
    fn the_binary_uses_the_librarys_modules() {
        assert_eq!(
            TypeId::of::<crate::core::workspace::Workspace>(),
            TypeId::of::<space::core::workspace::Workspace>(),
            "the binary compiles its own copy of core"
        );
        assert_eq!(
            TypeId::of::<crate::mcp::SpaceServer>(),
            TypeId::of::<space::mcp::SpaceServer>(),
            "the binary compiles its own copy of mcp"
        );
        assert_eq!(
            type_of(&crate::shell::print_init),
            type_of(&space::shell::print_init),
            "the binary compiles its own copy of shell"
        );
        assert_eq!(
            TypeId::of::<crate::tui::app::App>(),
            TypeId::of::<space::tui::app::App>(),
            "the binary compiles its own copy of tui"
        );
    }
}
