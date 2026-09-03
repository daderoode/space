//! Shared keybinding definitions consumed by the help overlay and status bar.

pub struct Binding {
    pub key: &'static str,
    pub desc: &'static str,
}

pub struct BindingGroup {
    pub name: &'static str,
    pub bindings: &'static [Binding],
}

const NAVIGATION: BindingGroup = BindingGroup {
    name: "Navigation",
    bindings: &[
        Binding {
            key: "Tab",
            desc: "Switch pane",
        },
        Binding {
            key: "↑/k",
            desc: "Up",
        },
        Binding {
            key: "↓/j",
            desc: "Down",
        },
        Binding {
            key: "h/l",
            desc: "Scroll table left/right (repo pane)",
        },
        Binding {
            key: "→",
            desc: "Expand / focus repos",
        },
        Binding {
            key: "←/Esc",
            desc: "Right pane: collapse / focus workspaces",
        },
    ],
};

const WORKSPACE_PANE: BindingGroup = BindingGroup {
    name: "Workspace Pane",
    bindings: &[
        Binding {
            key: "Enter",
            desc: "Go to workspace (cd)",
        },
        Binding {
            key: "c",
            desc: "Create workspace",
        },
        Binding {
            key: "a",
            desc: "Add repos",
        },
        Binding {
            key: "d",
            desc: "Delete workspace",
        },
        Binding {
            key: "g",
            desc: "Go (fuzzy picker)",
        },
        Binding {
            key: "r",
            desc: "Rescan repo list",
        },
        Binding {
            key: "/",
            desc: "Filter spaces",
        },
        Binding {
            key: "S",
            desc: "Config",
        },
    ],
};

const REPO_PANE: BindingGroup = BindingGroup {
    name: "Repo Pane",
    bindings: &[
        Binding {
            key: "Enter (repo)",
            desc: "Expand / collapse repo",
        },
        Binding {
            key: "Enter (file)",
            desc: "View file diff",
        },
        Binding {
            key: "←/Esc",
            desc: "Collapse all / back",
        },
        Binding {
            key: "s/space",
            desc: "Stage / unstage file",
        },
        Binding {
            key: "/",
            desc: "Search repos",
        },
        Binding {
            key: "S",
            desc: "Stage all unstaged",
        },
        Binding {
            key: "U",
            desc: "Unstage all staged",
        },
        Binding {
            key: "b",
            desc: "Switch branch",
        },
        Binding {
            key: "G",
            desc: "Git operations",
        },
    ],
};

const REPO_PICKER: BindingGroup = BindingGroup {
    name: "Repo Picker",
    bindings: &[
        Binding {
            key: "Tab",
            desc: "Toggle repo",
        },
        Binding {
            key: "Ctrl-S",
            desc: "Cycle scope",
        },
        Binding {
            key: "Ctrl-R",
            desc: "Rescan repo list",
        },
    ],
};

const GIT_OPS: BindingGroup = BindingGroup {
    name: "Git Operations",
    bindings: &[
        Binding {
            key: "f",
            desc: "Fetch",
        },
        Binding {
            key: "p",
            desc: "Pull",
        },
        Binding {
            key: "P",
            desc: "Push",
        },
        Binding {
            key: "c",
            desc: "Commit",
        },
        Binding {
            key: "l",
            desc: "Log",
        },
        Binding {
            key: "r",
            desc: "Rebase",
        },
        Binding {
            key: "\u{2191}\u{2193}/jk",
            desc: "Navigate (menu and log only)",
        },
        Binding {
            key: "enter",
            desc: "Select",
        },
        Binding {
            key: "esc/q",
            desc: "Close",
        },
    ],
};

const DIFF_VIEWER: BindingGroup = BindingGroup {
    name: "Diff Viewer",
    bindings: &[
        Binding {
            key: "up/k",
            desc: "Scroll up",
        },
        Binding {
            key: "dn/j",
            desc: "Scroll down",
        },
        Binding {
            key: "PgUp/PgDn",
            desc: "Page scroll",
        },
        Binding {
            key: "Home/End",
            desc: "Jump to start/end",
        },
        Binding {
            key: "s/space",
            desc: "Stage / unstage",
        },
        Binding {
            key: "Esc/q",
            desc: "Close",
        },
    ],
};

