use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;
use dialoguer::Confirm;

pub fn run(name: &str, force: bool) -> Result<()> {
    let cfg = SpaceConfig::load()?;

    if !force {
        if let Ok(detail) = workspace::workspace_detail(&cfg.workspaces.dir, name) {
            let dirty: Vec<&str> = detail
                .repos
                .iter()
                .filter(|r| r.status.modified + r.status.staged > 0)
                .map(|r| r.name.as_str())
                .collect();
            if !dirty.is_empty() {
                eprintln!("warning: dirty repos in '{}': {}", name, dirty.join(", "));
            }
        }

        let confirmed = Confirm::new()
            .with_prompt(format!("Remove workspace '{}'?", name))
            .default(false)
            .interact()?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    workspace::remove_workspace(&cfg.workspaces.dir, name, force)?;
    println!("Removed workspace '{}'.", name);
    Ok(())
}
