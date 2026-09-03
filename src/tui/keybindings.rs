//! Shared keybinding definitions consumed by the help overlay and status bar.

#[derive(Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: &'static str,
    pub desc: &'static str,
}

pub struct BindingGroup {
    pub name: &'static str,
    pub bindings: &'static [Binding],
}

/// Group names, shared by the registry below and by
/// `screens::help::landing_group`, so a rename cannot silently send help
/// to the top of the list instead of to the group for the current screen.
pub const NAVIGATION_NAME: &str = "Navigation";
pub const WORKSPACE_PANE_NAME: &str = "Workspace Pane";
pub const REPO_PANE_NAME: &str = "Repo Pane";
pub const REPO_PICKER_NAME: &str = "Repo Picker";
pub const CREATE_ADD_FLOW_NAME: &str = "Create / Add Flow";
pub const CREATING_LOG_NAME: &str = "Creating Log";
pub const DELETE_CONFIRM_NAME: &str = "Delete Confirm";
pub const SWITCH_BRANCH_NAME: &str = "Switch Branch";
pub const CONFIG_EDITOR_NAME: &str = "Config Editor";
pub const SPACE_REPO_PICKERS_NAME: &str = "Space & Repo Pickers";
pub const GIT_OPS_NAME: &str = "Git Operations";
pub const DIFF_VIEWER_NAME: &str = "Diff Viewer";
pub const SYNC_REPORT_NAME: &str = "Sync Report";
pub const HELP_OVERLAY_NAME: &str = "Help Overlay";
pub const GENERAL_NAME: &str = "General";

