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

/// Replace the home directory prefix with `~` for display purposes.
/// Uses `Path::strip_prefix` to ensure only an actual prefix match is replaced.
fn tilde_path(p: &std::path::Path) -> String {
    match dirs::home_dir() {
        Some(h) => match p.strip_prefix(&h) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        },
        None => p.display().to_string(),
    }
}

pub fn run(what: CompleteTarget) -> Result<()> {
    let cfg = SpaceConfig::load()?;

    match what {
        CompleteTarget::Workspaces => {
            // workspace_detail runs git-rev-parse per repo — acceptable for completion
            // latency but will be slow for workspaces with many repos.
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
                    let Some(n) = r.file_name() else { continue };
                    let name = n.to_string_lossy();
                    if name.is_empty() {
                        continue;
                    }
                    println!("{}:{}", zsh_escape(&name), zsh_escape(&tilde_path(r)));
                }
            }
        }
        CompleteTarget::AvailableRepos { workspace } => {
            let ws_path = cfg.workspaces.dir.join(&workspace);
            let mut existing = HashSet::new();
            if ws_path.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&ws_path) {
                    for entry in rd.flatten() {
                        let path = entry.path();
                        if path.is_dir() && path.join(".git").exists() {
                            existing.insert(entry.file_name().to_string_lossy().to_string());
                        }
                    }
                }
            }
            let cache_path = SpaceConfig::cache_path();
            if let Some(repos) = repo::load_cache(&cache_path, u64::MAX) {
                for r in &repos {
                    let Some(n) = r.file_name() else { continue };
                    let name = n.to_string_lossy();
                    if name.is_empty() || existing.contains(name.as_ref()) {
                        continue;
                    }
                    println!("{}:{}", zsh_escape(&name), zsh_escape(&tilde_path(r)));
                }
            }
        }
    }
    Ok(())
}
