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
            desc: "Refresh repos",
        },
        Binding {
            key: "/",
            desc: "Search repos",
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
            key: "T",
            desc: "Toggle diff target",
        },
        Binding {
            key: "s",
            desc: "Stage / unstage file",
        },
        Binding {
            key: "S",
            desc: "Stage all unstaged",
        },
        Binding {
            key: "U",
            desc: "Unstage all staged",
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
            key: "s",
            desc: "Stage / unstage",
        },
        Binding {
            key: "Esc/q",
            desc: "Close",
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
    static GROUPS: [BindingGroup; 5] =
        [NAVIGATION, WORKSPACE_PANE, REPO_PANE, DIFF_VIEWER, GENERAL];
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
            desc: "refresh",
        },
        Binding {
            key: "/",
            desc: "search",
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
    static RIGHT: [Binding; 8] = [
        Binding {
            key: "enter",
            desc: "expand/diff",
        },
        Binding {
            key: "←/esc",
            desc: "back",
        },
        Binding {
            key: "T",
            desc: "toggle diff",
        },
        Binding {
            key: "s",
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
