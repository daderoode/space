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
