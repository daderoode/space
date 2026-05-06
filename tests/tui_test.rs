mod common;

use common::{key, shift_key, test_app, test_app_with_config, TestEnv};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyCode;
use ratatui::Terminal;
use space::core::config::{RepoConfig, SpaceConfig, WorkspaceConfig};
use space::core::git::RepoStatus;
use space::core::workspace::{Workspace, WorkspaceRepo};
use space::tui::app::{App, Pane, Screen};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// Build a SpaceConfig pointing at a TestEnv's directories.
fn config_from_env(env: &TestEnv) -> SpaceConfig {
    SpaceConfig {
        repos: RepoConfig {
            roots: vec![env.repos_dir.clone()],
            max_depth: 3,
            cache_age_secs: 3600,
        },
        workspaces: WorkspaceConfig {
            dir: env.workspaces_dir.clone(),
        },
    }
}

fn render_text(app: &App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| space::tui::ui::view(app, frame))
        .unwrap();

    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();

    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }

    lines.join("\n")
}

fn max_rendered_width(rendered: &str) -> usize {
    rendered
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Dashboard navigation tests
// ---------------------------------------------------------------------------

#[test]
fn dashboard_q_quits() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit);
}

#[test]
fn dashboard_esc_quits() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit);
}

#[test]
fn dashboard_tab_toggles_focus() {
    let mut app = test_app(vec![], vec![]);
    assert_eq!(app.focus, Pane::Left);

    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Right);

    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Pane::Left);
}

#[test]
fn right_arrow_focuses_repo_pane() {
    let mut app = test_app(vec![], vec![]);
    assert_eq!(app.focus, Pane::Left);
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.focus, Pane::Right);
}

#[test]
fn left_arrow_refocuses_workspace_pane_when_nothing_expanded() {
    let mut app = test_app(vec![], vec![]);
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.focus, Pane::Left);
}

#[test]
fn left_arrow_noop_when_already_on_left_pane() {
    let mut app = test_app(vec![], vec![]);
    assert_eq!(app.focus, Pane::Left);
    app.handle_key(key(KeyCode::Left));
    assert_eq!(app.focus, Pane::Left);
}

#[test]
fn right_arrow_on_repo_pane_triggers_expand() {
    // Right arrow on repos pane dispatches ToggleRepoExpand.
    // With no workspace/repos, this is effectively a no-op but must not panic.
    let mut app = test_app(vec![], vec![]);
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Right)); // should not panic
}

#[test]
fn dashboard_j_moves_ws_down() {
    let workspaces = vec![
        Workspace {
            name: "alpha".to_string(),
            path: PathBuf::from("/tmp/alpha"),
            repos: vec![],
        },
        Workspace {
            name: "beta".to_string(),
            path: PathBuf::from("/tmp/beta"),
            repos: vec![],
        },
    ];
    let mut app = test_app(workspaces, vec![]);
    assert_eq!(app.selected_ws, 0);

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.selected_ws, 1);
}

#[test]
fn dashboard_k_moves_ws_up() {
    let workspaces = vec![
        Workspace {
            name: "alpha".to_string(),
            path: PathBuf::from("/tmp/alpha"),
            repos: vec![],
        },
        Workspace {
            name: "beta".to_string(),
            path: PathBuf::from("/tmp/beta"),
            repos: vec![],
        },
    ];
    let mut app = test_app(workspaces, vec![]);
    app.selected_ws = 1;

    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(app.selected_ws, 0);
}

#[test]
fn dashboard_enter_sets_cd_target() {
    let workspaces = vec![Workspace {
        name: "alpha".to_string(),
        path: PathBuf::from("/tmp/alpha"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);

    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.space_cd_target, Some(PathBuf::from("/tmp/alpha")));
    assert!(app.should_quit);
}

#[test]
fn dashboard_c_opens_create() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    assert!(
        matches!(app.screen, Screen::CreateWorkspace(_)),
        "expected CreateWorkspace screen, got {:?}",
        std::mem::discriminant(&app.screen)
    );
}

#[test]
fn dashboard_a_opens_add() {
    let repo_path = PathBuf::from("/tmp/repos/my-repo");
    let workspaces = vec![Workspace {
        name: "alpha".to_string(),
        path: PathBuf::from("/tmp/alpha"),
        repos: vec![WorkspaceRepo {
            name: "existing-repo".to_string(),
            path: PathBuf::from("/tmp/alpha/existing-repo"),
            branch: "main".to_string(),
            status: Default::default(),
            ahead: 0,
            behind: 0,
        }],
    }];
    let mut app = test_app(workspaces, vec![repo_path]);

    app.handle_key(key(KeyCode::Char('a')));
    assert!(
        matches!(app.screen, Screen::AddRepos(_)),
        "expected AddRepos screen, got {:?}",
        std::mem::discriminant(&app.screen)
    );
}

// ---------------------------------------------------------------------------
// Create flow tests
// ---------------------------------------------------------------------------

#[test]
fn create_name_to_strategy() {
    let mut app = test_app(vec![], vec![]);
    // Open create screen and advance to NameWorkspace stage
    app.handle_key(key(KeyCode::Char('c')));

    // Manually advance to NameWorkspace stage with a selected repo
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::create::CreateStage::NameWorkspace;
    }

    // Type a workspace name
    app.handle_key(key(KeyCode::Char('m')));
    app.handle_key(key(KeyCode::Char('y')));
    app.handle_key(key(KeyCode::Char('-')));
    app.handle_key(key(KeyCode::Char('w')));
    app.handle_key(key(KeyCode::Char('s')));

    // Press Enter to advance to PickBranchStrategy
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_strategy_new_branch_creates() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("my-repo");

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    // Open create screen and skip to PickBranchStrategy
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("test-ws".to_string());
        st.branch_strategy_idx = 0; // NewBranch
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
    }

    // Press Enter to trigger do_create
    app.handle_key(key(KeyCode::Enter));

    // Workspace dir should have been created, and screen should be Dashboard
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard screen after create, got {:?}",
        std::mem::discriminant(&app.screen)
    );
    assert!(env.workspaces_dir.join("test-ws").join("my-repo").exists());
}