const NAVIGATION: BindingGroup = BindingGroup {
    name: NAVIGATION_NAME,
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
            key: "PgUp/PgDn",
            desc: "Page up/down",
        },
        Binding {
            key: "Home/End",
            desc: "First / last row",
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
    name: WORKSPACE_PANE_NAME,
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
    name: REPO_PANE_NAME,
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
    name: REPO_PICKER_NAME,
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
    name: GIT_OPS_NAME,
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
            key: "PgUp/PgDn",
            desc: "Page the log",
        },
        Binding {
            key: "Home/End",
            desc: "Log top / bottom",
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
    name: DIFF_VIEWER_NAME,
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
    name: SYNC_REPORT_NAME,
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
    name: GENERAL_NAME,
    bindings: &[
        Binding {
            key: "r",
            desc: "Rescan repo list",
        },
        Binding {
            key: "?",
            desc: "Help (not while typing)",
        },
        Binding {
            key: "F1",
            desc: "Help (works while typing)",
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

const CREATE_ADD_FLOW: BindingGroup = BindingGroup {
    name: CREATE_ADD_FLOW_NAME,
    bindings: &[
        Binding {
            key: "Tab",
            desc: "Toggle repo in picker",
        },
        Binding {
            key: "Enter",
            desc: "Confirm and continue",
        },
        Binding {
            key: "↑↓",
            desc: "Choose branch strategy",
        },
        Binding {
            key: "Esc",
            desc: "Back one stage",
        },
    ],
};

const CREATING_LOG: BindingGroup = BindingGroup {
    name: CREATING_LOG_NAME,
    bindings: &[
        Binding {
            key: "\u{2191}\u{2193}/jk",
            desc: "Scroll the log",
        },
        Binding {
            key: "PgUp/PgDn",
            desc: "Page the log",
        },
        Binding {
            key: "Home/End",
            desc: "Top / bottom (End resumes follow)",
        },
        Binding {
            key: "Enter/Esc/q",
            desc: "Back to the dashboard",
        },
    ],
};

const DELETE_CONFIRM: BindingGroup = BindingGroup {
    name: DELETE_CONFIRM_NAME,
    bindings: &[
        Binding {
            key: "y",
            desc: "Delete the space",
        },
        Binding {
            key: "n/Esc/Enter",
            desc: "Cancel (the default)",
        },
        Binding {
            key: "q",
            desc: "Cancel",
        },
    ],
};

const SWITCH_BRANCH: BindingGroup = BindingGroup {
    name: SWITCH_BRANCH_NAME,
    bindings: &[
        Binding {
            key: "↑↓/jk",
            desc: "Choose strategy",
        },
        Binding {
            key: "Enter",
            desc: "Confirm",
        },
        Binding {
            key: "Esc",
            desc: "Back one stage",
        },
    ],
};

const CONFIG_EDITOR: BindingGroup = BindingGroup {
    name: CONFIG_EDITOR_NAME,
    bindings: &[
        Binding {
            key: "↑↓/jk",
            desc: "Move between fields",
        },
        Binding {
            key: "Enter",
            desc: "Edit field / commit edit",
        },
        Binding {
            key: "Ctrl-S",
            desc: "Save and exit",
        },
        Binding {
            key: "Esc",
            desc: "Cancel edit / close",
        },
    ],
};

const SPACE_REPO_PICKERS: BindingGroup = BindingGroup {
    name: SPACE_REPO_PICKERS_NAME,
    bindings: &[
        Binding {
            key: "↑↓",
            desc: "Move the highlight",
        },
        Binding {
            key: "Enter",
            desc: "Select",
        },
        Binding {
            key: "Esc",
            desc: "Cancel",
        },
        Binding {
            key: "letters",
            desc: "Type into the filter",
        },
    ],
};

const HELP_OVERLAY: BindingGroup = BindingGroup {
    name: HELP_OVERLAY_NAME,
    bindings: &[
        Binding {
            key: "↑↓/jk",
            desc: "Scroll",
        },
        Binding {
            key: "PgUp/PgDn",
            desc: "Page",
        },
        Binding {
            key: "Home/End",
            desc: "Top / bottom",
        },
        Binding {
            key: "Esc/q/?/F1",
            desc: "Close",
        },
    ],
};

/// All binding groups — consumed by the help overlay.
pub fn all_groups() -> &'static [BindingGroup] {
    static GROUPS: [BindingGroup; 15] = [
        NAVIGATION,
        WORKSPACE_PANE,
        REPO_PANE,
        REPO_PICKER,
        CREATE_ADD_FLOW,
        CREATING_LOG,
        DELETE_CONFIRM,
        SWITCH_BRANCH,
        CONFIG_EDITOR,
        SPACE_REPO_PICKERS,
        GIT_OPS,
        DIFF_VIEWER,
        SYNC_REPORT,
        HELP_OVERLAY,
        GENERAL,
    ];
    &GROUPS
}

/// Rendered lines the help overlay produces for the whole registry: one header
/// per group, one per binding, and one blank line between groups.
pub fn rendered_row_count() -> usize {
    let groups = all_groups();
    groups.iter().map(|g| 1 + g.bindings.len()).sum::<usize>() + groups.len().saturating_sub(1)
}

/// One rendered line of the help overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpRow {
    /// The blank line between two groups.
    Gap,
    /// A group's title line.
    Header(&'static str),
    /// One key and its description.
    Binding(&'static Binding),
}

/// The `visible` rendered rows starting at `offset`.
///
/// Pure so the windowing can be tested directly: an off-by-one here shifts
/// what the overlay shows without changing anything a content-sampling test
/// would notice. Row numbering matches `rendered_row_count` and
/// `group_row_offset` exactly, since all three walk the same shape.
pub fn help_rows(offset: usize, visible: usize) -> Vec<HelpRow> {
    let end = offset.saturating_add(visible);
    let mut out = Vec::new();
    let mut row = 0usize;
    for (i, group) in all_groups().iter().enumerate() {
        if row >= end {
            break;
        }
        if i > 0 {
            if row >= offset && row < end {
                out.push(HelpRow::Gap);
            }
            row += 1;
        }
        if row >= offset && row < end {
            out.push(HelpRow::Header(group.name));
        }
        row += 1;
        for binding in group.bindings {
            if row >= end {
                break;
            }
            if row >= offset {
                out.push(HelpRow::Binding(binding));
            }
            row += 1;
        }
    }
    out
}

/// The rendered line `group` starts on, for the help overlay's opening scroll.
/// An unknown name lands at the top rather than failing: a screen with no group
/// of its own is not an error.
pub fn group_row_offset(group: &str) -> usize {
    let mut row = 0usize;
    for (i, g) in all_groups().iter().enumerate() {
        if i > 0 {
            row += 1; // the blank line between groups
        }
        if g.name == group {
            return row;
        }
        row += 1 + g.bindings.len();
    }
    0
}

/// Bindings to show in the key bar for a given pane.
/// Short labels suited for a single condensed line. The renderer drops
/// entries that do not fit, always keeping the help key.
pub fn key_bar_bindings(pane: crate::tui::app::Pane) -> &'static [Binding] {
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
    use super::{all_groups, group_row_offset, rendered_row_count};

    /// The help overlay never wraps: each row is two spaces, the key padded to
    /// 12 columns, one guaranteed space and the description. The dialog is at
    /// least 56 wide, 54 inside the border, so every registered binding must
    /// fit 54 columns or its description is clipped.
    ///
    /// The width floor is 56 rather than 50 precisely so this budget holds at
    /// every terminal width the overlay is drawn at, not only at 80 columns.
    #[test]
    fn every_binding_fits_the_help_dialog() {
        for group in all_groups() {
            for binding in group.bindings {
                let key_cols = binding.key.chars().count().max(12);
                // +1 for the space the renderer always emits between the key
                // column and the description, so a 12-character key cannot run
                // into its own text.
                let row = 2 + key_cols + 1 + binding.desc.chars().count();
                assert!(
                    row <= 54,
                    "{} / {:?} is {} columns wide, over the 54-column help dialog",
                    group.name,
                    binding.desc,
                    row
                );
            }
        }
    }

    /// Group offsets are what the overlay scrolls to when help is opened from
    /// a flow, so they must match the lines the renderer actually emits.
    #[test]
    fn group_row_offsets_match_the_rendered_line_count() {
        let groups = all_groups();
        let mut row = 0usize;
        for (i, g) in groups.iter().enumerate() {
            if i > 0 {
                row += 1;
            }
            assert_eq!(
                group_row_offset(g.name),
                row,
                "offset for group {:?}",
                g.name
            );
            row += 1 + g.bindings.len();
        }
        assert_eq!(row, rendered_row_count(), "total rendered rows");
    }

    /// An unknown group name lands at the top rather than panicking.
    #[test]
    fn an_unknown_group_lands_at_the_top() {
        assert_eq!(group_row_offset("Nonexistent"), 0);
    }
}
