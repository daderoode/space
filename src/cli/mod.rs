use crate::tui;
use crate::tui::app::{App, Screen};
use crate::tui::screens;
use crate::Commands;
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub mod complete;
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

/// The directory name a repo is picked by: what `space repos` prints and
/// what the shell completion offers.
fn repo_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Resolve the repo names given on the command line (`space create a b`,
/// `space add ws a b`) to cached repo paths, each once. The result keeps the
/// argument order, though the picker's toggle set does not carry it through.
///
/// A name must equal exactly one cached repo's directory name, as it is on
/// disk and case-sensitive. A name is never a fuzzy query here: joined into
/// one, several names would be an AND over a single repo and match nothing
/// (ticket 23). The picker still takes a typed query interactively. A name
/// that matches no repo, or several, is an error before any terminal is
/// touched.
pub(crate) fn resolve_repo_names(repos: &[PathBuf], names: &[String]) -> Result<Vec<PathBuf>> {
    let mut resolved: Vec<PathBuf> = Vec::new();
    for name in names {
        // The cache holds a repo once per root it sits under, so overlapping
        // roots list the same path twice; two entries of one path are one repo.
        let mut matches: Vec<&PathBuf> = repos.iter().filter(|p| repo_name(p) == *name).collect();
        matches.sort();
        matches.dedup();
        match matches.as_slice() {
            [one] => {
                if !resolved.contains(one) {
                    resolved.push((*one).clone());
                }
            }
            [] => {
                if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
                    anyhow::bail!(
                        "'{name}' is not a repo name; pass the directory name exactly as 'space repos' lists it, with no path or scope"
                    );
                }
                let lower = name.to_lowercase();
                if let Some(near) = repos
                    .iter()
                    .map(|p| repo_name(p))
                    .find(|n| n.to_lowercase() == lower)
                {
                    anyhow::bail!(
                        "no repo named '{name}' in the repo list; names are case-sensitive, did you mean '{near}'?"
                    );
                }
                anyhow::bail!(
                    "no repo named '{name}' in the repo list (run 'space repos --refresh' to rescan)"
                );
            }
            many => {
                let mut msg = format!("'{name}' names {} repos:", many.len());
                for p in many {
                    msg.push_str(&format!("\n  {}", p.display()));
                }
                msg.push_str("\npick it in the picker instead");
                anyhow::bail!(msg);
            }
        }
    }
    Ok(resolved)
}

/// The screen `space create <names...>` opens: the names resolved against
/// the cache and toggled on in the picker.
pub(crate) fn create_screen(
    repos_cache: Vec<PathBuf>,
    names: &[String],
) -> Result<screens::create::CreateState> {
    let preselected = resolve_repo_names(&repos_cache, names)?;
    Ok(screens::create::CreateState::new(repos_cache, preselected))
}

/// The screen `space add <workspace> <names...>` opens. `held` are the names
/// of the repos the workspace already has. Names resolve against the whole
/// cache first, so a held repo is reported as held rather than unknown; the
/// held repos are then left out of the picker.
pub(crate) fn add_screen(
    workspace: &str,
    held: &HashSet<String>,
    repos_cache: &[PathBuf],
    names: &[String],
) -> Result<screens::add::AddState> {
    let preselected = resolve_repo_names(repos_cache, names)?;
    if let Some(in_space) = preselected.iter().find(|p| held.contains(&repo_name(p))) {
        anyhow::bail!(
            "repo '{}' is already in workspace '{}'",
            repo_name(in_space),
            workspace
        );
    }
    let available: Vec<PathBuf> = repos_cache
        .iter()
        .filter(|p| !held.contains(&repo_name(p)))
        .cloned()
        .collect();
    Ok(screens::add::AddState::new(
        workspace.to_string(),
        available,
        preselected,
    ))
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
            app.screen = Screen::CreateWorkspace(create_screen(app.repos_cache.clone(), &repos)?);
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
            app.screen = Screen::AddRepos(add_screen(
                &workspace,
                &existing_names,
                &app.repos_cache,
                &repos,
            )?);
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

        Commands::Mcp => crate::mcp::run(),

        Commands::Init { shell } => crate::shell::print_init(&shell),

        Commands::Complete { what } => crate::cli::complete::run(what),
    }
}

#[cfg(test)]
mod tests {
    use super::{add_screen, create_screen, resolve_repo_names};
    use crate::tui::widgets::fuzzy_picker::FuzzyPicker;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn toggled(picker: &FuzzyPicker) -> Vec<String> {
        let mut v: Vec<String> = picker
            .toggled
            .iter()
            .map(|&i| picker.all_items[i].full_path.display().to_string())
            .collect();
        v.sort();
        v
    }