#[test]
fn create_esc_from_name_goes_back() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    // Advance to NameWorkspace
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::create::CreateStage::NameWorkspace;
    }

    // Press Esc to go back to PickRepos
    app.handle_key(key(KeyCode::Esc));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickRepos
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_empty_name_rejected() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    // Advance to NameWorkspace
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::create::CreateStage::NameWorkspace;
    }

    // Press Enter with empty name
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::NameWorkspace,
            "stage should stay NameWorkspace when name is empty"
        );
        assert!(st.error.is_some(), "error should be set for empty name");
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_populates_recent_branches_on_strategy_entry() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("branchy-repo");

    // Create a second branch with a commit that has a guaranteed later timestamp
    let out = std::process::Command::new("git")
        .args(["checkout", "-b", "feature-recent"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "feature commit"])
        .env("GIT_COMMITTER_DATE", "2099-01-01T00:00:00Z")
        .env("GIT_AUTHOR_DATE", "2099-01-01T00:00:00Z")
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    // Go back to main
    let out = std::process::Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.stage = space::tui::screens::create::CreateStage::NameWorkspace;
        st.ws_name = tui_input::Input::default().with_value("test-ws".to_string());
    }

    // Press Enter to advance to PickBranchStrategy — should populate recent_branches
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy
        );
        assert!(
            !st.recent_branches.is_empty(),
            "recent_branches should be populated, got empty"
        );
        // Should be sorted by commit time descending (feature-recent committed later)
        assert_eq!(
            st.recent_branches[0].name, "feature-recent",
            "most recent branch should be first"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

// ---------------------------------------------------------------------------
// Add flow tests
// ---------------------------------------------------------------------------

#[test]
fn add_strategy_creates_worktrees() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("add-repo");

    // Create the workspace directory first (add expects it to exist)
    let ws_name = "existing-ws";
    std::fs::create_dir_all(env.workspaces_dir.join(ws_name)).unwrap();

    let config = config_from_env(&env);
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: env.workspaces_dir.join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app_with_config(config, workspaces, vec![repo_path.clone()]);

    // Open add screen and skip to PickBranchStrategy
    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.branch_strategy_idx = 0; // NewBranch
        st.stage = space::tui::screens::add::AddStage::PickBranchStrategy;
    }

    // Press Enter to trigger do_add
    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard screen after add, got {:?}",
        std::mem::discriminant(&app.screen)
    );
    assert!(
        env.workspaces_dir.join(ws_name).join("add-repo").exists(),
        "worktree should have been created"
    );
}

#[test]
fn add_esc_returns_to_dashboard() {
    let repo_path = PathBuf::from("/tmp/repos/my-repo");
    let workspaces = vec![Workspace {
        name: "alpha".to_string(),
        path: PathBuf::from("/tmp/alpha"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![repo_path]);

    // Open add screen
    app.handle_key(key(KeyCode::Char('a')));
    assert!(matches!(app.screen, Screen::AddRepos(_)));

    // Press Esc from PickRepos -> Dashboard
    app.handle_key(key(KeyCode::Esc));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after Esc from AddRepos"
    );
}

// ---------------------------------------------------------------------------
// Delete handler tests
// ---------------------------------------------------------------------------

#[test]
fn delete_y_removes_workspace() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("del-repo");

    // Create a real workspace with a worktree
    space::core::workspace::create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "doomed-ws",
        &space::core::workspace::BranchStrategy::NewBranch("doomed-ws".to_string()),
    )
    .unwrap();

    let config = config_from_env(&env);
    let workspaces = vec![Workspace {
        name: "doomed-ws".to_string(),
        path: env.workspaces_dir.join("doomed-ws"),
        repos: vec![WorkspaceRepo {
            name: "del-repo".to_string(),
            path: env.workspaces_dir.join("doomed-ws").join("del-repo"),
            branch: "doomed-ws".to_string(),
            status: Default::default(),
            ahead: 0,
            behind: 0,
        }],
    }];
    let mut app = test_app_with_config(config, workspaces, vec![repo_path]);

    // Press 'd' to open delete confirmation
    app.handle_key(key(KeyCode::Char('d')));
    assert!(matches!(app.screen, Screen::ConfirmDelete(_)));

    // Press 'y' to confirm
    app.handle_key(key(KeyCode::Char('y')));
    assert!(matches!(app.screen, Screen::Dashboard));
    assert!(
        !env.workspaces_dir.join("doomed-ws").exists(),
        "workspace should be deleted"
    );
}

#[test]
fn delete_n_cancels() {
    let workspaces = vec![Workspace {
        name: "keep-me".to_string(),
        path: PathBuf::from("/tmp/keep-me"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);

    app.handle_key(key(KeyCode::Char('d')));
    assert!(matches!(app.screen, Screen::ConfirmDelete(_)));

    app.handle_key(key(KeyCode::Char('n')));
    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(
        app.workspaces.len(),
        1,
        "workspace list should be preserved"
    );
}

#[test]
fn dashboard_d_no_workspaces_stays_on_dashboard() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('d')));
    // No workspace selected -> StartDelete produces no ConfirmDelete
    assert!(matches!(app.screen, Screen::Dashboard));
}

#[test]
fn dashboard_status_uses_plain_language_summary() {
    let workspaces = vec![Workspace {
        name: "env-reset-redesign".to_string(),
        path: PathBuf::from("/tmp/env-reset-redesign"),
        repos: vec![WorkspaceRepo {
            name: "omari-zw-vsuite-env".to_string(),
            path: PathBuf::from("/tmp/env-reset-redesign/omari-zw-vsuite-env"),
            branch: "env-reset-redesign".to_string(),
            status: RepoStatus {
                modified: 14,
                staged: 0,
                untracked: 3,
                conflicted: 0,
            },
            ahead: 0,
            behind: 0,
        }],
    }];
    let app = test_app(workspaces, vec![]);

    let rendered = render_text(&app, 160, 12);

    assert!(
        rendered.contains("14 modified, 3 new"),
        "expected plain-language status in rendered dashboard, got:\n{}",
        rendered
    );
}

#[test]
fn dashboard_status_fits_common_80_col_terminal() {
    let workspaces = vec![Workspace {
        name: "env-reset-redesign".to_string(),
        path: PathBuf::from("/tmp/env-reset-redesign"),
        repos: vec![WorkspaceRepo {
            name: "omari-zw-vsuite-env".to_string(),
            path: PathBuf::from("/tmp/env-reset-redesign/omari-zw-vsuite-env"),
            branch: "env-reset-redesign".to_string(),
            status: RepoStatus {
                modified: 14,
                staged: 0,
                untracked: 3,
                conflicted: 0,
            },
            ahead: 0,
            behind: 0,
        }],
    }];
    let app = test_app(workspaces, vec![]);

    let rendered = render_text(&app, 80, 12);

    assert!(
        rendered.contains("14 modified, 3 new"),
        "expected status summary to stay readable on an 80-column terminal, got:\n{}",
        rendered
    );
}

#[test]
fn dashboard_three_part_status_fits_common_80_col_terminal() {
    let workspaces = vec![Workspace {
        name: "env-reset-redesign".to_string(),
        path: PathBuf::from("/tmp/env-reset-redesign"),
        repos: vec![WorkspaceRepo {
            name: "omari-zw-vsuite-env".to_string(),
            path: PathBuf::from("/tmp/env-reset-redesign/omari-zw-vsuite-env"),
            branch: "env-reset-redesign".to_string(),
            status: RepoStatus {
                modified: 14,
                staged: 3,
                untracked: 2,
                conflicted: 0,
            },
            ahead: 0,
            behind: 0,
        }],
    }];
    let app = test_app(workspaces, vec![]);

    let rendered = render_text(&app, 80, 12);

    assert!(
        rendered.contains("17 changed, 2 new"),
        "expected compact plain-language status in an 80-column terminal, got:\n{}",
        rendered
    );
}

