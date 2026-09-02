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
// git::remote_url tests use these directly
use git2;
use tempfile;

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

/// Drive the background sync worker (Syncing stage) to completion by polling
/// `poll_sync_result`, mirroring what the real run loop does each frame, then
/// continue past the finished sync report with Enter.
fn drain_sync(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.poll_sync_result();
        let done = match &app.screen {
            Screen::CreateWorkspace(st)
                if st.stage == space::tui::screens::create::CreateStage::Syncing =>
            {
                st.report.done
            }
            Screen::AddRepos(st) if st.stage == space::tui::screens::add::AddStage::Syncing => {
                st.report.done
            }
            _ => true,
        };
        if done {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sync worker did not complete within timeout"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let on_report = matches!(
        app.screen,
        Screen::CreateWorkspace(ref st)
            if st.stage == space::tui::screens::create::CreateStage::Syncing
    ) || matches!(
        app.screen,
        Screen::AddRepos(ref st)
            if st.stage == space::tui::screens::add::AddStage::Syncing
    );
    if on_report {
        app.handle_key(key(KeyCode::Enter));
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
fn create_enter_name_advances_to_pick_repos() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterName
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }

    app.handle_key(key(KeyCode::Char('m')));
    app.handle_key(key(KeyCode::Char('y')));
    app.handle_key(key(KeyCode::Char('-')));
    app.handle_key(key(KeyCode::Char('w')));
    app.handle_key(key(KeyCode::Char('s')));
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickRepos,
            "EnterName + Enter should advance to PickRepos"
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

    // First Enter: PickBranchStrategy → EnterBranchName (pre-filled with ws_name)
    app.handle_key(key(KeyCode::Enter));
    // Second Enter: EnterBranchName → Creating (confirms pre-filled name)
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
fn create_esc_from_enter_name_exits_to_dashboard() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    app.handle_key(key(KeyCode::Esc));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "Esc from EnterName should return to Dashboard, got {:?}",
        std::mem::discriminant(&app.screen)
    );
}

#[test]
fn create_esc_from_pick_repos_returns_to_enter_name() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.ws_name = tui_input::Input::default().with_value("my-ws".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickRepos;
    }

    app.handle_key(key(KeyCode::Esc));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterName,
            "Esc from PickRepos should go back to EnterName"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_empty_name_rejected() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));

    // EnterName is the initial stage — press Enter immediately with empty name
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterName,
            "stage should stay EnterName when name is empty"
        );
        assert!(st.error.is_some(), "error should be set for empty name");
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_populates_recent_branches_on_pick_repos_enter() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("branchy-repo");

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
    let out = std::process::Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.ws_name = tui_input::Input::default().with_value("test-ws".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickRepos;
        // Picker was built from repos_cache (contains repo_path).
        // Toggle the highlighted item explicitly so confirmed_items() returns it.
        st.picker.toggle_highlighted();
    }

    app.handle_key(key(KeyCode::Enter));
    drain_sync(&mut app);

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy,
        );
        assert!(
            !st.recent_branches.is_empty(),
            "recent_branches should be populated after confirming repos"
        );
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

    // First Enter: PickBranchStrategy → EnterBranchName (pre-filled with workspace name)
    app.handle_key(key(KeyCode::Enter));
    // Second Enter: EnterBranchName → Creating (confirms pre-filled name)
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
        st.stage = space::tui::screens::create::CreateStage::PickRepos;
        st.branch_strategy_idx = 7;
        // Toggle the highlighted item explicitly so confirmed_items() returns it.
        st.picker.toggle_highlighted();
    }
    app.handle_key(key(KeyCode::Enter));
    drain_sync(&mut app);

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranchStrategy,
        );
        assert_eq!(
            st.branch_strategy_idx, 0,
            "branch_strategy_idx must be reset to 0 on repo confirmation (was 7)"
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
    // j/k skip SectionHeader rows, so navigation goes: Repo → a.rs → b.rs
    assert_eq!(app.cursor_row, 0);
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 2); // skips SectionHeader("Unstaged"), lands on a.rs
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.cursor_row, 4); // skips SectionHeader("Staged"), lands on b.rs
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
fn j_skips_section_headers() {
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
                status: FileStatus::Added,
                staged: true,
                insertions: 5,
                deletions: 0,
            },
        ],
    );
    // rows: [Repo(0), SectionHeader("Unstaged"), File(a.rs), SectionHeader("Staged"), File(b.rs)]
    // cursor at 0 (Repo)
    app.cursor_row = 0;
    app.handle_key(key(KeyCode::Down)); // should skip SectionHeader("Unstaged") → land on File(a.rs) at row 2
    assert_eq!(
        app.cursor_row, 2,
        "Down from Repo should skip SectionHeader and land on a.rs"
    );

    app.handle_key(key(KeyCode::Down)); // should skip SectionHeader("Staged") → land on File(b.rs) at row 4
    assert_eq!(
        app.cursor_row, 4,
        "Down from a.rs should skip SectionHeader and land on b.rs"
    );

    app.handle_key(key(KeyCode::Down)); // clamp at 4
    assert_eq!(app.cursor_row, 4, "Down at end should stay at b.rs");
}

#[test]
fn k_skips_section_headers() {
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
                status: FileStatus::Added,
                staged: true,
                insertions: 5,
                deletions: 0,
            },
        ],
    );
    // rows: [Repo(0), SectionHeader("Unstaged"), File(a.rs), SectionHeader("Staged"), File(b.rs)]
    app.cursor_row = 4; // start at b.rs
    app.handle_key(key(KeyCode::Up)); // should skip SectionHeader("Staged") → land on a.rs at row 2
    assert_eq!(
        app.cursor_row, 2,
        "Up from b.rs should skip SectionHeader and land on a.rs"
    );

    app.handle_key(key(KeyCode::Up)); // should skip SectionHeader("Unstaged") → land on Repo at row 0
    assert_eq!(
        app.cursor_row, 0,
        "Up from a.rs should skip SectionHeader and land on Repo"
    );
}

#[test]
fn cursor_repositions_after_staging() {
    use space::core::git::{FileEntry, FileStatus};

    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.expanded_repos.insert(0);
    // Two unstaged files
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
                staged: false,
                insertions: 2,
                deletions: 0,
            },
        ],
    );
    // rows: [Repo(0), SectionHeader("Unstaged"), File(a.rs at 2), File(b.rs at 3)]
    // cursor on a.rs
    app.cursor_row = 2;

    // Simulate staging a.rs by updating cache directly (no real git repo)
    app.repo_file_cache.insert(
        0,
        vec![
            FileEntry {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: true, // now staged
                insertions: 1,
                deletions: 0,
            },
            FileEntry {
                path: "b.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 2,
                deletions: 0,
            },
        ],
    );
    // rows after: [Repo(0), SectionHeader("Unstaged"), File(b.rs at 2), SectionHeader("Staged"), File(a.rs at 4)]
    // cursor_row=2 was a.rs (now b.rs is at row 2) — cursor stays valid
    // Apply the reposition logic manually by calling Down then verifying sensible state
    // The key invariant: cursor must NOT be on a SectionHeader
    let rows = app.flattened_rows();
    let cursor = app.cursor_row.min(rows.len().saturating_sub(1));
    assert!(
        !matches!(rows[cursor], space::tui::app::RepoRow::SectionHeader { .. }),
        "cursor should not rest on a SectionHeader after section structure change"
    );
}

/// End-to-end test: stage a file via the `s` key on a real repo and verify the
/// cursor lands on a non-SectionHeader row (proves `reposition_after_section_change`
/// fires correctly through the `StageFile` message handler).
#[test]
fn cursor_not_on_section_header_after_s_key_stages_file() {
    let (_env, _repo_path, mut app) = setup_real_repo_app();
    // setup_real_repo_app leaves cursor on the file row (unstaged modification)
    // rows: [Repo(0), SectionHeader("Unstaged"), File(file.txt)]
    // cursor is at row 2 (the file row)
    assert!(
        matches!(
            app.flattened_rows().get(app.cursor_row),
            Some(space::tui::app::RepoRow::File { .. })
        ),
        "pre-condition: cursor should start on a File row"
    );

    // Press s to stage the file
    app.handle_key(key(KeyCode::Char('s')));

    // After staging, rows change:
    // [Repo(0), SectionHeader("Staged"), File(file.txt staged)]
    // cursor must not rest on a SectionHeader
    let rows = app.flattened_rows();
    let cursor = app.cursor_row;
    assert!(
        cursor < rows.len(),
        "cursor_row {cursor} must be within bounds (rows len {})",
        rows.len()
    );
    assert!(
        !matches!(rows[cursor], space::tui::app::RepoRow::SectionHeader { .. }),
        "cursor must not rest on SectionHeader after staging, got row {cursor}"
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

    // Navigate to the file row (Down skips SectionHeader, second Down is a no-op at end)
    app.handle_key(key(KeyCode::Down)); // skips SectionHeader("Unstaged") → File(file.txt)
    app.handle_key(key(KeyCode::Down)); // clamp: already at last row, stays on File(file.txt)

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

    // Focus right pane and expand repo (cache is populated lazily on expand)
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Enter));

    // Verify file is staged before we unstage (cache now populated after expand)
    let files_before = app.repo_file_cache.get(&0).expect("cache should exist");
    assert!(
        files_before.iter().any(|e| e.staged),
        "should have at least one staged file before U"
    );

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
    app.handle_key(key(KeyCode::Down)); // skips SectionHeader("Staged") → File(file.txt)
    app.handle_key(key(KeyCode::Down)); // clamp: already at last row, stays on File(file.txt)

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
    use space::core::git::{DiffLine, DiffLineKind, FileDiff};
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

