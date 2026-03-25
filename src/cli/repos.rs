use crate::core::{config::SpaceConfig, repo};
use anyhow::Result;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};

fn scan_and_cache(
    cfg: &SpaceConfig,
    cache_path: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>> {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message("Scanning for repos...");
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    let found = repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth);
    repo::save_cache(cache_path, &found)?;
    pb.finish_and_clear();
    Ok(found)
}

pub fn run(refresh: bool) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let cache_path = SpaceConfig::cache_path();

    let repos = if !refresh {
        repo::load_cache(&cache_path, cfg.repos.cache_age_secs)
            .map(Ok)
            .unwrap_or_else(|| scan_and_cache(&cfg, &cache_path))?
    } else {
        scan_and_cache(&cfg, &cache_path)?
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
