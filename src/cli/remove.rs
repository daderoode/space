use anyhow::Result;
use crate::core::{config::SpaceConfig, workspace};

pub fn run(name: &str, force: bool) -> Result<()> {
    if !force {
        anyhow::bail!("use --force to remove a workspace without confirmation, or run without --force to use the interactive TUI");
    }
    let cfg = SpaceConfig::load()?;
    workspace::remove_workspace(&cfg.workspaces.dir, name, true)?;
    println!("Removed workspace '{}'", name);
    Ok(())
}