// ---------------------------------------------------------------------------
// Partially-staged file detection
// ---------------------------------------------------------------------------

#[test]
fn partially_staged_file_shows_in_both_sections() {
    use space::core::git::{FileEntry, FileStatus};

    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.expanded_repos.insert(0);

    // Simulate a partially-staged file: same path "main.rs" appears
    // in both staged and unstaged (e.g. only some hunks were staged)
    app.repo_file_cache.insert(
        0,
        vec![
            FileEntry {
                path: "main.rs".into(),
                status: FileStatus::Modified,
                staged: true, // the staged version
                insertions: 3,
                deletions: 1,
            },
            FileEntry {
                path: "main.rs".into(),
                status: FileStatus::Modified,
                staged: false, // the unstaged version
                insertions: 2,
                deletions: 0,
            },
            FileEntry {
                path: "other.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 1,
                deletions: 0,
            },
        ],
    );

    let rows = app.flattened_rows();

    // Collect all File rows
    let file_rows: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            if let space::tui::app::RepoRow::File {
                entry,
                partially_staged,
                ..
            } = r
            {
                Some((entry.path.as_str(), entry.staged, *partially_staged))
            } else {
                None
            }
        })
        .collect();

    // main.rs should appear twice (once unstaged, once staged) and both should be marked partial
    let main_rows: Vec<_> = file_rows
        .iter()
        .filter(|(p, _, _)| *p == "main.rs")
        .collect();
    assert_eq!(main_rows.len(), 2, "main.rs should appear in both sections");
    assert!(
        main_rows.iter().all(|(_, _, partial)| *partial),
        "both main.rs entries should be marked partially_staged"
    );

    // other.rs should not be marked partial
    let other = file_rows
        .iter()
        .find(|(p, _, _)| *p == "other.rs")
        .expect("other.rs should appear");
    assert!(!other.2, "other.rs should not be marked partially_staged");
}

/// Defensive test: a Conflicted entry with staged=true (impossible from git2 today,
/// but guarded against) must appear only in the Conflicts section, not the Staged section.
#[test]
fn conflicted_staged_entry_appears_only_in_conflicts_section() {
    use space::core::git::{FileEntry, FileStatus};

    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.expanded_repos.insert(0);

    // Inject a hypothetical entry that is both Conflicted and staged=true.
    // This state is impossible from file_diff() today (conflicts always have staged=false),
    // but Fix 1 defensively excludes it from the Staged section regardless.
    app.repo_file_cache.insert(
        0,
        vec![FileEntry {
            path: "conflict.rs".into(),
            status: FileStatus::Conflicted,
            staged: true, // hypothetical — should NOT land in Staged section
            insertions: 0,
            deletions: 0,
        }],
    );

    let rows = app.flattened_rows();

    // Should appear in the Conflicts section only
    let section_labels: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            if let space::tui::app::RepoRow::SectionHeader { label, .. } = r {
                Some(*label)
            } else {
                None
            }
        })
        .collect();

    assert!(
        section_labels.contains(&"Conflicts"),
        "expected a Conflicts section header"
    );
    assert!(
        !section_labels.contains(&"Staged"),
        "Conflicted entry with staged=true must not create a Staged section"
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

    // Externally modify the file (without staging — .git/index mtime doesn't change).
    // Poll until the filesystem mtime actually advances before writing, so the test
    // is not sensitive to mtime granularity (ext4 = 1s, APFS = 1ns).
    let pre_mtime = std::fs::metadata(repo_path.join("file.txt"))
        .and_then(|m| m.modified())
        .ok();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        std::fs::write(repo_path.join("file.txt"), "changed again").unwrap();
        let new_mtime = std::fs::metadata(repo_path.join("file.txt"))
            .and_then(|m| m.modified())
            .ok();
        if new_mtime != pre_mtime {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break; // proceed anyway — assert below will catch if cache wasn't invalidated
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

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

// ---------------------------------------------------------------------------
// Horizontal table scroll tests
// ---------------------------------------------------------------------------

#[test]
fn scroll_table_right_increments_scroll_x() {
    let ws = common::workspace_with_repos(&["api"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    assert_eq!(app.table_scroll_x, 0);
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.table_scroll_x, 5);
}

#[test]
fn scroll_table_left_decrements_scroll_x() {
    let ws = common::workspace_with_repos(&["api"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.table_scroll_x = 10;
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.table_scroll_x, 5);
}

#[test]
fn scroll_table_left_at_zero_stays_zero() {
    let ws = common::workspace_with_repos(&["api"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Char('h')));
    assert_eq!(app.table_scroll_x, 0);
}

#[test]
fn scroll_table_resets_on_workspace_switch() {
    let ws1 = common::workspace_with_repos(&["api"]);
    let ws2 = common::workspace_with_repos(&["web"]);
    let mut app = test_app(vec![ws1, ws2], vec![]);
    app.table_scroll_x = 30;
    app.handle_key(key(KeyCode::Down)); // SelectWorkspaceDown on Left pane (focus is Left by default)
    assert_eq!(app.table_scroll_x, 0);
}

#[test]
fn scroll_table_left_pane_noop() {
    let ws = common::workspace_with_repos(&["api"]);
    let mut app = test_app(vec![ws], vec![]);
    // focus is Left by default
    app.handle_key(key(KeyCode::Char('l')));
    assert_eq!(app.table_scroll_x, 0);
}

#[test]
fn scroll_x_offsets_branch_content_in_render() {
    use space::core::git::RepoStatus;
    use space::core::workspace::{Workspace, WorkspaceRepo};

    let ws = Workspace {
        name: "ws".into(),
        path: std::path::PathBuf::from("/tmp/ws"),
        repos: vec![WorkspaceRepo {
            name: "api".into(),
            path: std::path::PathBuf::from("/tmp/ws/api"),
            branch: "feature/very-long-branch-name-here".into(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;

    // At scroll_x=0 branch shows from start.
    // Use 160 cols: right pane inner_width=118, branch_display=25, max_scroll=28.
    // truncate_for_width("feature/very-long-branch-name-here", 25) = "feature/very-long-branch..."
    let rendered0 = render_text(&app, 160, 20);
    assert!(
        rendered0.contains("feature/very-long"),
        "branch start visible at scroll 0"
    );

    // At scroll_x=19 first 19 chars are scrolled off.
    // "feature/very-long-branch-name-here"[19..] = "ranch-name-here"
    app.table_scroll_x = 19;
    let rendered20 = render_text(&app, 160, 20);
    assert!(
        rendered20.contains("ranch-name-here"),
        "branch content offset by scroll_x=19"
    );
    assert!(
        !rendered20.contains("feature/"),
        "branch start scrolled off at scroll_x=19"
    );
}

// ---------------------------------------------------------------------------
// Branch name editor (Task 3b)
// ---------------------------------------------------------------------------

#[test]
fn create_new_branch_enters_name_stage() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.ws_name = tui_input::Input::default().with_value("my-feature".to_string());
        st.branch_strategy_idx = 0;
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
    }

    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterBranchName,
            "selecting New branch should open branch name editor"
        );
        assert_eq!(
            st.branch_name_input.value(),
            "my-feature",
            "branch name input should be pre-filled with workspace name"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_new_branch_name_preserved_on_reentry() {
    // User types a custom name, Escs back, re-selects "New branch" — input must be preserved.
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.ws_name = tui_input::Input::default().with_value("my-ws".to_string());
        st.stage = space::tui::screens::create::CreateStage::EnterBranchName;
        st.branch_name_input =
            tui_input::Input::default().with_value("feature/DEV-9999".to_string());
    }

    // Esc back to PickBranchStrategy
    app.handle_key(key(KeyCode::Esc));
    // Re-select "New branch" (idx 0)
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.branch_strategy_idx = 0;
    }
    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterBranchName,
        );
        assert_eq!(
            st.branch_name_input.value(),
            "feature/DEV-9999",
            "custom branch name must be preserved after Esc-then-reentry"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_new_branch_esc_returns_to_strategy() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.ws_name = tui_input::Input::default().with_value("my-feature".to_string());
        st.stage = space::tui::screens::create::CreateStage::EnterBranchName;
        st.branch_name_input = tui_input::Input::default().with_value("my-feature".to_string());
    }

    app.handle_key(key(KeyCode::Esc));

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
fn create_new_branch_empty_name_rejected() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.ws_name = tui_input::Input::default().with_value("my-feature".to_string());
        st.stage = space::tui::screens::create::CreateStage::EnterBranchName;
        st.branch_name_input = tui_input::Input::default(); // empty
    }

    app.handle_key(key(KeyCode::Enter));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::EnterBranchName,
            "empty branch name should be rejected"
        );
        assert!(
            st.error.is_some(),
            "error should be set for empty branch name"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_new_branch_custom_name_creates_worktree() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("branch-name-repo");

    let config = config_from_env(&env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("my-ws".to_string());
        st.stage = space::tui::screens::create::CreateStage::EnterBranchName;
        st.branch_name_input =
            tui_input::Input::default().with_value("feature/DEV-1234".to_string());
    }

    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after creating with custom branch name"
    );
    assert!(env
        .workspaces_dir
        .join("my-ws")
        .join("branch-name-repo")
        .exists());
}

#[test]
fn add_new_branch_enters_name_stage() {
    let ws_name = "existing-ws";
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: PathBuf::from("/tmp").join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![PathBuf::from("/tmp/repos/foo")]);
    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.branch_strategy_idx = 0;
        st.stage = space::tui::screens::add::AddStage::PickBranchStrategy;
    }

    app.handle_key(key(KeyCode::Enter));

    if let Screen::AddRepos(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::add::AddStage::EnterBranchName,
            "selecting New branch in Add flow should open branch name editor"
        );
        assert_eq!(
            st.branch_name_input.value(),
            ws_name,
            "branch name input should be pre-filled with workspace name"
        );
    } else {
        panic!("expected AddRepos screen");
    }
}

