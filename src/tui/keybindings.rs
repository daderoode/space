/// Shared keybinding definitions consumed by the help overlay and status bar.

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
            desc: "Collapse / focus workspaces",
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
            key: "Enter/→",
            desc: "Expand / collapse repo",
        },
        Binding {
            key: "←/Esc",
            desc: "Collapse all / back",
        },
        Binding {
            key: "T",
            desc: "Toggle diff target",
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
    static GROUPS: [BindingGroup; 4] = [NAVIGATION, WORKSPACE_PANE, REPO_PANE, GENERAL];
    &GROUPS
}
