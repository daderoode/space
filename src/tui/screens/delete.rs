use super::default_no_confirm;
use crate::core::workspace::{self, PendingWork, UncommittedWork};
use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::KeyEvent;
use std::path::Path;

/// Where the delete dialog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteStage {
    /// `Delete workspace?`: the first question, and for a clean space the
    /// only one.
    Confirm,
    /// The first `y` was given on a space whose repos hold uncommitted work,
    /// and the dialog now asks whether to throw it away (issue #75). Default
    /// No, like the first question.
    ConfirmDiscard,
}

#[derive(Debug)]
pub struct DeleteState {
    pub workspace_name: String,
    pub repo_names: Vec<String>,
    /// The repos whose removal would lose work, read when the dialog opened
    /// (`workspace::pending_work`). Empty for a clean space, and then the
    /// dialog behaves as it always has: one `y` deletes.
    pub pending: Vec<PendingWork>,
    pub stage: DeleteStage,
    /// The second `y` found the repos holding something other than what the
    /// dialog showed, so the dialog was refreshed instead of deleting, and
    /// says so.
    pub changed: bool,
}

impl DeleteState {
    /// Open the dialog on space `name` in `ws_dir`, reading what each of its
    /// repos holds now, fresh, rather than trusting the dashboard's counts,
    /// which were read whenever the space was last selected.
    ///
    /// A space that cannot be read at all (gone, say) opens with nothing
    /// pending: the removal itself then fails with the reason, and under
    /// `UncommittedWork::Refuse` it reads every repo again before it touches
    /// any, so what this read misses is still never deleted without the
    /// second question. A repo this read finds and the dashboard does not
    /// list is added to the list, so every warning has a row.
    pub fn new(ws_dir: &Path, name: &str, mut repo_names: Vec<String>) -> Self {
        let pending = workspace::pending_work(ws_dir, name).unwrap_or_default();
        add_missing_rows(&mut repo_names, &pending);
        DeleteState {
            workspace_name: name.to_string(),
            repo_names,
            pending,
            stage: DeleteStage::Confirm,
            changed: false,
        }
    }

    /// What repo `name` would lose, if anything.
    pub fn pending_for(&self, name: &str) -> Option<&PendingWork> {
        self.pending.iter().find(|p| p.repo == name)
    }

    /// Default-No confirmation shared with `ConfirmPush` and `RebaseConfirm`:
    /// only an explicit `y`/`Y` deletes, so a reflex Enter can never destroy
    /// a workspace.
    ///
    /// A clean space is deleted on the first `y`, under
    /// `UncommittedWork::Refuse`, so work that appeared after the dialog
    /// opened still stops it and shows as an error status. A space with
    /// pending work asks a second, default-No question first, and only a `y`
    /// to that deletes with `UncommittedWork::Discard`. Every decline, at
    /// either question, leaves the dialog with nothing deleted.
    ///
    /// `Discard` throws away whatever the repos hold when it runs, not what
    /// the dialog showed. So the second `y` reads the space again first, and
    /// if that differs from what was shown (work appeared in another repo, or
    /// went), the dialog shows the new reading and asks again rather than
    /// discarding something the user never saw. A space that reads clean by
    /// then goes back to the first question, whose `y` deletes it under
    /// `Refuse`.
    pub fn handle_key(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        // Nothing is typed here, so `?` always opens help.
        if super::opens_help(key.code, true) {
            return ScreenAction::OpenHelp;
        }
        match (default_no_confirm(key.code), self.stage) {
            (Some(true), DeleteStage::Confirm) if !self.pending.is_empty() => {
                self.stage = DeleteStage::ConfirmDiscard;
                ScreenAction::Continue
            }
            (Some(true), DeleteStage::Confirm) => ScreenAction::DeleteWorkspace {
                name: self.workspace_name.clone(),
                force: true,
                uncommitted: UncommittedWork::Refuse,
            },
            (Some(true), DeleteStage::ConfirmDiscard) => {
                let now = workspace::pending_work(&ctx.config.workspaces.dir, &self.workspace_name)
                    .unwrap_or_default();
                if now != self.pending {
                    add_missing_rows(&mut self.repo_names, &now);
                    if now.is_empty() {
                        self.stage = DeleteStage::Confirm;
                    }
                    self.pending = now;
                    self.changed = true;
                    return ScreenAction::Continue;
                }
                ScreenAction::DeleteWorkspace {
                    name: self.workspace_name.clone(),
                    force: true,
                    uncommitted: UncommittedWork::Discard,
                }
            }
            (Some(false), _) => ScreenAction::Back,
            (None, _) => ScreenAction::Continue,
        }
    }
}

/// Give every repo with pending work a row, for a repo the dashboard did not
/// list (its detail was read before the repo appeared, say).
fn add_missing_rows(repo_names: &mut Vec<String>, pending: &[PendingWork]) {
    for p in pending {
        if !repo_names.contains(&p.repo) {
            repo_names.push(p.repo.clone());
        }
    }
}
