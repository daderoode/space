use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;
use colored::Colorize;

pub fn run(verbose: bool) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let workspaces = workspace::list_workspaces(&cfg.workspaces.dir)?;

    if workspaces.is_empty() {
        println!("No workspaces. Use `space create` to make one.");
        return Ok(());
    }

    for ws in &workspaces {
        if verbose {
            let detail =
                workspace::workspace_detail(&cfg.workspaces.dir, &ws.name).unwrap_or_else(|_| {
                    workspace::Workspace {
                        name: ws.name.clone(),
                        path: ws.path.clone(),
                        repos: vec![],
                    }
                });
            println!("{}  ({} repos)", ws.name.cyan().bold(), detail.repos.len());
            for repo in &detail.repos {
                let status_label = if repo.status.modified + repo.status.staged > 0 {
                    "modified".yellow().to_string()
                } else {
                    "clean".green().to_string()
                };
                println!(
                    "  {:<30} {}  [{}]",
                    repo.name,
                    repo.branch.green(),
                    status_label
                );
            }
        } else {
            // Count git worktrees (dirs with a .git file/dir) to match verbose output
            let repo_count = std::fs::read_dir(&ws.path)
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| {
                            let p = e.path();
                            p.is_dir() && p.join(".git").exists()
                        })
                        .count()
                })
                .unwrap_or(0);
            println!("{}  ({} repos)", ws.name.cyan().bold(), repo_count);
        }
    }
    Ok(())
}
