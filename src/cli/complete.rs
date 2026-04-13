use crate::core::config::SpaceConfig;
use crate::core::repo;
use crate::core::workspace;
use crate::CompleteTarget;
use anyhow::Result;
use std::collections::HashSet;

/// Escape colons for zsh _describe format (colon is the delimiter).
fn zsh_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(':', "\\:")
}

pub fn run(what: CompleteTarget) -> Result<()> {
    let cfg = SpaceConfig::load()?;

    match what {
        CompleteTarget::Workspaces => {
            let workspaces = workspace::list_workspaces(&cfg.workspaces.dir)?;
            for ws in &workspaces {
                let detail = workspace::workspace_detail(&cfg.workspaces.dir, &ws.name)
                    .unwrap_or_else(|_| workspace::Workspace {
                        name: ws.name.clone(),
                        path: ws.path.clone(),
                        repos: vec![],
                    });
                let branch = detail
                    .repos
                    .first()
                    .map(|r| r.branch.as_str())
                    .unwrap_or("empty");
                let count = detail.repos.len();
                println!(
                    "{}:{} · {} repo{}",
                    zsh_escape(&ws.name),
                    branch,
                    count,
                    if count == 1 { "" } else { "s" }
                );
            }
        }
        CompleteTarget::Repos => {
            let cache_path = SpaceConfig::cache_path();
            if let Some(repos) = repo::load_cache(&cache_path, u64::MAX) {
                for r in &repos {
                    let name = r
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let display_path = r.display().to_string().replace(
                        &dirs::home_dir()
                            .map(|h| h.display().to_string())
                            .unwrap_or_default(),
                        "~",
                    );
                    println!("{}:{}", zsh_escape(&name), display_path);
                }
            }
        }
        CompleteTarget::AvailableRepos { workspace } => {
            let ws_path = cfg.workspaces.dir.join(&workspace);
            let mut existing = HashSet::new();
            if ws_path.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&ws_path) {
                    for entry in rd.flatten() {
                        if entry.path().is_dir() {
                            existing.insert(entry.file_name().to_string_lossy().to_string());
                        }
                    }
                }
            }
            let cache_path = SpaceConfig::cache_path();
            if let Some(repos) = repo::load_cache(&cache_path, u64::MAX) {
                for r in &repos {
                    let name = r
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !existing.contains(&name) {
                        let display_path = r.display().to_string().replace(
                            &dirs::home_dir()
                                .map(|h| h.display().to_string())
                                .unwrap_or_default(),
                            "~",
                        );
                        println!("{}:{}", zsh_escape(&name), display_path);
                    }
                }
            }
        }
    }
    Ok(())
}