#[test]
fn add_new_branch_esc_returns_to_strategy() {
    let ws_name = "existing-ws";
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: PathBuf::from("/tmp").join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![PathBuf::from("/tmp/repos/foo")]);
    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::add::AddStage::EnterBranchName;
        st.branch_name_input = tui_input::Input::default().with_value("existing-ws".to_string());
    }

    app.handle_key(key(KeyCode::Esc));

    if let Screen::AddRepos(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::add::AddStage::PickBranchStrategy
        );
        assert!(st.error.is_none(), "error should be cleared on Esc");
    } else {
        panic!("expected AddRepos screen");
    }
}

#[test]
fn add_new_branch_empty_name_rejected() {
    let ws_name = "existing-ws";
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: PathBuf::from("/tmp").join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app(workspaces, vec![PathBuf::from("/tmp/repos/foo")]);
    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![PathBuf::from("/tmp/repos/foo")];
        st.stage = space::tui::screens::add::AddStage::EnterBranchName;
        st.branch_name_input = tui_input::Input::default(); // empty
    }

    app.handle_key(key(KeyCode::Enter));

    if let Screen::AddRepos(ref st) = app.screen {
        assert_eq!(
            st.stage,
            space::tui::screens::add::AddStage::EnterBranchName,
            "empty branch name should be rejected"
        );
        assert!(
            st.error.is_some(),
            "error should be set for empty branch name"
        );
    } else {
        panic!("expected AddRepos screen");
    }
}

#[test]
fn add_new_branch_custom_name_creates_worktree() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("add-branch-name-repo");

    let ws_name = "add-ws";
    std::fs::create_dir_all(env.workspaces_dir.join(ws_name)).unwrap();

    let config = config_from_env(&env);
    let workspaces = vec![Workspace {
        name: ws_name.to_string(),
        path: env.workspaces_dir.join(ws_name),
        repos: vec![],
    }];
    let mut app = test_app_with_config(config, workspaces, vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.stage = space::tui::screens::add::AddStage::EnterBranchName;
        st.branch_name_input =
            tui_input::Input::default().with_value("feature/DEV-9999".to_string());
    }

    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after add with custom branch name"
    );
    assert!(
        env.workspaces_dir
            .join(ws_name)
            .join("add-branch-name-repo")
            .exists(),
        "worktree should have been created"
    );
}

// ---------------------------------------------------------------------------
// git::remote_url tests
// ---------------------------------------------------------------------------

#[test]
fn git_remote_url_returns_origin_url() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    repo.remote("origin", "https://github.com/test/my-repo")
        .unwrap();
    let url = space::core::git::remote_url(dir.path());
    assert_eq!(url, Some("https://github.com/test/my-repo".to_string()));
}

#[test]
fn git_remote_url_returns_none_when_no_remote() {
    let dir = tempfile::tempdir().unwrap();
    git2::Repository::init(dir.path()).unwrap();
    let url = space::core::git::remote_url(dir.path());
    assert!(url.is_none());
}

#[test]
fn scroll_x_past_branch_virtual_offsets_status_content() {
    use space::core::git::RepoStatus;
    use space::core::workspace::{Workspace, WorkspaceRepo};

    // Narrow terminal: max_scroll will be > BRANCH_VIRTUAL so STATUS scrolls.
    // inner_width=60 -> branch_display=13, status_display=21, max_scroll=61
    // At scroll_x=55, status_offset=55-50=5
    let ws = Workspace {
        name: "ws".into(),
        path: std::path::PathBuf::from("/tmp/ws"),
        repos: vec![WorkspaceRepo {
            name: "api".into(),
            path: std::path::PathBuf::from("/tmp/ws/api"),
            branch: "main".into(),
            status: RepoStatus {
                modified: 10,
                staged: 5,
                untracked: 3,
                conflicted: 0,
            },
            ahead: 0,
            behind: 0,
        }],
    };
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.table_scroll_x = 55;

    // render with width=80, height=10 (80 is the minimum allowed width)
    // Right pane = 75% of 80 = 60 cols wide, inner_width=58
    // branch_display=12, status_display=20, max_scroll=63
    // At scroll_x=55, status_offset=5
    let rendered = render_text(&app, 80, 10);

    // status_offset = 5, full status = "10 modified, 5 staged, 3 new"
    // After skipping 5 chars: "odified, 5 staged, 3 new"
    // We should see some part of the status string after the skip
    // The key check: status content IS rendered (not empty due to the bug)
    assert!(
        rendered.contains("odified") || rendered.contains("staged") || rendered.contains("new"),
        "STATUS content visible after scrolling past BRANCH_VIRTUAL: {:?}",
        rendered
    );
}