#[test]
fn delete_confirm_handles_long_workspace_name() {
    let workspaces = vec![Workspace {
        name: "I079696-omari-bundles-failure-recovery".to_string(),
        path: PathBuf::from("/tmp/I079696-omari-bundles-failure-recovery"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);
    app.screen = Screen::ConfirmDelete(space::tui::screens::delete::DeleteState {
        workspace_name: "I079696-omari-bundles-failure-recovery".to_string(),
        repo_names: vec![
            "omari-zw-vsuite-env".to_string(),
            "vgateway-discovery".to_string(),
        ],
    });

    let rendered = render_text(&app, 68, 12);

    assert!(
        rendered.contains("Delete workspace?"),
        "expected clearer delete heading in rendered popup, got:\n{}",
        rendered
    );
}

#[test]
fn delete_confirm_keeps_footer_visible_for_long_repo_lists() {
    let workspaces = vec![Workspace {
        name: "too-many-repos".to_string(),
        path: PathBuf::from("/tmp/too-many-repos"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);
    app.screen = Screen::ConfirmDelete(space::tui::screens::delete::DeleteState {
        workspace_name: "too-many-repos".to_string(),
        repo_names: (1..=12).map(|idx| format!("repo-{}", idx)).collect(),
    });

    let rendered = render_text(&app, 70, 10);

    assert!(
        rendered.contains("Enter/y delete"),
        "expected delete actions to remain visible in rendered popup, got:\n{}",
        rendered
    );
    assert!(
        rendered.contains("Esc/n cancel"),
        "expected cancel action to remain visible in rendered popup, got:\n{}",
        rendered
    );
    assert!(
        rendered.contains("... and"),
        "expected repo overflow summary in rendered popup, got:\n{}",
        rendered
    );
}

#[test]
fn delete_confirm_wraps_footer_on_narrow_terminals() {
    let workspaces = vec![Workspace {
        name: "tight-layout".to_string(),
        path: PathBuf::from("/tmp/tight-layout"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);
    app.screen = Screen::ConfirmDelete(space::tui::screens::delete::DeleteState {
        workspace_name: "tight-layout".to_string(),
        repo_names: vec!["repo-with-a-very-long-name".to_string()],
    });

    let rendered = render_text(&app, 24, 10);

    assert!(
        rendered.contains("Enter/y delete"),
        "expected delete action on a wrapped footer line, got:\n{}",
        rendered
    );
    assert!(
        rendered.contains("Esc/n cancel"),
        "expected cancel action on a wrapped footer line, got:\n{}",
        rendered
    );
    assert!(
        max_rendered_width(&rendered) <= 24,
        "expected wrapped delete footer to stay within the terminal width, got:\n{}",
        rendered
    );
}

// ---------------------------------------------------------------------------
// Go handler tests
// ---------------------------------------------------------------------------

#[test]
fn go_esc_returns_to_dashboard() {
    let workspaces = vec![Workspace {
        name: "alpha".to_string(),
        path: PathBuf::from("/tmp/alpha"),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![]);

    app.handle_key(key(KeyCode::Char('g')));
    assert!(matches!(app.screen, Screen::GoWorkspace(_)));

    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));
}

// ---------------------------------------------------------------------------
// Search handler tests
// ---------------------------------------------------------------------------

#[test]
fn search_esc_returns_to_dashboard() {
    let mut app = test_app(vec![], vec![PathBuf::from("/tmp/repos/foo")]);

    app.handle_key(key(KeyCode::Char('/')));
    assert!(matches!(app.screen, Screen::RepoSearch(_)));

    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));
}

// ---------------------------------------------------------------------------
// Branch strategy navigation with recent branches
// ---------------------------------------------------------------------------

#[test]
fn create_select_recent_branch_creates_worktree() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("branch-select-repo");

    // Create a feature branch
    let out = std::process::Command::new("git")
        .args(["branch", "feature-pick-me"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("ws-branch".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
        st.recent_branches = vec![space::core::git::BranchInfo {
            name: "feature-pick-me".to_string(),
            is_remote: false,
            is_current: false,
            last_commit_time: 1000,
        }];
        st.branch_strategy_idx = 3; // First recent branch
    }

    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after selecting recent branch, got {:?}",
        std::mem::discriminant(&app.screen)
    );
    assert!(env
        .workspaces_dir
        .join("ws-branch")
        .join("branch-select-repo")
        .exists());
}

#[test]
fn create_branch_strategy_navigation_with_recent_branches() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
        st.recent_branches = vec![
            space::core::git::BranchInfo {
                name: "branch-a".to_string(),
                is_remote: false,
                is_current: false,
                last_commit_time: 2000,
            },
            space::core::git::BranchInfo {
                name: "branch-b".to_string(),
                is_remote: false,
                is_current: false,
                last_commit_time: 1000,
            },
        ];
        st.branch_strategy_idx = 0;
    }

    // Navigate down through all items: 0,1,2,3,4,5 (3 fixed + 2 branches + show more)
    for _ in 0..10 {
        app.handle_key(key(KeyCode::Down));
    }
    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.branch_strategy_idx, 5,
            "should clamp at max_idx (3 + 2 branches)"
        );
    } else {
        panic!("expected CreateWorkspace");
    }

    // Navigate back up past 0
    for _ in 0..10 {
        app.handle_key(key(KeyCode::Up));
    }
    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(st.branch_strategy_idx, 0, "should clamp at 0");
    } else {
        panic!("expected CreateWorkspace");
    }
}

#[test]
fn add_select_recent_branch_creates_worktree() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("add-branch-repo");

    // Create a feature branch
    let out = std::process::Command::new("git")
        .args(["branch", "feature-add-pick"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    // Create the workspace directory (add expects it to exist)
    let ws_name = "add-ws";
    std::fs::create_dir_all(env.workspaces_dir.join(ws_name)).unwrap();

    let config = config_from_env(&env);
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: env.workspaces_dir.join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app_with_config(config, workspaces, vec![repo_path.clone()]);

    // Open add screen and skip to PickBranchStrategy
    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.stage = space::tui::screens::add::AddStage::PickBranchStrategy;
        st.recent_branches = vec![space::core::git::BranchInfo {
            name: "feature-add-pick".to_string(),
            is_remote: false,
            is_current: false,
            last_commit_time: 1000,
        }];
        st.branch_strategy_idx = 3; // First recent branch
    }

    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after selecting recent branch in add flow"
    );
    assert!(env
        .workspaces_dir
        .join(ws_name)
        .join("add-branch-repo")
        .exists());
}

#[test]
fn create_show_more_preserves_idx_and_esc_returns_to_show_more() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("picker-repo");

    let out = std::process::Command::new("git")
        .args(["branch", "feature-picker"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("ws-picker".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
        st.recent_branches = vec![space::core::git::BranchInfo {
            name: "some-branch".to_string(),
            is_remote: false,
            is_current: false,
            last_commit_time: 1000,
        }];
        st.branch_strategy_idx = 4; // "Show more..." (3 + 1 recent branch)
    }

    // Enter opens the fuzzy picker
    app.handle_key(key(KeyCode::Enter));
    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranch,
            "should transition to PickBranch"
        );
        // idx should stay at max_idx (4) so Esc returns to "Show more..."
        assert_eq!(
            st.branch_strategy_idx, 4,
            "branch_strategy_idx should stay at max_idx for Show more position"
        );
    } else {
        panic!("expected CreateWorkspace screen after Enter");
    }

    // Esc from picker returns to PickBranchStrategy with cursor on "Show more..."
    app.handle_key(key(KeyCode::Esc));
    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy,
            "Esc should return to PickBranchStrategy"
        );
        assert_eq!(
            st.branch_strategy_idx, 4,
            "cursor should be back on Show more after Esc"
        );
    } else {
        panic!("expected CreateWorkspace screen after Esc");
    }
}

