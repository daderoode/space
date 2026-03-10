use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;
use colored::Colorize;

pub fn run(name: &str) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let ws = workspace::workspace_detail(&cfg.workspaces.dir, name)?;

    println!("{} {}", "Workspace:".bold(), ws.name.cyan().bold());
    println!("{} {}", "Path:     ".bold(), ws.path.display());

    if ws.repos.is_empty() {
        println!("\n  (no repos)");
        return Ok(());
    }

    println!();
    for repo in &ws.repos {
        println!("  {}", repo.name.cyan().bold());
        println!("    Branch: {}", repo.branch.green());

        if repo.status.modified + repo.status.staged + repo.status.untracked == 0 {
            println!("    Status: {}", "clean".green());
        } else {
            println!(
                "    Status: {}",
                format!(
                    "{} modified, {} staged, {} untracked",
                    repo.status.modified, repo.status.staged, repo.status.untracked
                )
                .yellow()
            );
        }

        if repo.ahead != 0 || repo.behind != 0 {
            println!(
                "    Tracking: ahead {}, behind {}",
                repo.ahead.to_string().yellow(),
                repo.behind.to_string().yellow()
            );
        }
        println!();
    }
    Ok(())
}