// ---------------------------------------------------------------------------
// SwitchBranch screen tests (Task 2)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod switch_branch_tests {
    use super::*;
    use space::tui::actions::ScreenAction;
    use space::tui::screens::switch_branch::{SwitchBranchStage, SwitchBranchState};

    fn stub_state() -> SwitchBranchState {
        SwitchBranchState::new(
            "my-repo".to_string(),
            std::path::PathBuf::from("/tmp/my-repo"),
        )
    }

    fn real_repo_state() -> (tempfile::TempDir, SwitchBranchState) {
        let dir = tempfile::TempDir::new().unwrap();
        common::init_repo(dir.path());
        let status = std::process::Command::new("git")
            .args(["branch", "feature-a"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git branch feature-a failed");
        let state = SwitchBranchState::new("my-repo".to_string(), dir.path().to_path_buf());
        (dir, state)
    }

    fn make_ctx() -> space::tui::actions::ScreenContext<'static> {
        use space::core::config::SpaceConfig;
        static CFG: std::sync::OnceLock<SpaceConfig> = std::sync::OnceLock::new();
        let cfg = CFG.get_or_init(SpaceConfig::default);
        space::tui::actions::ScreenContext { config: cfg }
    }

    #[test]
    fn esc_from_pick_strategy_returns_back() {
        let mut state = stub_state();
        let action = state.handle_key(
            common::key(ratatui::crossterm::event::KeyCode::Esc),
            &make_ctx(),
        );
        assert!(matches!(action, ScreenAction::Back));
    }

    #[test]
    fn down_then_up_stays_in_bounds() {
        let mut state = stub_state();
        let max_idx = state.max_idx();
        for _ in 0..max_idx + 5 {
            state.handle_key(
                common::key(ratatui::crossterm::event::KeyCode::Down),
                &make_ctx(),
            );
        }
        assert_eq!(state.strategy_idx, max_idx);
        for _ in 0..max_idx + 5 {
            state.handle_key(
                common::key(ratatui::crossterm::event::KeyCode::Up),
                &make_ctx(),
            );
        }
        assert_eq!(state.strategy_idx, 0);
    }

    #[test]
    fn enter_on_new_branch_transitions_to_enter_branch_name() {
        let mut state = stub_state();
        state.handle_key(
            common::key(ratatui::crossterm::event::KeyCode::Enter),
            &make_ctx(),
        );
        assert_eq!(state.stage, SwitchBranchStage::EnterBranchName);
    }

    #[test]
    fn enter_branch_name_empty_shows_error() {
        let mut state = stub_state();
        state.stage = SwitchBranchStage::EnterBranchName;
        let action = state.handle_key(
            common::key(ratatui::crossterm::event::KeyCode::Enter),
            &make_ctx(),
        );
        assert!(matches!(action, ScreenAction::Continue));
        assert!(state.error.is_some());
    }

    #[test]
    fn enter_branch_name_returns_switch_action() {
        use ratatui::crossterm::event::KeyCode;
        let mut state = stub_state();
        state.stage = SwitchBranchStage::EnterBranchName;
        for ch in "new-feature".chars() {
            state.handle_key(common::key(KeyCode::Char(ch)), &make_ctx());
        }
        let action = state.handle_key(common::key(KeyCode::Enter), &make_ctx());
        assert!(
            matches!(action, ScreenAction::SwitchRepoBranch { ref branch, new_branch: true, .. } if branch == "new-feature")
        );
    }

    #[test]
    fn b_key_on_repo_row_opens_switch_branch_screen() {
        use ratatui::crossterm::event::KeyCode;
        use space::tui::app::{Pane, Screen};

        let ws = common::workspace_with_repos(&["alpha", "beta"]);
        let mut app = common::test_app(vec![ws], vec![]);
        app.focus = Pane::Right;

        app.handle_key(common::key(KeyCode::Char('b')));

        assert!(
            matches!(app.screen, Screen::SwitchBranch(_)),
            "b on a repo row should open SwitchBranch screen"
        );
    }

    #[test]
    fn enter_on_recent_branch_returns_switch_action() {
        use ratatui::crossterm::event::KeyCode;
        let (_dir, mut state) = real_repo_state();
        // Index 0 = New branch, index 1 = first recent branch
        state.handle_key(common::key(KeyCode::Down), &make_ctx());
        assert_eq!(state.strategy_idx, 1);
        let action = state.handle_key(common::key(KeyCode::Enter), &make_ctx());
        assert!(
            matches!(
                action,
                ScreenAction::SwitchRepoBranch {
                    new_branch: false,
                    ..
                }
            ),
            "selecting a recent branch should return SwitchRepoBranch with new_branch=false"
        );
    }

    #[test]
    fn navigation_bounds_with_real_branches() {
        use ratatui::crossterm::event::KeyCode;
        let (_dir, mut state) = real_repo_state();
        let max_idx = state.max_idx();
        assert!(
            max_idx >= 2,
            "real repo with feature-a branch should have at least 2 navigation entries"
        );
        for _ in 0..max_idx + 5 {
            state.handle_key(common::key(KeyCode::Down), &make_ctx());
        }
        assert_eq!(state.strategy_idx, max_idx);
    }

    #[test]
    fn b_key_on_left_pane_does_nothing() {
        use ratatui::crossterm::event::KeyCode;
        use space::tui::app::{Pane, Screen};

        let ws = common::workspace_with_repos(&["alpha"]);
        let mut app = common::test_app(vec![ws], vec![]);
        app.focus = Pane::Left;

        app.handle_key(common::key(KeyCode::Char('b')));

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "b on left pane should not open SwitchBranch screen"
        );
    }
}