#[test]
fn create_reentry_resets_branch_strategy_idx() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("reentry-repo");

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.stage = space::tui::screens::create::CreateStage::NameWorkspace;
        st.ws_name = tui_input::Input::default().with_value("ws-reentry".to_string());
        // Simulate stale idx from a previous PickBranchStrategy visit
        st.branch_strategy_idx = 7;
    }

    // Press Enter to advance to PickBranchStrategy
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy,
        );
        let max_valid = 3 + st.recent_branches.len();
        assert!(
            st.branch_strategy_idx <= max_valid,
            "branch_strategy_idx ({}) exceeds valid range (0..={}), would panic on Enter",
            st.branch_strategy_idx,
            max_valid,
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

// ---------------------------------------------------------------------------
// flattened_rows + cursor navigation (Task 3)
// ---------------------------------------------------------------------------

#[test]
fn flattened_rows_all_collapsed() {
    let ws = common::workspace_with_repos(&["repo-a", "repo-b", "repo-c"]);
    let app = test_app(vec![ws], vec![]);
    let rows = app.flattened_rows();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| matches!(
        r,
        space::tui::app::RepoRow::Repo {
            expanded: false,
            ..
        }
    )));
}

#[test]
fn flattened_rows_one_expanded_with_files() {
    use space::core::git::{FileEntry, FileStatus};
    let ws = common::workspace_with_repos(&["repo-a", "repo-b"]);
    let mut app = test_app(vec![ws], vec![]);
    app.expanded_repos.insert(1);
    app.repo_file_cache.insert(
        1,
        vec![
            FileEntry {
                path: "foo.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 3,
                deletions: 1,
            },
            FileEntry {
                path: "bar.rs".into(),
                status: FileStatus::Added,
                staged: true,
                insertions: 10,
                deletions: 0,
            },
        ],
    );
    let rows = app.flattened_rows();
    // repo-a, repo-b, SectionHeader("Unstaged"), foo.rs, SectionHeader("Staged"), bar.rs
    assert_eq!(rows.len(), 6);
    assert!(matches!(
        rows[0],
        space::tui::app::RepoRow::Repo {
            index: 0,
            expanded: false,
            ..
        }
    ));
    assert!(matches!(
        rows[1],
        space::tui::app::RepoRow::Repo {
            index: 1,
            expanded: true,
            ..
        }
    ));
    assert!(matches!(
        rows[2],
        space::tui::app::RepoRow::SectionHeader {
            repo_index: 1,
            label: "Unstaged",
            ..
        }
    ));
    assert!(matches!(
        rows[3],
        space::tui::app::RepoRow::File { repo_index: 1, .. }
    ));
    assert!(matches!(
        rows[4],
        space::tui::app::RepoRow::SectionHeader {
            repo_index: 1,
            label: "Staged",
            ..
        }
    ));
    assert!(matches!(
        rows[5],
        space::tui::app::RepoRow::File { repo_index: 1, .. }
    ));
}

#[test]
fn repo_index_for_cursor_on_file_row() {
    use space::core::git::{FileEntry, FileStatus};
    let ws = common::workspace_with_repos(&["repo-a", "repo-b"]);
    let mut app = test_app(vec![ws], vec![]);
    app.expanded_repos.insert(1);
    app.repo_file_cache.insert(
        1,
        vec![FileEntry {
            path: "x.rs".into(),
            status: FileStatus::Modified,
            staged: false,
            insertions: 1,
            deletions: 0,
        }],
    );
    // repo-a=0, repo-b=1, SectionHeader("Unstaged")=2, x.rs=3
    app.cursor_row = 3;
    assert_eq!(app.repo_index_for_cursor(), Some(1));
}

#[test]
fn cursor_row_navigates_through_file_rows() {
    use space::core::git::{FileEntry, FileStatus};
    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.expanded_repos.insert(0);
    app.repo_file_cache.insert(
        0,
        vec![
            FileEntry {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "b.rs".into(),
                status: FileStatus::Modified,
                staged: true,
                insertions: 2,
                deletions: 0,
            },
        ],
    );
    // rows: [Repo(0), SectionHeader("Unstaged")(1), File(a.rs)(2), SectionHeader("Staged")(3), File(b.rs)(4)]
    assert_eq!(app.cursor_row, 0);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 1); // SectionHeader("Unstaged")
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 2); // a.rs
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 3); // SectionHeader("Staged")
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 4); // b.rs
    app.handle_key(key(KeyCode::Down)); // clamp at end
    assert_eq!(app.cursor_row, 4);
}

#[test]
fn section_header_rows_are_non_interactive() {
    use space::core::git::{FileEntry, FileStatus};

    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.expanded_repos.insert(0);
    app.repo_file_cache.insert(
        0,
        vec![FileEntry {
            path: "x.rs".into(),
            status: FileStatus::Modified,
            staged: false,
            insertions: 1,
            deletions: 0,
        }],
    );
    // rows: [Repo(0), SectionHeader("Unstaged")(1), File(x.rs)(2)]
    // Navigate to the section header row
    app.cursor_row = 1;
    assert!(
        matches!(
            app.flattened_rows().get(1),
            Some(space::tui::app::RepoRow::SectionHeader { .. })
        ),
        "cursor should be on SectionHeader row"
    );

    // s on a SectionHeader should be a no-op (no staging)
    let files_before = app.repo_file_cache.get(&0).unwrap().clone();
    app.handle_key(key(KeyCode::Char('s')));
    let files_after = app.repo_file_cache.get(&0).unwrap();
    assert_eq!(
        files_before[0].staged, files_after[0].staged,
        "s on SectionHeader should not change staging state"
    );

    // Enter on a SectionHeader should be a no-op (no diff viewer)
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "Enter on SectionHeader should stay on Dashboard"
    );
}

#[test]
fn cursor_row_resets_on_workspace_switch() {
    let ws1 = common::workspace_with_repos(&["repo-a"]);
    let ws2 = common::workspace_with_repos(&["repo-b"]);
    let mut app = test_app(vec![ws1, ws2], vec![]);
    app.cursor_row = 5;
    app.focus = Pane::Left;
    app.handle_key(key(KeyCode::Down)); // select workspace 2
    assert_eq!(app.cursor_row, 0, "cursor should reset on workspace switch");
}

// ---------------------------------------------------------------------------
// Expand/collapse key handling (Task 4)
// ---------------------------------------------------------------------------

