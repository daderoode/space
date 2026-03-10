use anyhow::Result;
use crate::core::{config::SpaceConfig, workspace};

pub fn run(name: &str, force: bool) -> Result<()> {
    if !force {
        unreachable!("non-force remove handled by TUI in dispatch");
    }
    let cfg = SpaceConfig::load()?;
    workspace::remove_workspace(&cfg.workspaces.dir, name, true)?;
    println!("Removed workspace '{}'", name);
    Ok(())
}
