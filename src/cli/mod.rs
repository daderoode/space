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
        Commands::Completions { shell } => {
            use clap::CommandFactory;
            crate::shell::completions::generate(shell, &mut crate::Cli::command());
            Ok(())
        }
    }
}

use crate::core::{config::SpaceConfig, git, repo, workspace::BranchStrategy};
use std::path::Path;

/// Interactively pick a branch strategy for a given repo.
pub fn pick_branch_strategy(repo_path: &Path, ws_name: &str) -> Result<BranchStrategy> {
    let options = [
        format!("New branch '{}' off default", ws_name),
        "Select existing branch".to_string(),
        "Detached HEAD at default branch".to_string(),
    ];
    let idx = dialoguer::Select::new()
        .with_prompt("Branch strategy")
        .items(&options)
        .default(0)
        .interact()?;

    match idx {
        0 => Ok(BranchStrategy::NewBranch(ws_name.to_string())),
        1 => {
            let branches = git::list_branches(repo_path)?;
            let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
            if names.is_empty() {
                anyhow::bail!("no branches found");
            }
            let sel = dialoguer::FuzzySelect::new()
                .with_prompt("Branch")
                .items(&names)
                .interact()?;
            Ok(BranchStrategy::ExistingBranch(names[sel].to_string()))
        }
        _ => Ok(BranchStrategy::DetachedHead),
    }
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
