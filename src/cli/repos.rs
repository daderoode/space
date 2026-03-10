use crate::core::{config::SpaceConfig, repo};
use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(refresh: bool) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let cache_path = SpaceConfig::cache_path();

    let repos = if refresh || !cache_path.exists() {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message("Scanning for repos...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        let found = repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth);
        repo::save_cache(&cache_path, &found)?;
        pb.finish_and_clear();
        found
    } else {
        repo::load_cache(&cache_path).unwrap_or_default()
    };

    println!("{}", "Discovered repositories:".bold());
    println!();
    for r in &repos {
        let name = r.file_name().unwrap_or_default().to_string_lossy();
        let parent = r
            .parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("  {}  ({})", name.cyan(), parent.blue());
    }
    println!();
    let roots: Vec<String> = cfg
        .repos
        .roots
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    println!(
        "{} repos found in: {}",
        repos.len().to_string().bold(),
        roots.join(" ")
    );
    Ok(())
}
