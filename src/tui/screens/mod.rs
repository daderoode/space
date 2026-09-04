pub mod add;
pub mod config;
pub mod create;
pub mod delete;
pub mod diff;
pub mod gitops;
pub mod go;
pub mod help;
pub mod search;
pub mod switch_branch;
pub mod sync_report;

use crate::tui::widgets::fuzzy_picker::PickerItem;
use ratatui::crossterm::event::KeyCode;
use std::path::PathBuf;

/// The one default-No confirmation used by every destructive dialog (delete
/// workspace, publish branch, rebase): only an explicit `y`/`Y` confirms, so
/// a reflex Enter never mutates anything. Returns `Some(true)` to confirm,
/// `Some(false)` to decline (`n`/`N`/`q`/`Enter`/`Esc`) and `None` for keys
/// the dialog ignores.
pub(crate) fn default_no_confirm(code: KeyCode) -> Option<bool> {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n')
        | KeyCode::Char('N')
        | KeyCode::Char('q')
        | KeyCode::Enter
        | KeyCode::Esc => Some(false),
        _ => None,
    }
}

/// Whether this key press should open the help overlay.
///
/// `?` is a legitimate character in every text input, so it only opens help on
/// stages that type nothing; each screen passes a positive list of its own
/// non-text stages, so a stage added later fails safe by not opening help
/// rather than by swallowing a typed `?`. `F1` is handled app-wide and works
/// everywhere, including while typing.
pub(crate) fn opens_help(code: KeyCode, stage_types_nothing: bool) -> bool {
    stage_types_nothing && code == KeyCode::Char('?')
}

/// Build repo picker rows from repo paths, filling the branch and remote
/// columns from each repo's current state. Shared by the create and add
/// flows for both the initial picker and a rescan rebuild.
pub(crate) fn repo_items(repos: Vec<PathBuf>) -> Vec<PickerItem> {
    repos
        .into_iter()
        .map(|path| {
            let (branch, remote_url) = crate::core::git::repo_display_info(&path);
            PickerItem {
                branch,
                remote_url,
                ..PickerItem::from_path(path)
            }
        })
        .collect()
}
