use crate::core::{config::SpaceConfig, workspace};
use anyhow::Result;
use dialoguer::FuzzySelect;

pub fn run(name: Option<String>) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let workspaces = workspace::list_workspaces(&cfg.workspaces.dir)?;

    if workspaces.is_empty() {
        anyhow::bail!("no workspaces found");
    }

    let ws_name = match name {
        Some(n) => n,
        None => {
            let names: Vec<&str> = workspaces.iter().map(|w| w.name.as_str()).collect();
            let idx = FuzzySelect::new()
                .with_prompt("Workspace")
                .items(&names)
                .interact()?;
            names[idx].to_string()
        }
    };

    let target = cfg.workspaces.dir.join(&ws_name);
    if !target.exists() {
        anyhow::bail!("workspace '{}' not found", ws_name);
    }

    // Print marker — zsh wrapper reads this and calls cd
    println!("__SPACE_CD__:{}", target.display());
    Ok(())
}