#[test]
fn enter_on_repo_row_expands_it() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("expandable");
    // Write an untracked file so file_diff has something to find
    std::fs::write(repo_path.join("x.txt"), "change").unwrap();

    let ws_path = env.workspaces_dir.join("my-ws");
    std::fs::create_dir_all(&ws_path).unwrap();
    let ws = space::core::workspace::Workspace {
        name: "my-ws".into(),
        path: ws_path,
        repos: vec![space::core::workspace::WorkspaceRepo {
            name: "expandable".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: space::core::git::RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![]);
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter));
    assert!(
        app.expanded_repos.contains(&0),
        "repo 0 should be expanded after Enter"
    );
    assert!(
        app.repo_file_cache.contains_key(&0),
        "cache should be populated"
    );
}

#[test]
fn enter_on_expanded_repo_collapses_it() {
    use space::core::git::{FileEntry, FileStatus};
    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    // Pre-expand manually
    app.expanded_repos.insert(0);
    app.repo_file_cache.insert(
        0,
        vec![FileEntry {
            path: "x.rs".into(),
            status: FileStatus::Modified,
            staged: false,
            insertions: 1,
            deletions: 0,
        }],
    );
    app.cursor_row = 0;
    app.handle_key(key(KeyCode::Enter));
    assert!(
        !app.expanded_repos.contains(&0),
        "should collapse on second Enter"
    );
    assert_eq!(app.cursor_row, 0, "cursor should snap to repo row");
}

#[test]
fn esc_collapses_expanded_repos_without_refocusing() {
    let ws = common::workspace_with_repos(&["repo-a", "repo-b"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.expanded_repos.insert(0);
    app.expanded_repos.insert(1);
    app.handle_key(key(KeyCode::Esc));
    assert!(
        app.expanded_repos.is_empty(),
        "Esc should collapse all expanded repos"
    );
    assert_eq!(
        app.focus,
        Pane::Right,
        "focus stays on right pane after collapse"
    );
}

#[test]
fn esc_with_nothing_expanded_refocuses_left_pane() {
    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(
        app.focus,
        Pane::Left,
        "Esc with nothing expanded should refocus left pane"
    );
}

#[test]
fn esc_on_left_pane_quits() {
    let mut app = test_app(vec![], vec![]);
    assert_eq!(app.focus, Pane::Left);
    app.handle_key(key(KeyCode::Esc));
    assert!(app.should_quit, "Esc on left pane should quit");
}

#[test]
fn workspace_switch_clears_expand_state() {
    let ws1 = common::workspace_with_repos(&["repo-a"]);
    let ws2 = common::workspace_with_repos(&["repo-b"]);
    let mut app = test_app(vec![ws1, ws2], vec![]);
    app.expanded_repos.insert(0);
    app.focus = Pane::Left;
    app.handle_key(key(KeyCode::Down)); // select workspace 2
    assert!(
        app.expanded_repos.is_empty(),
        "workspace switch should clear expanded state"
    );
    assert_eq!(app.cursor_row, 0);
}

// ---------------------------------------------------------------------------
// Integration smoke tests for Phase 1 feature (Task manual test equivalent)
// ---------------------------------------------------------------------------

#[test]
fn phase1_expand_dirty_repo_shows_file_entries() {
    // Tests the full stack: real git repo with staged/unstaged changes,
    // WorkspaceRepo pointing at it, expand via Enter, verify file entries appear.
    use space::core::git::{FileStatus, RepoStatus};
    use space::core::workspace::{Workspace, WorkspaceRepo};

    let env = TestEnv::new();
    let repo_path = env.create_repo("dirty-repo");

    // Commit existing.rs first so it becomes a tracked file
    std::fs::write(repo_path.join("existing.rs"), "line1\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "existing.rs"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "-m", "add existing"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // NOW stage a new file (after the commit, so it won't be swept in)
    std::fs::write(
        repo_path.join("staged.rs"),
        "pub fn hello() {}\npub fn world() {}\n",
    )
    .unwrap();
    std::process::Command::new("git")
        .args(["add", "staged.rs"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Unstaged modification to the committed file
    std::fs::write(repo_path.join("existing.rs"), "line1\nline2\nline3\n").unwrap();

    // Untracked file
    std::fs::write(repo_path.join("untracked.txt"), "untracked\n").unwrap();

    let ws = Workspace {
        name: "test-ws".into(),
        path: env.workspaces_dir.join("test-ws"),
        repos: vec![WorkspaceRepo {
            name: "dirty-repo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![]);
    app.focus = Pane::Right;

    // Pre-expand: should have 1 repo row (collapsed)
    assert_eq!(app.flattened_rows().len(), 1);

    // Expand with Enter
    app.handle_key(key(KeyCode::Enter));

    // Should now have > 1 row (repo + file entries)
    let rows = app.flattened_rows();
    assert!(
        rows.len() > 1,
        "expanded repo should show file rows, got {} rows",
        rows.len()
    );

    // First row is still the repo header, now expanded
    assert!(matches!(
        rows[0],
        space::tui::app::RepoRow::Repo { expanded: true, .. }
    ));

    // File rows should follow
    let file_rows: Vec<_> = rows
        .iter()
        .filter(|r| matches!(r, space::tui::app::RepoRow::File { .. }))
        .collect();
    assert!(
        !file_rows.is_empty(),
        "should have file rows after expansion"
    );

    // staged.rs should appear as staged=true, Added
    let staged_entry = app
        .repo_file_cache
        .get(&0)
        .unwrap()
        .iter()
        .find(|e| e.path == "staged.rs")
        .expect("staged.rs should be in cache");
    assert!(staged_entry.staged, "staged.rs should be staged=true");
    assert_eq!(staged_entry.status, FileStatus::Added);
    assert_eq!(staged_entry.insertions, 2, "staged.rs has 2 lines");

    // existing.rs should appear as unstaged modification
    let unstaged_entry = app
        .repo_file_cache
        .get(&0)
        .unwrap()
        .iter()
        .find(|e| e.path == "existing.rs")
        .expect("existing.rs should be in cache");
    assert!(
        !unstaged_entry.staged,
        "existing.rs modification is unstaged"
    );
    assert_eq!(unstaged_entry.status, FileStatus::Modified);
    assert!(unstaged_entry.insertions > 0, "existing.rs has new lines");
}

#[test]
fn phase1_collapse_snaps_cursor_to_repo_row() {
    use space::core::git::{FileEntry, FileStatus};

    let ws = common::workspace_with_repos(&["repo-a", "repo-b"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;

    // Expand repo-b (index 1), inject 3 fake file entries
    app.expanded_repos.insert(1);
    app.repo_file_cache.insert(
        1,
        vec![
            FileEntry {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "b.rs".into(),
                status: FileStatus::Added,
                staged: true,
                insertions: 5,
                deletions: 0,
            },
            FileEntry {
                path: "c.rs".into(),
                status: FileStatus::Deleted,
                staged: false,
                insertions: 0,
                deletions: 3,
            },
        ],
    );

    // Navigate cursor to repo-b's header row (index 1: repo-a=0, repo-b=1, then files)
    app.cursor_row = 1;
    assert_eq!(app.repo_index_for_cursor(), Some(1));

    // Collapse via Enter on the repo header row
    app.handle_key(key(KeyCode::Enter));

    // repo-b should be collapsed
    assert!(!app.expanded_repos.contains(&1));
    // cursor should snap to repo-b's row (index 1 in collapsed list: repo-a=0, repo-b=1)
    assert_eq!(
        app.cursor_row, 1,
        "cursor should snap to repo-b row after collapse"
    );
    assert_eq!(
        app.flattened_rows().len(),
        2,
        "only 2 rows after full collapse"
    );
}

#[test]
fn phase1_right_arrow_on_workspace_pane_then_enter_expands_repo() {
    // Full navigation flow: start on left pane, arrow right, expand with Enter
    use space::core::git::RepoStatus;
    use space::core::workspace::{Workspace, WorkspaceRepo};

    let env = TestEnv::new();
    let repo_path = env.create_repo("nav-repo");
    std::fs::write(repo_path.join("changed.rs"), "new line\n").unwrap();

    let ws = Workspace {
        name: "nav-ws".into(),
        path: env.workspaces_dir.join("nav-ws"),
        repos: vec![WorkspaceRepo {
            name: "nav-repo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![]);

    // Start on left (workspaces) pane
    assert_eq!(app.focus, Pane::Left);

    // Right arrow moves focus to repos pane
    app.handle_key(key(KeyCode::Right));
    assert_eq!(app.focus, Pane::Right);

    // Enter expands the repo
    app.handle_key(key(KeyCode::Enter));
    assert!(app.expanded_repos.contains(&0), "repo should expand");

    // Esc collapses (nothing expanded check: first Esc collapses, second refocuses)
    app.handle_key(key(KeyCode::Esc));
    assert!(app.expanded_repos.is_empty(), "Esc should collapse");
    assert_eq!(
        app.focus,
        Pane::Right,
        "focus still on right after collapse"
    );

    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, Pane::Left, "second Esc refocuses left pane");
}

// ---------------------------------------------------------------------------
// Help overlay tests
// ---------------------------------------------------------------------------

#[test]
fn dashboard_question_mark_opens_help() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    assert!(
        matches!(app.screen, Screen::Help),
        "expected Help screen, got {:?}",
        std::mem::discriminant(&app.screen)
    );
}

#[test]
fn help_esc_returns_to_dashboard() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    assert!(matches!(app.screen, Screen::Help));

    app.handle_key(key(KeyCode::Esc));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after Esc from Help"
    );
}

#[test]
fn help_q_returns_to_dashboard() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after q from Help"
    );
}

#[test]
fn help_question_mark_returns_to_dashboard() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Char('?')));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after ? toggles Help off"
    );
}

#[test]
fn help_other_keys_are_noop() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));

    // Press various keys that should not close help
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('k')));
    app.handle_key(key(KeyCode::Char('c')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Help),
        "Help should remain open after non-close keys"
    );
}

