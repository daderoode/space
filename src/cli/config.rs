use crate::core::config::SpaceConfig;
use anyhow::Result;
use dialoguer::Input;

pub fn run() -> Result<()> {
    let mut cfg = SpaceConfig::load()?;

    println!("Current configuration (press enter to keep current value):\n");

    let ws_dir: String = Input::new()
        .with_prompt("Workspaces directory")
        .default(cfg.workspaces.dir.to_string_lossy().to_string())
        .interact_text()?;
    cfg.workspaces.dir = ws_dir.into();

    let roots_str = cfg
        .repos
        .roots
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(",");
    let new_roots: String = Input::new()
        .with_prompt("Repo roots (comma-separated)")
        .default(roots_str)
        .interact_text()?;
    cfg.repos.roots = new_roots
        .split(',')
        .map(|s| std::path::PathBuf::from(s.trim()))
        .collect();

    let depth: String = Input::new()
        .with_prompt("Max search depth")
        .default(cfg.repos.max_depth.to_string())
        .interact_text()?;
    cfg.repos.max_depth = depth.parse().unwrap_or(cfg.repos.max_depth);

    let age: String = Input::new()
        .with_prompt("Cache age (seconds)")
        .default(cfg.repos.cache_age_secs.to_string())
        .interact_text()?;
    cfg.repos.cache_age_secs = age.parse().unwrap_or(cfg.repos.cache_age_secs);

    cfg.save()?;
    println!("\nSaved to {}", SpaceConfig::config_path().display());
    Ok(())
}
