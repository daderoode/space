mod common;

use common::{key, test_app, test_app_with_config, TestEnv};
use ratatui::crossterm::event::KeyCode;
use space::core::config::{RepoConfig, SpaceConfig, WorkspaceConfig};
use space::core::workspace::{Workspace, WorkspaceRepo};
use space::tui::app::{Pane, Screen};
use std::path::PathBuf;

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
fn right_arrow_on_repo_pane_is_noop_for_now() {
    // Right arrow on repos pane will trigger ToggleRepoExpand in Task 4.
    // For now it is a noop -- just confirm it doesn't crash.
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
    assert_eq!(rows.len(), 4); // repo-a, repo-b, foo.rs, bar.rs
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
        space::tui::app::RepoRow::File { repo_index: 1, .. }
    ));
    assert!(matches!(
        rows[3],
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
    app.cursor_row = 2; // repo-a=0, repo-b=1, x.rs=2
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
    // rows: [Repo(0), File(a.rs), File(b.rs)]
    assert_eq!(app.cursor_row, 0);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 1);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 2);
    app.handle_key(key(KeyCode::Down)); // clamp at end
    assert_eq!(app.cursor_row, 2);
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