// ---------------------------------------------------------------------------
// Diff viewer + staging integration tests (Phase 2)
// ---------------------------------------------------------------------------

/// Helper: set up a TestEnv with a real repo containing a committed file and
/// an unstaged modification. Returns (env, repo_path, app) with diff_target=Head,
/// the repo expanded, and cursor on the file row.
fn setup_real_repo_app() -> (TestEnv, PathBuf, App) {
    let env = TestEnv::new();
    let repo_path = env.create_repo("testrepo");

    // Commit a file, then modify it to create an unstaged change
    std::fs::write(repo_path.join("file.txt"), "initial").unwrap();
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new("git")
        .args(["commit", "-m", "add file"])
        .current_dir(&repo_path)
        .output()
        .expect("git commit failed to run");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(repo_path.join("file.txt"), "modified").unwrap();

    let ws = Workspace {
        name: "test-ws".into(),
        path: env.workspaces_dir.clone(),
        repos: vec![WorkspaceRepo {
            name: "testrepo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![repo_path.clone()]);
    app.load_selected_workspace_detail();

    // Focus right pane and expand the repo
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter)); // expand repo at cursor_row=0

    // Navigate past the section header to the file row
    app.handle_key(key(KeyCode::Down)); // SectionHeader("Unstaged")
    app.handle_key(key(KeyCode::Down)); // File(file.txt)

    (env, repo_path, app)
}

#[test]
fn enter_on_file_row_opens_diff_viewer() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Cursor should be on a file row now; press Enter to open diff viewer
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen, got {:?}",
        std::mem::discriminant(&app.screen)
    );
}

#[test]
fn esc_in_diff_viewer_returns_to_dashboard() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    app.handle_key(key(KeyCode::Enter)); // open diff viewer
    assert!(matches!(app.screen, Screen::DiffViewer(_)));

    app.handle_key(key(KeyCode::Esc));
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after Esc from DiffViewer"
    );
}

#[test]
fn j_in_diff_viewer_increments_scroll() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    app.handle_key(key(KeyCode::Enter)); // open diff viewer
    if let Screen::DiffViewer(ref state) = app.screen {
        assert_eq!(state.scroll_offset, 0, "scroll should start at 0");
    } else {
        panic!("expected DiffViewer screen");
    }

    app.handle_key(key(KeyCode::Char('j')));
    if let Screen::DiffViewer(ref state) = app.screen {
        // If there are diff lines, scroll should have advanced
        if state.total_lines > 1 {
            assert!(
                state.scroll_offset > 0,
                "scroll_offset should increase after j"
            );
        }
    } else {
        panic!("expected DiffViewer screen after j");
    }
}

#[test]
fn k_at_top_does_not_underflow() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    app.handle_key(key(KeyCode::Enter)); // open diff viewer
    if let Screen::DiffViewer(ref state) = app.screen {
        assert_eq!(state.scroll_offset, 0, "should start at 0");
    } else {
        panic!("expected DiffViewer screen");
    }

    app.handle_key(key(KeyCode::Char('k')));
    if let Screen::DiffViewer(ref state) = app.screen {
        assert_eq!(
            state.scroll_offset, 0,
            "scroll_offset should remain 0 after k at top"
        );
    } else {
        panic!("expected DiffViewer screen after k");
    }
}

#[test]
fn s_on_file_row_in_head_mode_stages() {
    let (_env, repo_path, mut app) = setup_real_repo_app();

    // Verify file is unstaged before staging
    let files_before = app.repo_file_cache.get(&0).expect("should have cache");
    let file_entry = files_before
        .iter()
        .find(|e| e.path == "file.txt")
        .expect("file.txt should be in cache");
    assert!(
        !file_entry.staged,
        "file.txt should be unstaged before pressing s"
    );

    // Press s to stage the file
    app.handle_key(key(KeyCode::Char('s')));

    // After staging, the repo_file_cache should be refreshed
    let files_after = app.repo_file_cache.get(&0).expect("cache should exist");
    let file_entry_after = files_after
        .iter()
        .find(|e| e.path == "file.txt")
        .expect("file.txt should still be in cache");
    assert!(
        file_entry_after.staged,
        "file.txt should be staged after pressing s"
    );

    // Verify via git that the file is actually staged
    let status_out = std::process::Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&repo_path)
        .output()
        .expect("git diff --cached failed to run");
    let staged_files = String::from_utf8_lossy(&status_out.stdout);
    assert!(
        staged_files.contains("file.txt"),
        "file.txt should be staged in git"
    );
}

