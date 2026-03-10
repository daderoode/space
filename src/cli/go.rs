use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;

pub fn run(name: Option<String>) -> Result<()> {
    match name {
        None => anyhow::bail!("go requires a workspace name"),
        Some(n) => {
            let cfg = SpaceConfig::load()?;
            let workspaces = workspace::list_workspaces(&cfg.workspaces.dir)?;
            let ws = workspaces
                .iter()
                .find(|w| w.name == n)
                .ok_or_else(|| anyhow::anyhow!("workspace '{}' not found", n))?;
            crate::cli::emit_cd_target(&ws.path);
            Ok(())
        }
    }
}