// ---------------------------------------------------------------------------
// Git operations menu tests (Phase 1: Skeleton)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod gitops_tests {
    use super::*;

    #[test]
    fn g_key_on_repo_row_opens_gitops_menu() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;

        // cursor_row = 0 is the repo row in the Right pane
        app.handle_key(key(KeyCode::Char('G')));

        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "G on a repo row should open the git ops menu"
        );
    }

    #[test]
    fn g_key_on_non_repo_row_does_nothing() {
        use space::core::git::{FileEntry, FileStatus};

        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.expanded_repos.insert(0);
        app.repo_file_cache.insert(
            0,
            vec![FileEntry {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: false,
                insertions: 1,
                deletions: 0,
            }],
        );
        // rows: [Repo(0), SectionHeader("Unstaged")(1), File(a.rs)(2)]
        app.cursor_row = 2; // land on a file row, not a repo row

        app.handle_key(key(KeyCode::Char('G')));

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "G on a file row should not open the git ops menu"
        );

        // Also: G in the Left pane must not open the menu
        let ws2 = common::workspace_with_repos(&["repo-a"]);
        let mut app2 = test_app(vec![ws2], vec![]);
        app2.focus = Pane::Left;
        app2.handle_key(key(KeyCode::Char('G')));
        assert!(
            matches!(app2.screen, Screen::Dashboard),
            "G in the left pane should not open the git ops menu"
        );
    }

    #[test]
    fn gitops_menu_renders_repo_name_and_action_labels() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        let rendered = render_text(&app, 80, 24);

        assert!(
            rendered.contains("Git: repo-a"),
            "menu title should name the repo, got:\n{}",
            rendered
        );
        for label in ["fetch", "pull", "push", "commit", "log", "rebase"] {
            assert!(
                rendered.contains(label),
                "menu should list the {:?} action, got:\n{}",
                label,
                rendered
            );
        }
    }

    #[test]
    fn gitops_menu_j_k_move_the_highlight() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        // Initially the first item (fetch) is highlighted.
        let r0 = render_text(&app, 80, 24);
        assert!(
            r0.contains("> f  fetch"),
            "fetch should start highlighted, got:\n{}",
            r0
        );

        // Down moves the highlight to pull.
        app.handle_key(key(KeyCode::Char('j')));
        let r1 = render_text(&app, 80, 24);
        assert!(
            r1.contains("> p  pull") && !r1.contains("> f  fetch"),
            "j should move the highlight from fetch to pull, got:\n{}",
            r1
        );

        // Up moves the highlight back to fetch.
        app.handle_key(key(KeyCode::Char('k')));
        let r2 = render_text(&app, 80, 24);
        assert!(
            r2.contains("> f  fetch") && !r2.contains("> p  pull"),
            "k should move the highlight back to fetch, got:\n{}",
            r2
        );
    }

    #[test]
    fn gitops_menu_f_starts_fetch_and_transitions_to_running() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        app.handle_key(key(KeyCode::Char('f')));

        // Assert on the transition + rx immediately after the keypress: the
        // worker's git fetch is asynchronous, so the stage/rx are set now even
        // though the fetch against a fake path will later fail.
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Running,
                "pressing f must transition the git ops overlay to the Running stage"
            ),
            _ => panic!("expected the git ops overlay to stay open in the Running stage"),
        }
        assert!(
            app.gitop_rx.is_some(),
            "pressing f must start the fetch worker (gitop_rx set)"
        );
    }

    #[test]
    fn gitops_menu_p_starts_pull_and_transitions_to_running() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        app.handle_key(key(KeyCode::Char('p')));

        // As with fetch, the pull worker runs asynchronously: the stage/rx are
        // set immediately even though the pull against a fake path later fails.
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Running,
                "pressing p must transition the git ops overlay to the Running stage"
            ),
            _ => panic!("expected the git ops overlay to stay open in the Running stage"),
        }
        assert!(
            app.gitop_rx.is_some(),
            "pressing p must start the pull worker (gitop_rx set)"
        );
        // The Running header must reflect the actual op, not a hardcoded "Fetch".
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Pulling"),
            "the pull Running header should say Pulling, got:\n{}",
            rendered
        );
    }

    #[test]
    fn gitops_menu_commit_disabled_when_nothing_staged() {
        // workspace_with_repos uses a fake /tmp path, so file_diff fails and
        // has_staged is false -> commit is disabled.
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        app.handle_key(key(KeyCode::Char('c')));

        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "firing a disabled commit should keep the menu open"
        );
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Stage files first with s/S"),
            "commit with nothing staged should show the stage-first hint, got:\n{}",
            rendered
        );
    }

    #[test]
    fn gitops_menu_r_opens_rebase_preflight_blocked_on_fake_repo() {
        use space::tui::screens::gitops::GitOpsStage;
        // workspace_with_repos uses a fake /tmp path, so Repository::open fails
        // and is_on_branch returns false: the pre-flight blocks with the
        // detached-HEAD reason on the unopenable-path case. The genuine
        // detached-HEAD case (a real repo) is covered by
        // gitops_rebase_preflight_blocks_genuine_detached_head.
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        app.handle_key(key(KeyCode::Char('r')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePreflight,
                "r must enter the rebase pre-flight stage"
            ),
            _ => panic!("expected the git ops overlay in the RebasePreflight stage"),
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Detached HEAD"),
            "a repo without a branch must show the detached-HEAD blocker, got:\n{}",
            rendered
        );

        // Enter must NOT proceed past a blocked pre-flight.
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePreflight,
                "Enter on a blocked pre-flight must stay put"
            ),
            _ => panic!("expected the overlay to stay on the blocked pre-flight"),
        }

        // Esc returns to the menu (overlay stays open).
        app.handle_key(key(KeyCode::Esc));
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Menu,
                "Esc from the pre-flight must return to the menu"
            ),
            _ => panic!("expected the git ops overlay on the Menu stage"),
        }
    }

    #[test]
    fn gitops_menu_esc_closes_to_dashboard() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));
        assert!(matches!(app.screen, Screen::GitOps(_)));

        app.handle_key(key(KeyCode::Esc));
        assert!(
            matches!(app.screen, Screen::Dashboard),
            "Esc should close the git ops menu back to the dashboard"
        );
    }

    #[test]
    fn gitops_menu_q_closes_to_dashboard() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));
        assert!(matches!(app.screen, Screen::GitOps(_)));

        app.handle_key(key(KeyCode::Char('q')));
        assert!(
            matches!(app.screen, Screen::Dashboard),
            "q should close the git ops menu back to the dashboard"
        );
    }

    #[test]
    fn gitops_menu_enter_fires_highlighted_item() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        // Move highlight to rebase (index 5), then Enter fires it. Rebase opens
        // its pre-flight synchronously (no worker), so this durably verifies
        // Enter dispatches the highlighted item (distinct from a letter-key
        // press) without depending on push/pull, which start network workers.
        use space::tui::screens::gitops::GitOpsStage;
        for _ in 0..5 {
            app.handle_key(key(KeyCode::Char('j')));
        }
        app.handle_key(key(KeyCode::Enter));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePreflight,
                "Enter on the highlighted rebase item must open its pre-flight"
            ),
            _ => panic!("expected the git ops overlay in the RebasePreflight stage"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "firing rebase from the menu must not start a worker"
        );
    }

    #[test]
    fn gitops_push_no_upstream_transitions_to_confirm() {
        use space::tui::screens::gitops::GitOpsStage;
        // workspace_with_repos uses a fake /tmp path, so has_upstream is false
        // and push must ask for confirmation before publishing the branch.
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        app.handle_key(key(KeyCode::Char('P')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::ConfirmPush,
                "push with no upstream must transition to ConfirmPush, not Running"
            ),
            _ => panic!("expected the git ops overlay to stay open on ConfirmPush"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "ConfirmPush must not start the push worker yet"
        );
    }

    #[test]
    fn gitops_confirm_push_y_starts_push_worker() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('P'))); // no upstream => ConfirmPush

        app.handle_key(key(KeyCode::Char('y')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Running,
                "confirming push with y must start the worker (Running stage)"
            ),
            _ => panic!("expected the git ops overlay to stay open in the Running stage"),
        }
        assert!(
            app.gitop_rx.is_some(),
            "confirming push must start the push worker (gitop_rx set)"
        );
    }

    #[test]
    fn gitops_esc_after_successful_op_refreshes_like_auto_close() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));

        // Simulate a finished, successful network op in the Running stage,
        // with stale repo-pane state that the close must refresh away.
        if let Screen::GitOps(ref mut st) = app.screen {
            st.stage = GitOpsStage::Running;
            st.finished = Some(true);
        } else {
            panic!("expected the git ops overlay");
        }
        app.expanded_repos.insert(0);

        // Story 12: Esc closes a successful op sooner than the ~3s timer —
        // and must leave the same refreshed state the auto-close produces.
        app.handle_key(key(KeyCode::Esc));

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "Esc after success must return to the dashboard"
        );
        assert!(
            app.expanded_repos.is_empty(),
            "closing a successful op early must refresh the repo pane exactly \
             like the auto-close path (stale expansion state must be gone)"
        );
    }

    #[test]
    fn gitops_confirm_push_enter_declines_matching_y_n_prompt() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('P'))); // no upstream => ConfirmPush

        // The prompt reads [y/N]: Enter is the default No and must never
        // publish the branch.
        app.handle_key(key(KeyCode::Enter));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Menu,
                "Enter must decline (default No) and return to the menu"
            ),
            _ => panic!("expected the git ops overlay to stay open on the Menu"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "Enter must not start the push worker"
        );
    }

    #[test]
    fn gitops_confirm_push_n_returns_to_menu() {
        use space::tui::screens::gitops::GitOpsStage;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('P'))); // no upstream => ConfirmPush

        app.handle_key(key(KeyCode::Char('n')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Menu,
                "declining push with n must return to the menu stage"
            ),
            _ => panic!("expected the git ops overlay to stay open on the Menu"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "declining push must not start any worker"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 5: Commit (synchronous local op)
    // -----------------------------------------------------------------------

    /// Build a TestEnv-backed app whose single workspace repo is a real git repo
    /// with one staged file, focused on the Right pane with the cursor on the
    /// repo row. `G` opens the git ops menu with `has_staged == true`.
    fn setup_gitops_staged_app(name: &str) -> (TestEnv, PathBuf, App) {
        let env = TestEnv::new();
        let repo_path = env.create_repo(name);

        // Create and stage a file so has_staged is true.
        std::fs::write(repo_path.join("staged.txt"), "content\n").unwrap();
        let out = std::process::Command::new("git")
            .args(["add", "staged.txt"])
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
                name: name.into(),
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
        app.focus = Pane::Right; // cursor_row = 0 is the repo row
        (env, repo_path, app)
    }

    /// Current HEAD sha of `repo`, or empty when HEAD is unborn.
    fn head_sha(repo: &std::path::Path) -> String {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Subject line of the HEAD commit of `repo`.
    fn head_subject(repo: &std::path::Path) -> String {
        let out = std::process::Command::new("git")
            .args(["log", "-1", "--pretty=%s"])
            .current_dir(repo)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn gitops_commit_c_transitions_to_committing_when_staged() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, _repo, mut app) = setup_gitops_staged_app("commit-stage-repo");

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('c')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Committing,
                "c with staged files must enter the Committing stage"
            ),
            _ => panic!("expected the git ops overlay in the Committing stage"),
        }
    }

    #[test]
    fn gitops_committing_empty_message_does_not_commit() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, repo, mut app) = setup_gitops_staged_app("commit-empty-repo");
        let head_before = head_sha(&repo);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Enter)); // empty message

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Committing,
                "Enter with an empty message must stay in the Committing stage"
            ),
            _ => panic!("expected the git ops overlay still in the Committing stage"),
        }
        assert_eq!(
            head_before,
            head_sha(&repo),
            "an empty commit message must not create a commit"
        );
    }

    #[test]
    fn gitops_committing_enter_commits_and_returns_to_dashboard() {
        let (_env, repo, mut app) = setup_gitops_staged_app("commit-ok-repo");
        let head_before = head_sha(&repo);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('c')));
        // Type the commit subject one char at a time, then Enter.
        for ch in "hello world".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Enter));

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "a successful commit must return to the dashboard"
        );
        assert_ne!(
            head_before,
            head_sha(&repo),
            "a successful commit must advance HEAD"
        );
        assert_eq!(
            head_subject(&repo),
            "hello world",
            "the new commit's subject must match the typed message"
        );
    }

    #[test]
    fn gitops_committing_esc_returns_to_menu() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, repo, mut app) = setup_gitops_staged_app("commit-esc-repo");
        let head_before = head_sha(&repo);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Esc));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Menu,
                "Esc in the Committing stage must return to the Menu stage"
            ),
            _ => panic!("expected the git ops overlay back on the Menu stage"),
        }
        assert_eq!(
            head_before,
            head_sha(&repo),
            "Esc in the Committing stage must not create a commit"
        );
    }

    #[test]
    fn gitops_committing_renders_staged_files_and_typed_message() {
        let (_env, _repo, mut app) = setup_gitops_staged_app("commit-render-repo");
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('c')));
        for ch in "wip".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }

        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("staged.txt"),
            "the Committing stage must list the staged file, got:\n{}",
            rendered
        );
        assert!(
            rendered.contains("wip"),
            "the Committing stage must show the typed message, got:\n{}",
            rendered
        );
    }

    // -----------------------------------------------------------------------
    // Phase 6: Log (synchronous read-only revwalk)
    // -----------------------------------------------------------------------

    /// Build a TestEnv-backed app whose single workspace repo is a real git
    /// repo with `extra_commits` empty commits layered on top of the init
    /// commit. Focused on the Right pane with the cursor on the repo row so
    /// `G` then `l` opens the Log stage.
    fn setup_gitops_log_app(name: &str, extra_commits: &[&str]) -> (TestEnv, PathBuf, App) {
        let env = TestEnv::new();
        let repo_path = env.create_repo(name);
        for msg in extra_commits {
            let out = std::process::Command::new("git")
                .args(["commit", "--allow-empty", "-m", msg])
                .current_dir(&repo_path)
                .output()
                .expect("git commit failed to run");
            assert!(
                out.status.success(),
                "git commit failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let ws = Workspace {
            name: "test-ws".into(),
            path: env.workspaces_dir.clone(),
            repos: vec![WorkspaceRepo {
                name: name.into(),
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
        app.focus = Pane::Right; // cursor_row = 0 is the repo row
        (env, repo_path, app)
    }

    #[test]
    fn gitops_log_l_transitions_to_log_and_shows_commit_subject() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, _repo, mut app) = setup_gitops_log_app("log-open-repo", &["logtest-subject"]);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('l')));

        match &app.screen {
            Screen::GitOps(st) => {
                assert_eq!(st.stage, GitOpsStage::Log, "l must enter the Log stage")
            }
            _ => panic!("expected the git ops overlay in the Log stage"),
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("logtest-subject"),
            "the Log stage must show a known commit subject, got:\n{}",
            rendered
        );
    }

    #[test]
    fn gitops_log_esc_returns_to_menu() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, _repo, mut app) = setup_gitops_log_app("log-esc-repo", &["only-commit"]);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('l')));
        app.handle_key(key(KeyCode::Esc));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Menu,
                "Esc in the Log stage must return to the Menu stage"
            ),
            _ => panic!("expected the git ops overlay back on the Menu stage"),
        }
    }

    #[test]
    fn gitops_log_j_scrolls_down() {
        // Many more commits than can fit on screen, so a down press has room to
        // move the scroll offset.
        let msgs: Vec<String> = (0..40).map(|i| format!("commit-{}", i)).collect();
        let msg_refs: Vec<&str> = msgs.iter().map(String::as_str).collect();
        let (_env, _repo, mut app) = setup_gitops_log_app("log-scroll-repo", &msg_refs);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('l')));

        let before = match &app.screen {
            Screen::GitOps(st) => st.log_scroll,
            _ => panic!("expected the Log stage"),
        };
        app.handle_key(key(KeyCode::Char('j')));
        let after = match &app.screen {
            Screen::GitOps(st) => st.log_scroll,
            _ => panic!("expected the Log stage"),
        };

        assert_eq!(before, 0, "log_scroll should start at the top");
        assert!(
            after > before,
            "j must increase log_scroll (was {}, now {})",
            before,
            after
        );
    }

    #[test]
    fn gitops_log_over_scroll_with_few_commits_does_not_panic_or_blank() {
        // Fewer commits than the viewport: the down-scroll clamp must never
        // underflow (usize) or reveal a blank screenful past the last commit.
        let (_env, _repo, mut app) = setup_gitops_log_app("log-few-repo", &["only-extra"]);
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('l')));

        // Hammer Down/End well past the number of commits.
        for _ in 0..50 {
            app.handle_key(key(KeyCode::Char('j')));
        }
        app.handle_key(key(KeyCode::End));

        // Render at a tall viewport (many more rows than commits): must not
        // panic, and a real commit must still be visible (no blank over-scroll).
        let rendered = render_text(&app, 80, 40);
        assert!(
            rendered.contains("only-extra") || rendered.contains("init"),
            "the log must still show a commit after over-scrolling, got:\n{}",
            rendered
        );
    }

    // -----------------------------------------------------------------------
    // Item 7: Safe rebase flow (pre-flight → target picker → confirm → run)
    // -----------------------------------------------------------------------

    #[test]
    fn gitops_rebase_clean_repo_preflight_advances_to_target_picker() {
        use space::tui::screens::gitops::GitOpsStage;
        // setup_gitops_log_app with no extra commits = a clean real repo.
        let (_env, _repo, mut app) = setup_gitops_log_app("rebase-clean-repo", &[]);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));

        match &app.screen {
            Screen::GitOps(st) => {
                assert_eq!(
                    st.stage,
                    GitOpsStage::RebasePreflight,
                    "r must enter the rebase pre-flight stage"
                );
                assert!(
                    st.rebase_block.is_none(),
                    "a clean repo must not be blocked, got {:?}",
                    st.rebase_block
                );
            }
            _ => panic!("expected the git ops overlay in the RebasePreflight stage"),
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Working tree clean"),
            "a clean pre-flight must say so, got:\n{}",
            rendered
        );

        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePickTarget,
                "Enter on a clean pre-flight must open the target picker"
            ),
            _ => panic!("expected the git ops overlay in the RebasePickTarget stage"),
        }
        // The picker title must state the rebase consequence, not the generic
        // switch-branch wording.
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Rebase onto"),
            "the target picker title must say Rebase onto, got:\n{}",
            rendered
        );
    }

    #[test]
    fn gitops_rebase_dirty_repo_blocks_preflight() {
        use space::tui::screens::gitops::GitOpsStage;
        // setup_gitops_staged_app leaves a staged file → the tree is dirty.
        let (_env, _repo, mut app) = setup_gitops_staged_app("rebase-dirty-repo");

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));

        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("uncommitted changes"),
            "a dirty tree must show the uncommitted-changes blocker, got:\n{}",
            rendered
        );
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePreflight,
                "Enter on a dirty pre-flight must stay put"
            ),
            _ => panic!("expected the overlay to stay on the blocked pre-flight"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "a blocked pre-flight must never start a worker"
        );
    }

    #[test]
    fn gitops_rebase_full_flow_y_starts_worker() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, _repo, mut app) = setup_gitops_log_app("rebase-flow-repo", &[]);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Enter)); // pre-flight -> picker
        app.handle_key(key(KeyCode::Enter)); // pick the only branch (main)

        match &app.screen {
            Screen::GitOps(st) => {
                assert_eq!(
                    st.stage,
                    GitOpsStage::RebaseConfirm,
                    "picking a target must open the confirm stage"
                );
                assert_eq!(
                    st.rebase_onto.as_deref(),
                    Some("main"),
                    "the picked target must be stored"
                );
            }
            _ => panic!("expected the git ops overlay in the RebaseConfirm stage"),
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("[y/N]"),
            "the confirm stage must show the [y/N] prompt, got:\n{}",
            rendered
        );
        // Substring kept short: the warning line wraps at the dialog's minimum
        // width, so a longer phrase would span the line break and never match.
        assert!(
            rendered.contains("push will be rejected"),
            "the confirm stage must warn about the post-rebase push rejection, got:\n{}",
            rendered
        );

        app.handle_key(key(KeyCode::Char('y')));
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::Running,
                "confirming with y must start the worker (Running stage)"
            ),
            _ => panic!("expected the git ops overlay in the Running stage"),
        }
        assert!(
            app.gitop_rx.is_some(),
            "confirming the rebase must start the git-ops worker (gitop_rx set)"
        );
        // The Running header must use the correct progressive form ("Rebasing",
        // not "Rebaseing" from naive label + "ing" concatenation).
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Rebasing") && !rendered.contains("Rebaseing"),
            "the rebase Running header should say Rebasing, got:\n{}",
            rendered
        );
    }

    #[test]
    fn gitops_rebase_worker_runs_to_completion_and_auto_closes() {
        let (_env, _repo, mut app) = setup_gitops_log_app("rebase-done-repo", &[]);

        // Drive to the Running stage exactly like
        // gitops_rebase_full_flow_y_starts_worker: open git-ops, enter rebase,
        // clear the clean pre-flight, pick the only branch (main), confirm with y.
        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Enter)); // pre-flight -> picker
        app.handle_key(key(KeyCode::Enter)); // pick the only branch (main)
        app.handle_key(key(KeyCode::Char('y'))); // confirm -> Running, spawns worker
        assert!(
            app.gitop_rx.is_some(),
            "confirming the rebase must start the git-ops worker (gitop_rx set)"
        );

        // Drive the worker to Done by polling each frame in a bounded loop,
        // mirroring drain_sync. Rebasing main onto main is UpToDate, so the
        // worker reports Done { success: true } and poll records finished +
        // close_at (a 3s auto-close timer, which cannot elapse mid-loop).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.poll_gitop_result();
            // Break as soon as Done lands (finished becomes Some). Guard against
            // the screen no longer being GitOps so the loop can never spin after
            // an unexpected auto-close.
            let finished = if let Screen::GitOps(st) = &app.screen {
                st.finished
            } else {
                None
            };
            if finished.is_some() || !matches!(app.screen, Screen::GitOps(_)) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "rebase worker did not finish within timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        // The worker finished successfully (an up-to-date rebase counts as
        // success), and the overlay is still open in the Running stage.
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.finished,
                Some(true),
                "the rebase worker must report success (finished = Some(true))"
            ),
            _ => panic!("expected the git ops overlay still open after the worker finished"),
        }

        // The success completion header renders (op label "Rebase" + " complete").
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Rebase complete"),
            "a successful rebase must render the completion header, got:\n{}",
            rendered
        );

        // Exercise the auto-close path without waiting the real ~3s timer: move
        // close_at into the past, then poll once more.
        if let Screen::GitOps(ref mut st) = app.screen {
            st.close_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        } else {
            panic!("expected the git ops overlay before the auto-close poll");
        }
        app.poll_gitop_result();

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "the elapsed success timer must auto-close back to the dashboard"
        );
        assert!(
            app.gitop_rx.is_none(),
            "auto-close must clear the git-ops worker receiver"
        );
    }

    #[test]
    fn gitops_rebase_confirm_enter_declines_back_to_picker() {
        use space::tui::screens::gitops::GitOpsStage;
        let (_env, _repo, mut app) = setup_gitops_log_app("rebase-decline-repo", &[]);

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::Enter)); // pre-flight -> picker
        app.handle_key(key(KeyCode::Enter)); // pick main -> confirm

        // The prompt reads [y/N]: Enter is the default No and must never
        // rewrite history.
        app.handle_key(key(KeyCode::Enter));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePickTarget,
                "Enter must decline (default No) and return to the target picker"
            ),
            _ => panic!("expected the git ops overlay back on the target picker"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "declining the rebase must not start any worker"
        );
    }

    #[test]
    fn gitops_rebase_preflight_blocks_genuine_detached_head() {
        use space::tui::screens::gitops::GitOpsStage;
        // A REAL repo detached on disk after setup: start_rebase reads HEAD
        // live via is_on_branch, so it must block on the genuine detached-HEAD
        // case. This complements
        // gitops_menu_r_opens_rebase_preflight_blocked_on_fake_repo, which only
        // exercises an unopenable /tmp path (is_on_branch returns false because
        // Repository::open fails), never a real detached HEAD.
        let (_env, repo_path, mut app) = setup_gitops_log_app("rebase-detached-repo", &[]);

        // Detach HEAD in the real repo before pressing r.
        let out = std::process::Command::new("git")
            .args(["checkout", "--detach"])
            .current_dir(&repo_path)
            .output()
            .expect("git checkout --detach failed to run");
        assert!(
            out.status.success(),
            "git checkout --detach failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        app.handle_key(key(KeyCode::Char('G')));
        app.handle_key(key(KeyCode::Char('r')));

        match &app.screen {
            Screen::GitOps(st) => assert_eq!(
                st.stage,
                GitOpsStage::RebasePreflight,
                "r must enter the rebase pre-flight stage"
            ),
            _ => panic!("expected the git ops overlay in the RebasePreflight stage"),
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("Detached HEAD"),
            "a genuine detached HEAD must show the detached-HEAD blocker, got:\n{}",
            rendered
        );

        // A blocked pre-flight must never advance to the target picker.
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::GitOps(st) => {
                assert_eq!(
                    st.stage,
                    GitOpsStage::RebasePreflight,
                    "Enter on a blocked pre-flight must stay put"
                );
                assert_ne!(
                    st.stage,
                    GitOpsStage::RebasePickTarget,
                    "a blocked pre-flight must not reach the target picker"
                );
            }
            _ => panic!("expected the overlay to stay on the blocked pre-flight"),
        }
        assert!(
            app.gitop_rx.is_none(),
            "a blocked pre-flight must never start a worker"
        );
    }
}