#[test]
fn shift_s_on_repo_row_stages_all_unstaged() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("bulk-stage-repo");

    // Create two committed files, then modify both
    for name in &["a.txt", "b.txt"] {
        std::fs::write(repo_path.join(name), "initial").unwrap();
    }
    let out = std::process::Command::new("git")
        .args(["add", "a.txt", "b.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new("git")
        .args(["commit", "-m", "add files"])
        .current_dir(&repo_path)
        .output()
        .expect("git commit failed to run");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    for name in &["a.txt", "b.txt"] {
        std::fs::write(repo_path.join(name), "modified").unwrap();
    }

    let ws = Workspace {
        name: "test-ws".into(),
        path: env.workspaces_dir.clone(),
        repos: vec![WorkspaceRepo {
            name: "bulk-stage-repo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![repo_path.clone()]);
    app.load_selected_workspace_detail();

    // Focus right pane and expand repo
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter));

    // Cursor is on repo row (row 0). Press Shift+S to stage all
    assert_eq!(app.cursor_row, 0);
    app.handle_key(shift_key(KeyCode::Char('S')));

    // All files should now be staged
    let files = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files.iter().all(|e| e.staged),
        "all files should be staged after S: {:?}",
        files
            .iter()
            .map(|e| (&e.path, e.staged))
            .collect::<Vec<_>>()
    );
}

#[test]
fn shift_u_on_repo_row_unstages_all_staged() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("bulk-unstage-repo");

    // Create a file, commit it, modify it, then stage the modification
    std::fs::write(repo_path.join("file.txt"), "initial").unwrap();
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new("git")
        .args(["commit", "-m", "add file"])
        .current_dir(&repo_path)
        .output()
        .expect("git commit failed to run");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(repo_path.join("file.txt"), "modified").unwrap();
    // Stage the modification
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ws = Workspace {
        name: "test-ws".into(),
        path: env.workspaces_dir.clone(),
        repos: vec![WorkspaceRepo {
            name: "bulk-unstage-repo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![repo_path.clone()]);
    app.load_selected_workspace_detail();

    // Verify file is staged before we unstage
    let files_before = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files_before.iter().any(|e| e.staged),
        "should have at least one staged file before U"
    );

    // Focus right pane and expand repo
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter));

    // Cursor is on repo row (row 0). Press U to unstage all
    assert_eq!(app.cursor_row, 0);
    app.handle_key(shift_key(KeyCode::Char('U')));

    // All files should now be unstaged
    let files = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files.iter().all(|e| !e.staged),
        "all files should be unstaged after U: {:?}",
        files
            .iter()
            .map(|e| (&e.path, e.staged))
            .collect::<Vec<_>>()
    );
}

#[test]
fn staging_invalidates_diff_content_cache() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Open diff viewer to populate the diff_content_cache
    app.handle_key(key(KeyCode::Enter)); // open diff viewer
    assert!(matches!(app.screen, Screen::DiffViewer(_)));

    // Go back to dashboard
    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));

    // Verify diff_content_cache has entries for repo 0
    assert!(
        app.diff_content_cache.keys().any(|k| k.repo_index == 0),
        "diff_content_cache should have entries for repo 0 after viewing diff"
    );

    // Navigate back to the file row and stage it
    app.handle_key(key(KeyCode::Down)); // move to file row
    app.handle_key(key(KeyCode::Char('s'))); // stage the file

    // After staging, diff_content_cache entries for repo 0 should be gone
    assert!(
        !app.diff_content_cache.keys().any(|k| k.repo_index == 0),
        "diff_content_cache entries for repo 0 should be invalidated after staging"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 – diff viewer staging, scrolling, caching, error handling
// ---------------------------------------------------------------------------

#[test]
fn s_in_diff_viewer_stages_file_and_returns_to_dashboard() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Verify file.txt is unstaged
    let files = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files
            .iter()
            .any(|e| !e.staged && e.path.contains("file.txt")),
        "file.txt should be unstaged initially"
    );

    // Open diff viewer
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen after Enter"
    );

    // Press 's' to stage from inside the viewer
    app.handle_key(key(KeyCode::Char('s')));

    // Should return to dashboard
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after staging from DiffViewer"
    );

    // file.txt should now be staged
    let files_after = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files_after
            .iter()
            .any(|e| e.staged && e.path.contains("file.txt")),
        "file.txt should be staged after pressing s in diff viewer"
    );

    // Status message should mention staging
    let msg = app.status_message.as_deref().unwrap_or("");
    assert!(
        msg.to_lowercase().contains("staged"),
        "status_message should contain 'Staged', got: {msg}"
    );
}

#[test]
fn s_in_diff_viewer_unstages_staged_file_and_returns_to_dashboard() {
    // Setup: create a repo with a staged modification (not unstaged like setup_real_repo_app)
    let env = TestEnv::new();
    let repo_path = env.create_repo("testrepo");

    // Commit a file
    std::fs::write(repo_path.join("file.txt"), "initial").unwrap();
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = std::process::Command::new("git")
        .args(["commit", "-m", "add file"])
        .current_dir(&repo_path)
        .output()
        .expect("git commit failed to run");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Modify the file and stage the modification
    std::fs::write(repo_path.join("file.txt"), "modified").unwrap();
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add (stage modification) failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Build the app with DiffTarget::Head
    let ws = Workspace {
        name: "test-ws".into(),
        path: env.workspaces_dir.clone(),
        repos: vec![WorkspaceRepo {
            name: "testrepo".into(),
            path: repo_path.clone(),
            branch: "main".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![ws], vec![repo_path.clone()]);
    app.load_selected_workspace_detail();

    // Focus right pane, expand repo, navigate to file row
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter)); // expand repo at cursor_row=0
    app.handle_key(key(KeyCode::Down)); // SectionHeader("Staged")
    app.handle_key(key(KeyCode::Down)); // File(file.txt)

    // Verify file.txt is staged before opening viewer
    let files = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files
            .iter()
            .any(|e| e.staged && e.path.contains("file.txt")),
        "file.txt should be staged initially"
    );

    // Open diff viewer
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen after Enter"
    );

    // Assert state.staged == true (we're viewing a staged file)
    if let Screen::DiffViewer(ref state) = app.screen {
        assert!(
            state.staged,
            "DiffViewerState.staged should be true for a staged file"
        );
    }

    // Press 's' to unstage from inside the viewer
    app.handle_key(key(KeyCode::Char('s')));

    // Should return to dashboard
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after unstaging from DiffViewer"
    );

    // file.txt should now be unstaged in repo_file_cache
    let files_after = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files_after
            .iter()
            .any(|e| !e.staged && e.path.contains("file.txt")),
        "file.txt should be unstaged after pressing s in diff viewer"
    );

    // Status message should mention "Unstaged"
    let msg = app.status_message.as_deref().unwrap_or("");
    assert!(
        msg.to_lowercase().contains("unstaged"),
        "status_message should contain 'Unstaged', got: {msg}"
    );
}

