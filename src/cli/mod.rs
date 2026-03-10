use crate::Commands;
use anyhow::Result;

pub mod add;
pub mod config;
pub mod create;
pub mod go;
pub mod list;
pub mod remove;
pub mod repos;
pub mod status;

pub fn dispatch(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Ls { verbose } => list::run(verbose),
        Commands::Status { name } => status::run(&name),
        Commands::Go { name } => go::run(name),
        Commands::Repos { refresh } => repos::run(refresh),
        Commands::Create { repos } => create::run(repos),
        Commands::Add { workspace, repos } => add::run(&workspace, repos),
        Commands::Rm { name, force } => remove::run(&name, force),
        Commands::Config => config::run(),
        Commands::Completions { shell: _ } => {
            // clap_complete removed in v0.2.0 — stub until Task 10/11
            anyhow::bail!("shell completions not yet implemented in v0.2.0")
        }
    }
}

use crate::core::{config::SpaceConfig, repo, workspace::BranchStrategy};
use std::path::Path;

/// Interactively pick a branch strategy for a given repo.
/// Stub — dialoguer removed in v0.2.0 (Task 10/11 will rewrite with TUI).
pub fn pick_branch_strategy(_repo_path: &Path, ws_name: &str) -> Result<BranchStrategy> {
    Ok(BranchStrategy::NewBranch(ws_name.to_string()))
}

/// Resolve repo arg strings to paths using the cache + fuzzy match.
pub fn resolve_repos(args: &[String], cfg: &SpaceConfig) -> Vec<std::path::PathBuf> {
    let cache_path = SpaceConfig::cache_path();
    let all_repos = repo::load_cache(&cache_path)
        .unwrap_or_else(|| repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth));
    args.iter()
        .flat_map(|q| {
            let matches = repo::fuzzy_match(q, &all_repos);
            if matches.is_empty() {
                eprintln!("warning: no repo matching '{}'", q);
            }
            matches.into_iter().take(1)
        })
        .collect()
}
