use crate::cli::{pick_branch_strategy, resolve_repos};
use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;
use dialoguer::Select;

pub fn run(ws_name: &str, repo_args: Vec<String>) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let repos = resolve_repos(&repo_args, &cfg);

    if repos.is_empty() {
        anyhow::bail!("No repos resolved.");
    }

    let strategy_opts = [
        format!("New branch '{}'", ws_name),
        "Select existing branch (per repo)".to_string(),
        "Detached HEAD".to_string(),
    ];
    let strategy_choice = Select::new()
        .with_prompt("Branch strategy")
        .items(&strategy_opts)
        .default(0)
        .interact()?;

    for repo_path in &repos {
        let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
        let strategy = match strategy_choice {
            0 => workspace::BranchStrategy::NewBranch(ws_name.to_string()),
            1 => pick_branch_strategy(repo_path, ws_name)?,
            _ => workspace::BranchStrategy::DetachedHead,
        };
        match workspace::create_worktree(repo_path, &cfg.workspaces.dir, ws_name, &strategy) {
            Ok(wt) => println!("  + {} → {}", repo_name, wt.display()),
            Err(e) => eprintln!("  ! {} failed: {}", repo_name, e),
        }
    }
    Ok(())
}