#[test]
fn diff_viewer_page_and_home_end_scrolling() {
    use space::core::config::SpaceConfig;
    use space::core::git::{DiffLine, DiffLineKind, DiffTarget, FileDiff};
    use space::tui::actions::ScreenContext;
    use space::tui::screens::diff::DiffViewerState;

    let lines: Vec<DiffLine> = (0..30)
        .map(|i| DiffLine {
            kind: DiffLineKind::Context,
            content: format!(" line {i}"),
        })
        .collect();

    let diff = FileDiff {
        path: "big.txt".into(),
        old_path: None,
        is_binary: false,
        lines,
    };

    let mut state = DiffViewerState {
        repo_index: 0,
        repo_name: "test".into(),
        repo_path: PathBuf::from("/tmp/fake"),
        file_path: "big.txt".into(),
        target: DiffTarget::Head,
        staged: false,
        diff: Ok(diff),
        scroll_offset: 0,
        total_lines: 30,
    };

    let config = SpaceConfig::default();
    let ctx = ScreenContext { config: &config };

    // PageDown from 0 → 10
    state.handle_key(key(KeyCode::PageDown), &ctx);
    assert_eq!(state.scroll_offset, 10, "PageDown from 0 should go to 10");

    // PageDown from 10 → 20
    state.handle_key(key(KeyCode::PageDown), &ctx);
    assert_eq!(state.scroll_offset, 20, "PageDown from 10 should go to 20");

    // PageDown from 20 → capped at 29
    state.handle_key(key(KeyCode::PageDown), &ctx);
    assert_eq!(state.scroll_offset, 29, "PageDown from 20 should cap at 29");

    // PageUp from 29 → 19
    state.handle_key(key(KeyCode::PageUp), &ctx);
    assert_eq!(state.scroll_offset, 19, "PageUp from 29 should go to 19");

    // End → 29
    state.handle_key(key(KeyCode::End), &ctx);
    assert_eq!(state.scroll_offset, 29, "End should go to 29");

    // Home → 0
    state.handle_key(key(KeyCode::Home), &ctx);
    assert_eq!(state.scroll_offset, 0, "Home should go to 0");

    // PageUp from 0 → stays at 0 (no underflow)
    state.handle_key(key(KeyCode::PageUp), &ctx);
    assert_eq!(state.scroll_offset, 0, "PageUp from 0 should stay at 0");
}

#[test]
fn reopening_diff_viewer_uses_cache() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Open diff viewer
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::DiffViewer(_)));

    // Close it
    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));

    // Verify cache has entries for repo 0
    assert!(
        app.diff_content_cache.keys().any(|k| k.repo_index == 0),
        "diff_content_cache should have entries for repo 0 after viewing diff"
    );

    // Count cache entries for repo 0
    let count_before = app
        .diff_content_cache
        .keys()
        .filter(|k| k.repo_index == 0)
        .count();

    // Re-open the same file
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen on re-open"
    );

    // Close again so cache is still visible
    app.handle_key(key(KeyCode::Esc));

    // Count should be the same (cache hit, no new entries)
    let count_after = app
        .diff_content_cache
        .keys()
        .filter(|k| k.repo_index == 0)
        .count();
    assert_eq!(
        count_before, count_after,
        "cache entry count should be the same after re-opening (cache hit): before={count_before}, after={count_after}"
    );
}

#[test]
fn stage_with_invalid_repo_path_shows_error_status() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Corrupt the repo path
    app.workspaces[0].repos[0].path = PathBuf::from("/nonexistent/repo");

    // Press 's' to try staging
    app.handle_key(key(KeyCode::Char('s')));

    // Status message should contain "failed"
    let msg = app.status_message.as_deref().unwrap_or("");
    assert!(
        msg.to_lowercase().contains("failed"),
        "status_message should contain 'failed', got: {msg}"
    );

    // Should still be on dashboard (no crash)
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "should remain on Dashboard after failed stage"
    );
}

#[test]
fn bulk_stage_with_invalid_repo_path_shows_error_status() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();

    // Move cursor to repo row
    app.cursor_row = 0;

    // Corrupt the repo path
    app.workspaces[0].repos[0].path = PathBuf::from("/nonexistent/repo");

    // Press shift+S to try bulk staging
    app.handle_key(shift_key(KeyCode::Char('S')));

    // Status message should contain "failed"
    let msg = app.status_message.as_deref().unwrap_or("");
    assert!(
        msg.to_lowercase().contains("failed"),
        "status_message should contain 'failed', got: {msg}"
    );

    // Should still be on dashboard (no crash)
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "should remain on Dashboard after failed bulk stage"
    );
}

#[test]
fn external_change_invalidates_diff_content_cache() {
    let (_env, repo_path, mut app) = setup_real_repo_app();

    // Open diff viewer (populates diff_content_cache)
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen"
    );

    // Close diff viewer
    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));

    // Assert diff_content_cache is non-empty
    assert!(
        !app.diff_content_cache.is_empty(),
        "diff_content_cache should be populated after viewing a diff"
    );

    // Remember the old cache content for comparison
    let old_cache_keys: Vec<_> = app.diff_content_cache.keys().cloned().collect();

    // Simulate an external change: stage the current modification (changes .git/index mtime),
    // then write new content so there's still an unstaged diff to view.
    let out = std::process::Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(&repo_path)
        .output()
        .expect("git add failed to run");
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Write new content so there's still an unstaged change against HEAD
    std::fs::write(repo_path.join("file.txt"), "externally modified content").unwrap();

    // Open diff viewer again -- staleness check should invalidate the cache
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen after re-opening"
    );

    // The diff_content_cache should have been invalidated and repopulated with fresh data.
    // Verify the new diff reflects the external change by checking that a cache entry exists
    // and contains the new content.
    let has_new_content = app.diff_content_cache.values().any(|result| {
        if let Ok(diff) = result {
            diff.lines
                .iter()
                .any(|line| line.content.contains("externally modified content"))
        } else {
            false
        }
    });
    assert!(
        has_new_content,
        "diff_content_cache should reflect the externally modified content; keys: {:?}",
        old_cache_keys
    );
}

#[test]
fn external_file_edit_invalidates_cached_unstaged_diff() {
    let (_env, repo_path, mut app) = setup_real_repo_app();

    // Open diff viewer — populates diff_content_cache and file_mtime_cache
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen"
    );

    // Close diff viewer
    app.handle_key(key(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Dashboard));

    // Assert caches are populated
    assert!(
        !app.diff_content_cache.is_empty(),
        "diff_content_cache should be populated after viewing a diff"
    );
    assert!(
        !app.file_mtime_cache.is_empty(),
        "file_mtime_cache should be populated after viewing an unstaged diff"
    );

    // Externally modify the file (without staging — .git/index mtime doesn't change)
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(repo_path.join("file.txt"), "changed again").unwrap();

    // Open diff viewer again — file mtime staleness check should invalidate the cache
    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::DiffViewer(_)),
        "expected DiffViewer screen after re-opening"
    );

    // The diff content should reflect the new file content
    let has_new_content = app.diff_content_cache.values().any(|result| {
        if let Ok(diff) = result {
            diff.lines
                .iter()
                .any(|line| line.content.contains("changed again"))
        } else {
            false
        }
    });
    assert!(
        has_new_content,
        "diff_content_cache should reflect the externally modified file content 'changed again'"
    );
}
