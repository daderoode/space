use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;

pub fn run(name: &str) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let ws = workspace::workspace_detail(&cfg.workspaces.dir, name)?;

    println!("Workspace: {} ({})", ws.name, ws.path.display());
    if ws.repos.is_empty() {
        println!("  (no repos)");
        return Ok(());
    }
    println!(
        "{:<30} {:<20} {:>6} {:>6} {:>9}",
        "REPO", "BRANCH", "MOD", "STAGED", "+/-"
    );
    println!("{}", "-".repeat(72));
    for repo in &ws.repos {
        let ahead_behind = format!("+{} -{}", repo.ahead, repo.behind);
        println!(
            "{:<30} {:<20} {:>6} {:>6} {:>9}",
            repo.name, repo.branch, repo.status.modified, repo.status.staged, ahead_behind,
        );
    }
    Ok(())
}