const SYNC_REPORT: BindingGroup = BindingGroup {
    name: "Sync Report",
    bindings: &[
        Binding {
            key: "\u{2191}\u{2193}/jk",
            desc: "Select repo (once done)",
        },
        Binding {
            key: "PgUp/PgDn",
            desc: "Page by 10 rows",
        },
        Binding {
            key: "Home/End",
            desc: "First / last repo",
        },
        Binding {
            key: "Enter",
            desc: "Continue to branch picker (once done)",
        },
        Binding {
            key: "Esc",
            desc: "Cancel / back to repo picker",
        },
    ],
};

const GENERAL: BindingGroup = BindingGroup {
    name: "General",
    bindings: &[
        Binding {
            key: "?",
            desc: "Help",
        },
        Binding {
            key: "q",
            desc: "Quit",
        },
        Binding {
            key: "Ctrl-C",
            desc: "Force quit",
        },
    ],
};

/// All binding groups — consumed by the help overlay.
pub fn all_groups() -> &'static [BindingGroup] {
    static GROUPS: [BindingGroup; 8] = [
        NAVIGATION,
        WORKSPACE_PANE,
        REPO_PANE,
        REPO_PICKER,
        GIT_OPS,
        DIFF_VIEWER,
        SYNC_REPORT,
        GENERAL,
    ];
    &GROUPS
}

/// Bindings to show in the status bar for a given pane.
/// Short labels suited for a single condensed line.
pub fn status_bar_bindings(pane: crate::tui::app::Pane) -> &'static [Binding] {
    static LEFT: [Binding; 10] = [
        Binding {
            key: "enter",
            desc: "go",
        },
        Binding {
            key: "→",
            desc: "repos",
        },
        Binding {
            key: "c",
            desc: "create",
        },
        Binding {
            key: "a",
            desc: "add",
        },
        Binding {
            key: "d",
            desc: "delete",
        },
        Binding {
            key: "r",
            desc: "rescan",
        },
        Binding {
            key: "/",
            desc: "filter",
        },
        Binding {
            key: "S",
            desc: "config",
        },
        Binding {
            key: "?",
            desc: "help",
        },
        Binding {
            key: "q",
            desc: "quit",
        },
    ];
    static RIGHT: [Binding; 11] = [
        Binding {
            key: "enter",
            desc: "expand/diff",
        },
        Binding {
            key: "←/esc",
            desc: "back",
        },
        Binding {
            key: "h/l",
            desc: "scroll",
        },
        Binding {
            key: "s/space",
            desc: "stage",
        },
        Binding {
            key: "S",
            desc: "stage all",
        },
        Binding {
            key: "U",
            desc: "unstage all",
        },
        Binding {
            key: "b",
            desc: "switch branch",
        },
        Binding {
            key: "G",
            desc: "git ops",
        },
        Binding {
            key: "/",
            desc: "search",
        },
        Binding {
            key: "?",
            desc: "help",
        },
        Binding {
            key: "q",
            desc: "quit",
        },
    ];
    match pane {
        crate::tui::app::Pane::Left => &LEFT,
        crate::tui::app::Pane::Right => &RIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::all_groups;

    /// The help overlay never wraps: each row is two spaces, the key padded
    /// to 12 columns and the description. On the 80-column default terminal
    /// the dialog is 56 wide, 54 inside the border, so every registered
    /// binding must fit 54 columns or its description is clipped there.
    #[test]
    fn every_binding_fits_the_default_terminal_help_dialog() {
        for group in all_groups() {
            for binding in group.bindings {
                let key_cols = binding.key.chars().count().max(12);
                let row = 2 + key_cols + binding.desc.chars().count();
                assert!(
                    row <= 54,
                    "{} / {:?} is {} columns wide, over the 54-column help dialog at 80 columns",
                    group.name,
                    binding.desc,
                    row
                );
            }
        }
    }
}