    /// The `space create` arm hands the resolved paths to the screen.
    #[test]
    fn create_screen_toggles_the_named_repos() {
        let st = create_screen(
            cache(&["/r/api", "/r/web", "/r/other"]),
            &names(&["web", "api"]),
        )
        .unwrap();
        assert_eq!(toggled(&st.picker), ["/r/api", "/r/web"]);
        assert_eq!(st.picker.input.value(), "");
        assert_eq!(st.picker.all_items.len(), 3);
    }

    /// The `space add` arm resolves, refuses a held repo, then drops the
    /// held repos from the picker and toggles the rest.
    #[test]
    fn add_screen_toggles_the_named_repos_and_leaves_held_ones_out() {
        let held: HashSet<String> = ["other".to_string()].into_iter().collect();
        let repos = cache(&["/r/api", "/r/web", "/r/other"]);
        let st = add_screen("ws", &held, &repos, &names(&["web"])).unwrap();
        assert_eq!(toggled(&st.picker), ["/r/web"]);
        let listed: Vec<&str> = st
            .picker
            .all_items
            .iter()
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(listed, ["api", "web"]);
        let err = add_screen("ws", &held, &repos, &names(&["other"])).unwrap_err();
        assert_eq!(err.to_string(), "repo 'other' is already in workspace 'ws'");
    }

    /// A scope, a path or an empty argument is refused as not a name, without
    /// the rescan hint, which could not help.
    #[test]
    fn resolve_repo_names_refuses_a_path_or_empty_argument_as_not_a_name() {
        let repos = cache(&["/r/api"]);
        for arg in ["acme/", "./api", "/r/api", "", ".", ".."] {
            let err = resolve_repo_names(&repos, &names(&[arg])).unwrap_err();
            assert!(
                err.to_string()
                    .starts_with(&format!("'{arg}' is not a repo name;")),
                "{arg:?}: {err}"
            );
            assert!(!err.to_string().contains("rescan"), "{arg:?}: {err}");
        }
    }

    fn cache(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolve_repo_names_is_exact_not_fuzzy() {
        let repos = cache(&["/r/api-service", "/r/api"]);
        let got = resolve_repo_names(&repos, &names(&["api"])).unwrap();
        assert_eq!(got, cache(&["/r/api"]));
        let err = resolve_repo_names(&repos, &names(&["ap"])).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("no repo named 'ap' in the repo list"),
            "{err}"
        );
    }

    #[test]
    fn resolve_repo_names_keeps_argument_order_and_drops_repeats() {
        let repos = cache(&["/r/api", "/r/web", "/r/other"]);
        let got = resolve_repo_names(&repos, &names(&["web", "api", "web"])).unwrap();
        assert_eq!(got, cache(&["/r/web", "/r/api"]));
    }

    #[test]
    fn resolve_repo_names_with_no_names_selects_nothing() {
        let repos = cache(&["/r/api"]);
        assert!(resolve_repo_names(&repos, &[]).unwrap().is_empty());
    }

    #[test]
    fn resolve_repo_names_refuses_a_case_only_mismatch_and_names_the_exact_one() {
        let repos = cache(&["/r/api"]);
        let err = resolve_repo_names(&repos, &names(&["API"])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no repo named 'API' in the repo list; names are case-sensitive, did you mean 'api'?"
        );
    }

    /// Overlapping roots (`~/p` and `~/p/work`) cache a repo once per root.
    #[test]
    fn resolve_repo_names_treats_a_path_cached_twice_as_one_repo() {
        let repos = cache(&["/p/work/api", "/p/web", "/p/work/api"]);
        let got = resolve_repo_names(&repos, &names(&["api"])).unwrap();
        assert_eq!(got, cache(&["/p/work/api"]));
    }

    #[test]
    fn resolve_repo_names_refuses_an_ambiguous_name_with_every_path() {
        let repos = cache(&["/a/api", "/b/api", "/r/web"]);
        let err = resolve_repo_names(&repos, &names(&["web", "api"])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "'api' names 2 repos:\n  /a/api\n  /b/api\npick it in the picker instead"
        );
    }

    #[test]
    fn resolve_repo_names_stops_at_the_first_failing_name() {
        let repos = cache(&["/r/api"]);
        let err = resolve_repo_names(&repos, &names(&["nope", "API"])).unwrap_err();
        assert!(err.to_string().starts_with("no repo named 'nope'"), "{err}");
    }
}
