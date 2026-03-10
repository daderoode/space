use crate::Commands;
use anyhow::Result;
use crate::tui;
use crate::tui::app::{App, Screen};
use crate::tui::screens;

pub mod go;
pub mod list;
pub mod remove;
pub mod repos;
pub mod status;

pub fn dispatch(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Ls { verbose } => list::run(verbose),
        Commands::Status { name } => status::run(&name),
        Commands::Repos { refresh } => repos::run(refresh),

        Commands::Go { name: None } => {
            let mut app = App::new()?;
            let state = screens::go::GoState::new(&app.workspaces);
            app.screen = Screen::GoWorkspace(state);
            tui::app::run(&mut app)?;
            if let Some(path) = app.space_cd_target {
                println!("__SPACE_CD__:{}", path.display());
            }
            Ok(())
        }

        Commands::Go { name: Some(name) } => go::run(Some(name)),

        Commands::Create { repos } => {
            let mut app = App::new()?;
            app.screen = Screen::CreateWorkspace(screens::create::CreateState::new(
                app.repos_cache.clone(),
                repos,
            ));
            tui::app::run(&mut app)?;
            if let Some(path) = app.space_cd_target {
                println!("__SPACE_CD__:{}", path.display());
            }
            Ok(())
        }

        Commands::Add { workspace, repos } => {
            let mut app = App::new()?;
            // Find workspace index and load its detail so we know existing repos
            let ws_idx = app.workspaces.iter().position(|w| w.name == workspace);
            if let Some(idx) = ws_idx {
                app.selected_ws = idx;
                app.load_selected_workspace_detail();
            }
            // Determine available repos (exclude those already in the workspace)
            let existing_names: std::collections::HashSet<String> = app
                .workspaces
                .get(app.selected_ws)
                .map(|w| w.repos.iter().map(|r| r.name.clone()).collect())
                .unwrap_or_default();
            let available: Vec<std::path::PathBuf> = app
                .repos_cache
                .iter()
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    !existing_names.contains(&name)
                })
                .cloned()
                .collect();
            let ws_path = app
                .workspaces
                .get(app.selected_ws)
                .map(|w| w.path.clone())
                .unwrap_or_else(|| app.config.workspaces.dir.join(&workspace));
            app.screen = Screen::AddRepos(screens::add::AddState::new(
                workspace.clone(),
                ws_path,
                available,
                repos,
            ));
            tui::app::run(&mut app)?;
            if let Some(path) = app.space_cd_target {
                println!("__SPACE_CD__:{}", path.display());
            }
            Ok(())
        }

        Commands::Config => {
            let mut app = App::new()?;
            app.screen = Screen::ConfigEditor(screens::config::ConfigState::from_config(&app.config));
            tui::app::run(&mut app)?;
            if let Some(path) = app.space_cd_target {
                println!("__SPACE_CD__:{}", path.display());
            }
            Ok(())
        }

        Commands::Rm { name, force: false } => {
            let mut app = App::new()?;
            // Build DeleteState: find workspace to get its path and repos
            let ws_detail = app.workspaces.iter().find(|w| w.name == name);
            let (ws_path, repo_names) = if let Some(ws) = ws_detail {
                (
                    ws.path.clone(),
                    ws.repos.iter().map(|r| r.name.clone()).collect(),
                )
            } else {
                (app.config.workspaces.dir.join(&name), vec![])
            };
            app.screen = Screen::ConfirmDelete(screens::delete::DeleteState {
                workspace_name: name.clone(),
                workspace_path: ws_path,
                repo_names,
            });
            tui::app::run(&mut app)?;
            if let Some(path) = app.space_cd_target {
                println!("__SPACE_CD__:{}", path.display());
            }
            Ok(())
        }

        Commands::Rm { name, force: true } => remove::run(&name, true),

        Commands::Completions { shell } => {
            crate::shell::print_completions(&shell);
            Ok(())
        }
    }
}

use crate::core::{config::SpaceConfig, repo, workspace::BranchStrategy};
use std::path::Path;

/// Interactively pick a branch strategy for a given repo.
/// Stub — dialoguer removed in v0.2.0 (Task 10/11 will rewrite with TUI).
pub fn pick_branch_strategy(_repo_path: &Path, ws_name: &str) -> Result<BranchStrategy> {
    Ok(BranchStrategy::NewBranch(ws_name.to_string()))
}

/// Resolve repo arg strings to paths using the cache + fuzzy match.
pub fn resolve_repos(args: &[String], cfg: &SpaceConfig) -> Vec<std::path::PathBuf> {
    let cache_path = SpaceConfig::cache_path();
    let all_repos = repo::load_cache(&cache_path)
        .unwrap_or_else(|| repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth));
    args.iter()
        .flat_map(|q| {
            let matches = repo::fuzzy_match(q, &all_repos);
            if matches.is_empty() {
                eprintln!("warning: no repo matching '{}'", q);
            }
            matches.into_iter().take(1)
        })
        .collect()
}
