use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;

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
            println!("{} ({} repos)", ws.name, detail.repos.len());
            for repo in &detail.repos {
                let dirty = if repo.status.modified + repo.status.staged > 0 {
                    " *"
                } else {
                    ""
                };
                println!("  {} [{}]{}", repo.name, repo.branch, dirty);
            }
        } else {
            println!("{}", ws.name);
        }
    }
    Ok(())
}