// ---- 1.5 Sync report ----

mod sync_report_tests {
    use super::*;
    use space::core::workspace::{FetchOutcome, SyncOutcome};
    use space::tui::screens::add::{AddStage, AddState};
    use space::tui::screens::create::{CreateStage, CreateState};
    use space::tui::screens::sync_report::SyncReport;
    use std::path::Path;

    fn sr_git(args: &[&str], dir: &Path) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed:\n{}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn sr_git_setup(dir: &Path) {
        sr_git(&["config", "user.email", "t@local"], dir);
        sr_git(&["config", "user.name", "T"], dir);
        sr_git(&["config", "commit.gpgsign", "false"], dir);
    }

    /// A repo under `repos_dir/<name>` cloned from a bare origin, with `main`
    /// checked out and a `dev` branch that is one commit behind `origin/dev`,
    /// so a sync fast-forwards `dev`.
    fn behind_repo(env: &TestEnv, name: &str) -> PathBuf {
        let fixtures = env.dir.path().join("fixtures").join(name);
        std::fs::create_dir_all(&fixtures).unwrap();
        let bare = fixtures.join("origin.git");
        sr_git(
            &["init", "--bare", "-b", "main", &bare.to_string_lossy()],
            &fixtures,
        );
        let local = env.repos_dir.join(name);
        sr_git(
            &[
                "clone",
                "-q",
                &bare.to_string_lossy(),
                &local.to_string_lossy(),
            ],
            &fixtures,
        );
        sr_git_setup(&local);
        sr_git(&["commit", "--allow-empty", "-m", "init"], &local);
        sr_git(&["push", "-q", "-u", "origin", "main"], &local);
        sr_git(&["checkout", "-q", "-b", "dev"], &local);
        sr_git(&["commit", "--allow-empty", "-m", "dev-init"], &local);
        sr_git(&["push", "-q", "-u", "origin", "dev"], &local);
        sr_git(&["checkout", "-q", "main"], &local);

        let helper = fixtures.join("helper");
        sr_git(
            &[
                "clone",
                "-q",
                &bare.to_string_lossy(),
                &helper.to_string_lossy(),
            ],
            &fixtures,
        );
        sr_git_setup(&helper);
        sr_git(&["checkout", "-q", "-b", "dev", "origin/dev"], &helper);
        sr_git(&["commit", "--allow-empty", "-m", "dev-remote"], &helper);
        sr_git(&["push", "-q", "origin", "dev"], &helper);
        local
    }

