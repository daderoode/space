use crate::tui;
use crate::tui::app::{App, Screen};
use crate::tui::screens;
use crate::Commands;
use anyhow::Result;

pub mod go;
pub mod list;
pub mod remove;
pub mod repos;
pub mod status;

/// Emit the cd target path using the temp-file protocol when stdout is piped
/// (e.g. inside the zsh wrapper's `$(...)`), otherwise fall back to the legacy
/// stdout marker so bare-binary invocations still work.
///
/// New wrapper: sets `__SPACE_CD_FILE__` env var to a temp path; binary writes
/// the path there instead of stdout, keeping stdout connected to the terminal
/// so TUI rendering works.
pub(crate) fn emit_cd_target(path: &std::path::Path) {
    if let Ok(cdfile) = std::env::var("__SPACE_CD_FILE__") {
        std::fs::write(&cdfile, path.display().to_string()).ok();
    } else {
        println!("__SPACE_CD__:{}", path.display());
    }
}

/// Run the TUI event loop and emit the cd marker if a workspace was selected.
pub(crate) fn run_tui_and_emit_cd(app: &mut App) -> Result<()> {
    tui::app::run(app)?;
    if let Some(ref path) = app.space_cd_target {
        emit_cd_target(path);
    }
    Ok(())
}

pub fn dispatch(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Ls { verbose } => list::run(verbose),
        Commands::Status { name } => status::run(&name),
        Commands::Repos { refresh } => repos::run(refresh),

        Commands::Go { name: None } => {
            let mut app = App::new()?;
            let state = screens::go::GoState::new(&app.workspaces);
            app.screen = Screen::GoWorkspace(state);
            run_tui_and_emit_cd(&mut app)
        }

        Commands::Go { name: Some(name) } => go::run(Some(name)),

        Commands::Create { repos } => {
            let mut app = App::new()?;
            app.screen = Screen::CreateWorkspace(screens::create::CreateState::new(
                app.repos_cache.clone(),
                repos,
            ));
            run_tui_and_emit_cd(&mut app)
        }

        Commands::Add { workspace, repos } => {
            let mut app = App::new()?;
            // Find workspace index — bail early if not found
            let Some(idx) = app.workspaces.iter().position(|w| w.name == workspace) else {
                anyhow::bail!("workspace '{}' not found", workspace);
            };
            app.selected_ws = idx;
            app.load_selected_workspace_detail();
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
            app.screen = Screen::AddRepos(screens::add::AddState::new(
                workspace.clone(),
                available,
                repos,
            ));
            run_tui_and_emit_cd(&mut app)
        }

        Commands::Config => {
            let mut app = App::new()?;
            app.screen =
                Screen::ConfigEditor(screens::config::ConfigState::from_config(&app.config));
            run_tui_and_emit_cd(&mut app)
        }

        Commands::Rm { name, force: false } => {
            let mut app = App::new()?;
            // Build DeleteState: find workspace to get repo names for display
            let ws_detail = app.workspaces.iter().find(|w| w.name == name);
            let repo_names: Vec<String> = if let Some(ws) = ws_detail {
                ws.repos.iter().map(|r| r.name.clone()).collect()
            } else {
                vec![]
            };
            app.screen = Screen::ConfirmDelete(screens::delete::DeleteState {
                workspace_name: name.clone(),
                repo_names,
            });
            run_tui_and_emit_cd(&mut app)
        }

        Commands::Rm { name, force: true } => remove::run(&name, true),

        Commands::Completions { shell } => crate::shell::print_completions(&shell),
    }
}
