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
use std::path::PathBuf;

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