    /// Poll until the active screen's sync report is done, without pressing Enter.
    fn wait_for_report(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.poll_sync_result();
            let done = match &app.screen {
                Screen::CreateWorkspace(st) => st.report.done,
                Screen::AddRepos(st) => st.report.done,
                _ => panic!("expected the create or add screen"),
            };
            if done {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sync worker did not complete within timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn ok_outcome() -> SyncOutcome {
        SyncOutcome {
            fetch: FetchOutcome::Ok,
            forwarded: vec![],
            skipped: vec![],
        }
    }

    fn failed_outcome(stderr: &str) -> SyncOutcome {
        SyncOutcome {
            fetch: FetchOutcome::Failed {
                exit_code: Some(128),
                stderr: stderr.to_string(),
            },
            forwarded: vec![],
            skipped: vec![],
        }
    }

    fn create_state_on_report(report: SyncReport) -> CreateState {
        let mut st = CreateState::new(vec![], vec![]);
        st.ws_name = tui_input::Input::default().with_value("ws".to_string());
        st.stage = CreateStage::Syncing;
        st.report = report;
        st
    }

    #[test]
    fn mixed_run_shows_rows_title_cursor_on_failure_and_enter_continues() {
        let env = TestEnv::new();
        let good = behind_repo(&env, "payments-api");
        let bad = env.create_repo("web-console"); // no remote: the fetch fails
        let config = config_from_env(&env);
        let mut app = test_app_with_config(config, vec![], vec![good.clone(), bad.clone()]);

        app.handle_key(key(KeyCode::Char('c')));
        if let Screen::CreateWorkspace(ref mut st) = app.screen {
            st.ws_name = tui_input::Input::default().with_value("ws".to_string());
            st.stage = CreateStage::PickRepos;
            st.picker.toggle_highlighted();
            st.picker.move_down();
            st.picker.toggle_highlighted();
        }
        app.handle_key(key(KeyCode::Enter));
        wait_for_report(&mut app);

        let Screen::CreateWorkspace(st) = &app.screen else {
            panic!("expected CreateWorkspace screen");
        };
        assert_eq!(st.stage, CreateStage::Syncing, "Done pauses on the report");
        assert_eq!(st.report.rows.len(), 2);
        let good_idx = st
            .report
            .rows
            .iter()
            .position(|r| r.name == "payments-api")
            .unwrap();
        let bad_idx = 1 - good_idx;
        assert_eq!(st.report.rows[good_idx].outcome_label(), "fast-forwarded");
        assert_eq!(st.report.rows[good_idx].detail().0, "dev");
        assert_eq!(st.report.rows[bad_idx].outcome_label(), "fetch failed");
        assert_eq!(st.report.title(), "Sync report \u{b7} 1 ok, 1 failed");
        assert_eq!(
            st.report.cursor, bad_idx,
            "cursor lands on the first failed row"
        );

        let rendered = render_text(&app, 100, 30);
        for needle in [
            "Sync report \u{b7} 1 ok, 1 failed",
            "\u{25b6} \u{2717} web-console",
            "fast-forwarded  dev",
            "fetch failed (git exit 128) \u{b7} branch picker will use local refs",
            "fatal: 'origin' does not appear to be a git repository",
            "ENTER continue \u{b7} ESC back",
        ] {
            assert!(
                rendered.contains(needle),
                "report must show {:?}, got:\n{}",
                needle,
                rendered
            );
        }

        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(
                st.stage,
                CreateStage::PickBranchStrategy,
                "Enter continues to the branch picker"
            ),
            _ => panic!("expected CreateWorkspace screen"),
        }
    }

    #[test]
    fn add_flow_esc_returns_to_pick_repos_with_selection_and_query_intact() {
        let env = TestEnv::new();
        let repo = env.create_repo("alpha");
        let config = config_from_env(&env);
        let mut app = test_app_with_config(config, vec![], vec![repo.clone()]);
        app.screen = Screen::AddRepos(AddState::new("ws".to_string(), vec![repo], vec![]));

        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::AddRepos(st) => assert_eq!(st.stage, AddStage::Syncing),
            _ => panic!("expected AddRepos screen"),
        }
        wait_for_report(&mut app);

        app.handle_key(key(KeyCode::Esc));
        let Screen::AddRepos(st) = &app.screen else {
            panic!("expected AddRepos screen");
        };
        assert_eq!(
            st.stage,
            AddStage::PickRepos,
            "Esc returns to the repo picker"
        );
        assert_eq!(st.picker.input.value(), "a", "the query survives");
        assert_eq!(
            st.picker.confirmed_items().len(),
            1,
            "the selection survives"
        );

        // Re-confirming re-syncs the same selection into a fresh report.
        app.handle_key(key(KeyCode::Enter));
        wait_for_report(&mut app);
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::AddRepos(st) => assert_eq!(
                st.stage,
                AddStage::PickBranchStrategy,
                "Enter on the finished report continues"
            ),
            _ => panic!("expected AddRepos screen"),
        }
    }

    #[test]
    fn enter_is_ignored_and_esc_cancels_while_running() {
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a"), PathBuf::from("/r/b")]);
        report.started(0);
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 80, 24);
        for needle in [
            "Sync report \u{b7} 0 of 2",
            "\u{25cf} a",
            "syncing\u{2026}",
            "fetching origin\u{2026}",
            "ESC cancel",
        ] {
            assert!(
                rendered.contains(needle),
                "running report must show {:?}, got:\n{}",
                needle,
                rendered
            );
        }

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Down));
        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(
                    st.stage,
                    CreateStage::Syncing,
                    "Enter is ignored until Done"
                );
                assert_eq!(st.report.cursor, 0, "cursor keys are ignored until Done");
            }
            _ => panic!("expected CreateWorkspace screen"),
        }

        app.handle_key(key(KeyCode::Esc));
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(
                st.stage,
                CreateStage::PickRepos,
                "Esc while running cancels back to the repo picker"
            ),
            _ => panic!("expected CreateWorkspace screen"),
        }
    }

    #[test]
    fn all_failed_shows_notice_and_continue_anyway() {
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a"), PathBuf::from("/r/b")]);
        report.finished(0, failed_outcome("fatal: no way"));
        report.finished(1, failed_outcome("fatal: nor this"));
        report.finish();
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 100, 30);
        for needle in [
            "Sync report \u{b7} 0 ok, 2 failed",
            "Nothing was fetched. You can still continue;",
            "ENTER continue anyway",
        ] {
            assert!(
                rendered.contains(needle),
                "all-failed report must show {:?}, got:\n{}",
                needle,
                rendered
            );
        }
        app.handle_key(key(KeyCode::Enter));
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(
                st.stage,
                CreateStage::PickBranchStrategy,
                "continuing after an all-failed run is allowed"
            ),
            _ => panic!("expected CreateWorkspace screen"),
        }
    }

    #[test]
    fn many_repos_scroll_to_keep_cursor_visible() {
        let mut app = test_app(vec![], vec![]);
        let repos: Vec<PathBuf> = (0..14)
            .map(|i| PathBuf::from(format!("/r/repo{:02}", i)))
            .collect();
        let mut report = SyncReport::new(&repos);
        for i in 0..14 {
            report.finished(i, ok_outcome());
        }
        report.finish();
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        // 80x24: the dialog caps at 19 rows, 12 list rows fit.
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("repo00"),
            "top of the list:\n{}",
            rendered
        );
        assert!(
            rendered.contains("repo11"),
            "12th row visible:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("repo13"),
            "the last row is reachable only by scrolling:\n{}",
            rendered
        );
        assert!(
            rendered.contains(
                "\u{2191}\u{2193} select \u{b7} PgUp/PgDn page \u{b7} ENTER continue \u{b7} ESC back"
            ),
            "the rows prefix drops on a 60-column dialog:\n{}",
            rendered
        );
        let wide = render_text(&app, 120, 24);
        assert!(
            wide.contains("rows 1\u{2013}12 of 14 \u{b7}"),
            "a wide dialog shows the rows prefix:\n{}",
            wide
        );

        app.handle_key(key(KeyCode::End));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("\u{25b6} \u{2713} repo13"),
            "End reaches the last row:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("repo00"),
            "the list scrolled:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("repo01"),
            "the list scrolled:\n{}",
            rendered
        );

        app.handle_key(key(KeyCode::PageUp));
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(st.report.cursor, 3),
            _ => panic!("expected CreateWorkspace screen"),
        }
        app.handle_key(key(KeyCode::Home));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("\u{25b6} \u{2713} repo00"),
            "Home:\n{}",
            rendered
        );
    }

    #[test]
    fn creating_log_tail_follows_and_end_resumes() {
        let mut app = test_app(vec![], vec![]);
        let mut st = CreateState::new(vec![], vec![]);
        st.stage = CreateStage::Creating;
        st.progress = (1..=30).map(|i| format!("step {:02}", i)).collect();
        st.error = Some("boom".to_string());
        app.screen = Screen::CreateWorkspace(st);

        // 80x24: the dialog caps at 19 rows, 16 log lines fit.
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 30"),
            "tail-follow shows the newest line:\n{}",
            rendered
        );
        assert!(rendered.contains("step 15"), "16 lines fit:\n{}", rendered);
        assert!(
            !rendered.contains("step 14"),
            "earlier lines are scrolled off:\n{}",
            rendered
        );
        assert!(rendered.contains("Error: boom"), "footer:\n{}", rendered);

        app.handle_key(key(KeyCode::Up));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 14"),
            "Up scrolls back one line:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("step 30"),
            "Up detaches from the tail:\n{}",
            rendered
        );

        // New lines arriving while detached leave the view where it is.
        if let Screen::CreateWorkspace(ref mut st) = app.screen {
            st.progress.push("step 31".to_string());
        }
        let rendered = render_text(&app, 80, 24);
        assert!(rendered.contains("step 14") && !rendered.contains("step 31"));

        app.handle_key(key(KeyCode::End));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 31"),
            "End resumes following:\n{}",
            rendered
        );
        if let Screen::CreateWorkspace(ref mut st) = app.screen {
            st.progress.push("step 32".to_string());
        }
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 32"),
            "following tracks new lines:\n{}",
            rendered
        );

        app.handle_key(key(KeyCode::Home));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 01"),
            "Home shows the first line:\n{}",
            rendered
        );

        // Esc still leaves the stage with the error status.
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::Dashboard));
    }

    #[test]
    fn add_flow_creating_log_scrolls_too() {
        let mut app = test_app(vec![], vec![]);
        let mut st = AddState::new("ws".to_string(), vec![], vec![]);
        st.stage = AddStage::Creating;
        st.progress = (1..=30).map(|i| format!("step {:02}", i)).collect();
        st.error = Some("boom".to_string());
        app.screen = Screen::AddRepos(st);

        let rendered = render_text(&app, 80, 24);
        assert!(rendered.contains("step 30") && !rendered.contains("step 14"));
        app.handle_key(key(KeyCode::PageUp));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("step 05") && !rendered.contains("step 30"),
            "PgUp:\n{}",
            rendered
        );
        app.handle_key(key(KeyCode::End));
        let rendered = render_text(&app, 80, 24);
        assert!(rendered.contains("step 30"), "End resumes:\n{}", rendered);
    }

    #[test]
    fn small_report_with_long_pane_fits_below_the_cap() {
        // The spec's longest known case: ssh, five stderr lines, one blank,
        // 7 pane lines with the header and status line. On a 24-row terminal
        // a two-repo report is well below the cap, so nothing may be cut.
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/ssh"), PathBuf::from("/r/ok")]);
        report.finished(0, failed_outcome("line1\nline2\n\nline4\nline5\n"));
        report.finished(1, ok_outcome());
        report.finish();
        assert_eq!(report.cursor, 0);
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 80, 24);
        for needle in ["line1", "line2", "line4", "line5"] {
            assert!(
                rendered.contains(needle),
                "{} must be visible:\n{}",
                needle,
                rendered
            );
        }
        assert!(
            !rendered.contains("more lines"),
            "nothing may be cut below the cap:\n{}",
            rendered
        );
    }

    #[test]
    fn running_report_keeps_newest_row_visible() {
        let mut app = test_app(vec![], vec![]);
        let repos: Vec<PathBuf> = (0..14)
            .map(|i| PathBuf::from(format!("/r/repo{:02}", i)))
            .collect();
        let mut report = SyncReport::new(&repos);
        for i in 0..13 {
            report.started(i);
            report.finished(i, ok_outcome());
        }
        report.started(13);
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("\u{25b6} \u{25cf} repo13") && rendered.contains("syncing\u{2026}"),
            "the row being synced is on screen:\n{}",
            rendered
        );
        assert!(
            !rendered.contains("repo00"),
            "earlier rows scrolled off:\n{}",
            rendered
        );
        assert!(
            rendered.contains("Sync report \u{b7} 13 of 14"),
            "title:\n{}",
            rendered
        );
    }

    #[test]
    fn narrow_terminal_keeps_dialog_and_footer_inside_the_frame() {
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a")]);
        report.finished(0, failed_outcome("fatal: x"));
        report.finish();
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));
        let rendered = render_text(&app, 50, 12);
        assert!(
            rendered.contains("ENTER continue anyway \u{b7} ESC back"),
            "ENTER and ESC never drop:\n{}",
            rendered
        );
        assert!(max_rendered_width(&rendered) <= 50);
    }
}
