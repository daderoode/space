use crate::core::{config::SpaceConfig, repo};
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

pub fn run(refresh: bool) -> Result<()> {
    let cfg = SpaceConfig::load()?;
    let cache_path = SpaceConfig::cache_path();

    if refresh || !cache_path.exists() {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        pb.set_message("Scanning for repos...");
        pb.enable_steady_tick(std::time::Duration::from_millis(80));

        let repos = repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth);
        repo::save_cache(&cache_path, &repos)?;
        pb.finish_with_message(format!("Found {} repos", repos.len()));
        for r in &repos {
            println!("{}", r.display());
        }
    } else {
        let repos: Vec<std::path::PathBuf> = repo::load_cache(&cache_path).unwrap_or_default();
        for r in &repos {
            println!("{}", r.display());
        }
    }
    Ok(())
}
