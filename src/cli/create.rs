use crate::cli::{pick_branch_strategy, resolve_repos};
use crate::core::{config::SpaceConfig, repo, workspace};
use anyhow::Result;
use dialoguer::{Input, MultiSelect, Select};

pub fn run(repo_args: Vec<String>) -> Result<()> {
    let cfg = SpaceConfig::load()?;

    let ws_name: String = Input::new().with_prompt("Workspace name").interact_text()?;

    let repos = if repo_args.is_empty() {
        let cache_path = SpaceConfig::cache_path();
        let all = repo::load_cache(&cache_path).unwrap_or_default();
        if all.is_empty() {
            anyhow::bail!("No repos in cache. Run `space repos --refresh` first.");
        }
        let names: Vec<String> = all
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let selected = MultiSelect::new()
            .with_prompt("Select repos (space to toggle, enter to confirm)")
            .items(&names)
            .interact()?;
        if selected.is_empty() {
            anyhow::bail!("No repos selected.");
        }
        selected.into_iter().map(|i| all[i].clone()).collect()
    } else {
        resolve_repos(&repo_args, &cfg)
    };

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

    let ws_dir = &cfg.workspaces.dir;
    let mut created_any = false;

    for repo_path in &repos {
        let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
        let strategy = match strategy_choice {
            0 => workspace::BranchStrategy::NewBranch(ws_name.clone()),
            1 => pick_branch_strategy(repo_path, &ws_name)?,
            _ => workspace::BranchStrategy::DetachedHead,
        };

        match workspace::create_worktree(repo_path, ws_dir, &ws_name, &strategy) {
            Ok(wt) => {
                println!("  + {} → {}", repo_name, wt.display());
                created_any = true;
            }
            Err(e) => eprintln!("  ! {} failed: {}", repo_name, e),
        }
    }

    if created_any {
        let ws_path = ws_dir.join(&ws_name);
        println!("__SPACE_CD__:{}", ws_path.display());
    }
    Ok(())
}
