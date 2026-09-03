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
        rendered.contains("y delete"),
        "expected delete actions to remain visible in rendered popup, got:\n{}",
        rendered
    );
    assert!(
        rendered.contains("Esc/n/Enter cancel"),
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
        rendered.contains("y delete"),
        "expected delete action on a wrapped footer line, got:\n{}",
        rendered
    );
    assert!(
        rendered.contains("Esc/n/Enter cancel"),
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
    // Repo search is pane-gated to the repos pane (the workspaces pane filters spaces).
    app.focus = Pane::Right;

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
fn esc_on_left_pane_is_a_noop() {
    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    assert_eq!(app.focus, Pane::Left);

    app.handle_key(key(KeyCode::Esc));

    assert!(!app.should_quit, "Esc on the left pane must not quit");
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "Esc on the left pane must leave the app on the dashboard"
    );
    assert_eq!(app.focus, Pane::Left, "focus must be unchanged");
}

#[test]
fn q_still_quits_after_esc_is_neutered() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.should_quit);
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.should_quit, "q remains the documented way out");
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
    assert!(app.help.is_some(), "expected the help overlay to be open");
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "help is an overlay layer: the screen beneath it never changes"
    );
}

#[test]
fn help_esc_closes_the_overlay() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    assert!(app.help.is_some());

    app.handle_key(key(KeyCode::Esc));
    assert!(app.help.is_none(), "expected the overlay closed after Esc");
    assert!(matches!(app.screen, Screen::Dashboard));
}

#[test]
fn help_q_closes_the_overlay() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));
    app.handle_key(key(KeyCode::Char('q')));
    assert!(app.help.is_none(), "expected the overlay closed after q");
    assert!(
        !app.should_quit,
        "q closes the overlay, it does not fall through and quit the app"
    );
}

#[test]
fn help_question_mark_and_f1_both_close_the_overlay() {
    for closer in [KeyCode::Char('?'), KeyCode::F(1)] {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        app.handle_key(key(closer));
        assert!(
            app.help.is_none(),
            "{:?} must toggle the overlay off",
            closer
        );
    }
}

#[test]
fn help_other_keys_are_noop() {
    let mut app = test_app(vec![], vec![]);
    app.handle_key(key(KeyCode::Char('?')));

    // Keys that must not close help, and must not reach the screen beneath.
    for code in [
        KeyCode::Char('c'),
        KeyCode::Char('d'),
        KeyCode::Enter,
        KeyCode::Tab,
    ] {
        app.handle_key(key(code));
        assert!(
            app.help.is_some(),
            "{:?} should not close the help overlay",
            code
        );
        assert!(
            matches!(app.screen, Screen::Dashboard),
            "{:?} must not reach the screen beneath the overlay",
            code
        );
    }
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

// ---- 1.1 Rescan the repo list inside the picker ----

mod rescan_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    use space::tui::actions::StatusKind;
    use std::collections::BTreeSet;
    use std::sync::{LazyLock, Mutex};

    /// The rescan saves the repo cache to `SpaceConfig::cache_path()`, which
    /// reads SPACE_CONFIG_DIR. Point it at the TestEnv so the tests never touch
    /// the real user cache, and serialise them because the env var is
    /// process-global (same pattern as mcp_test.rs).
    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct EnvGuard;
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("SPACE_CONFIG_DIR") };
        }
    }

    fn with_config_dir<F: FnOnce(&TestEnv)>(f: F) {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let env = TestEnv::new();
        unsafe { std::env::set_var("SPACE_CONFIG_DIR", &env.config_dir) };
        let _guard = EnvGuard;
        f(&env);
    }

    fn ctrl_r() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)
    }

    fn item_paths(picker: &space::tui::widgets::fuzzy_picker::FuzzyPicker) -> BTreeSet<PathBuf> {
        picker
            .all_items
            .iter()
            .map(|i| i.full_path.clone())
            .collect()
    }

    fn toggled_paths(picker: &space::tui::widgets::fuzzy_picker::FuzzyPicker) -> BTreeSet<PathBuf> {
        picker
            .toggled
            .iter()
            .map(|&i| picker.all_items[i].full_path.clone())
            .collect()
    }

    #[test]
    fn ctrl_r_in_create_picker_shows_new_repo_and_keeps_toggle_and_query() {
        with_config_dir(|env| {
            let alpha = env.create_repo("alpha");
            let beta = env.create_repo("beta");
            let config = config_from_env(env);
            let mut app = test_app_with_config(config, vec![], vec![alpha.clone(), beta.clone()]);
            app.cursor_row = 3;

            open_create_picker(&mut app, "ws");
            // Toggle the top row (alpha, cache order), then type a query that
            // still matches every repo in this test.
            app.handle_key(key(KeyCode::Tab));
            app.handle_key(key(KeyCode::Char('a')));

            let gamma = env.create_repo("gamma");
            app.handle_key(ctrl_r());

            let Screen::CreateWorkspace(st) = &app.screen else {
                panic!("Ctrl-R must keep the create flow open");
            };
            assert_eq!(
                st.stage,
                space::tui::screens::create::CreateStage::PickRepos
            );
            assert_eq!(
                item_paths(&st.picker),
                BTreeSet::from([alpha.clone(), beta.clone(), gamma.clone()]),
                "the new repo must appear after the rescan"
            );
            assert_eq!(st.picker.query(), "a", "the query must be intact");
            assert_eq!(
                toggled_paths(&st.picker),
                BTreeSet::from([alpha.clone()]),
                "the toggle must survive the rescan"
            );
            assert!(
                st.picker
                    .filtered
                    .iter()
                    .any(|&i| st.picker.all_items[i].full_path == gamma),
                "the new repo must be visible under the kept query"
            );
            assert_eq!(
                app.status_message.as_deref(),
                Some("Rescanned: 3 repos, 1 new")
            );
            assert_eq!(app.status_kind, StatusKind::Success);
            assert_eq!(
                app.repos_cache.iter().cloned().collect::<BTreeSet<_>>(),
                BTreeSet::from([alpha, beta, gamma]),
                "the app repo list must be replaced by the rescan"
            );
            assert_eq!(
                app.cursor_row, 3,
                "a picker rescan must not touch the dashboard cursor"
            );
            assert!(
                env.config_dir.join("repos.cache").exists(),
                "the rescan must save the cache"
            );
        });
    }

    #[test]
    fn ctrl_r_drops_toggle_for_removed_repo_with_warning() {
        with_config_dir(|env| {
            let alpha = env.create_repo("alpha");
            let beta = env.create_repo("beta");
            let config = config_from_env(env);
            let mut app = test_app_with_config(config, vec![], vec![alpha.clone(), beta.clone()]);

            open_create_picker(&mut app, "ws");
            app.handle_key(key(KeyCode::Tab)); // toggles alpha

            std::fs::remove_dir_all(&alpha).unwrap();
            app.handle_key(ctrl_r());

            let Screen::CreateWorkspace(st) = &app.screen else {
                panic!("Ctrl-R must keep the create flow open");
            };
            assert_eq!(item_paths(&st.picker), BTreeSet::from([beta]));
            assert!(
                st.picker.toggled.is_empty(),
                "a toggled repo that is gone must be dropped"
            );
            assert_eq!(
                app.status_message.as_deref(),
                Some("Rescanned: 1 repo, 0 new, 1 selected repo no longer found")
            );
            assert_eq!(app.status_kind, StatusKind::Warning);
        });
    }

    #[test]
    fn ctrl_r_in_add_picker_keeps_space_repos_excluded() {
        with_config_dir(|env| {
            let alpha = env.create_repo("alpha");
            let beta = env.create_repo("beta");
            let config = config_from_env(env);
            let workspaces = vec![Workspace {
                name: "ws".to_string(),
                path: env.workspaces_dir.join("ws"),
                repos: vec![WorkspaceRepo {
                    name: "alpha".to_string(),
                    path: env.workspaces_dir.join("ws").join("alpha"),
                    branch: "main".to_string(),
                    status: Default::default(),
                    ahead: 0,
                    behind: 0,
                }],
            }];
            let mut app =
                test_app_with_config(config, workspaces, vec![alpha.clone(), beta.clone()]);

            app.handle_key(key(KeyCode::Char('a')));
            match &app.screen {
                Screen::AddRepos(st) => {
                    assert_eq!(item_paths(&st.picker), BTreeSet::from([beta.clone()]))
                }
                _ => panic!("expected AddRepos screen"),
            }

            let gamma = env.create_repo("gamma");
            app.handle_key(ctrl_r());

            let Screen::AddRepos(st) = &app.screen else {
                panic!("Ctrl-R must keep the add flow open");
            };
            assert_eq!(st.stage, space::tui::screens::add::AddStage::PickRepos);
            assert_eq!(
                item_paths(&st.picker),
                BTreeSet::from([beta, gamma]),
                "repos already in the space must stay excluded; the new repo must appear"
            );
            // alpha is still in the repo list; only the picker hides it.
            assert_eq!(
                app.status_message.as_deref(),
                Some("Rescanned: 3 repos, 1 new")
            );
            assert_eq!(app.status_kind, StatusKind::Success);
        });
    }

    #[test]
    fn ctrl_r_in_add_picker_leaves_flow_when_space_is_gone() {
        with_config_dir(|env| {
            let alpha = env.create_repo("alpha");
            let config = config_from_env(env);
            let workspaces = vec![Workspace {
                name: "ws".to_string(),
                path: env.workspaces_dir.join("ws"),
                repos: vec![],
            }];
            let mut app = test_app_with_config(config, workspaces, vec![alpha]);

            app.handle_key(key(KeyCode::Char('a')));
            assert!(matches!(app.screen, Screen::AddRepos(_)));

            // The space disappears while the picker is open.
            app.workspaces.clear();
            app.handle_key(ctrl_r());

            assert!(
                matches!(app.screen, Screen::Dashboard),
                "a vanished space must leave the add flow"
            );
            assert_eq!(
                app.status_message.as_deref(),
                Some("Space 'ws' no longer exists")
            );
            assert_eq!(app.status_kind, StatusKind::Error);
        });
    }

    #[test]
    fn dashboard_r_reports_rescanned_with_new_count() {
        with_config_dir(|env| {
            let alpha = env.create_repo("alpha");
            let beta = env.create_repo("beta");
            let config = config_from_env(env);
            let mut app = test_app_with_config(config, vec![], vec![alpha.clone()]);

            app.handle_key(key(KeyCode::Char('r')));

            assert_eq!(
                app.repos_cache.iter().cloned().collect::<BTreeSet<_>>(),
                BTreeSet::from([alpha, beta])
            );
            assert_eq!(
                app.status_message.as_deref(),
                Some("Rescanned: 2 repos, 1 new")
            );
            assert_eq!(app.status_kind, StatusKind::Success);
        });
    }

    #[test]
    fn help_and_status_bar_use_rescan_wording() {
        let groups = space::tui::keybindings::all_groups();
        let picker = groups
            .iter()
            .find(|g| g.name == space::tui::keybindings::REPO_PICKER_NAME)
            .expect("help registry must have a Repo Picker group");
        let rows: Vec<(&str, &str)> = picker.bindings.iter().map(|b| (b.key, b.desc)).collect();
        assert_eq!(
            rows,
            vec![
                ("Tab", "Toggle repo"),
                ("Ctrl-S", "Cycle scope"),
                ("Ctrl-R", "Rescan repo list"),
            ]
        );
        // `r` sits in General, not Workspace Pane: it rescans the repo list and
        // also reloads the repos pane, so item 1.3 left it ungated while gating
        // `c`, `a` and `d` to the workspaces pane.
        let general = groups
            .iter()
            .find(|g| g.name == space::tui::keybindings::GENERAL_NAME)
            .unwrap();
        assert!(
            general
                .bindings
                .iter()
                .any(|b| b.key == "r" && b.desc == "Rescan repo list"),
            "dashboard r must read 'Rescan repo list' in the help registry"
        );
        let ws_pane = groups
            .iter()
            .find(|g| g.name == space::tui::keybindings::WORKSPACE_PANE_NAME)
            .unwrap();
        assert!(
            !ws_pane.bindings.iter().any(|b| b.key == "r"),
            "r is a general key, not a workspace-pane key"
        );
        let left = space::tui::keybindings::key_bar_bindings(Pane::Left);
        assert!(
            left.iter().any(|b| b.key == "r" && b.desc == "rescan"),
            "status bar must read 'r rescan'"
        );

        // The rescan wording must survive all the way to the rendered overlay.
        // The registry is taller than any terminal now, so scroll to the end
        // where General lives; that every group is reachable at all is asserted
        // by `every_group_is_reachable_by_scrolling_at_eighty_by_twenty_four`.
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        app.handle_key(key(KeyCode::End));
        let rendered = render_text(&app, 100, 70);
        assert!(rendered.contains("General"), "got:\n{}", rendered);
        assert!(rendered.contains("Rescan repo list"), "got:\n{}", rendered);
    }
}

// ---- 1.2 In-place space filter ----

/// Two bare spaces (no repos) with the first one selected.
fn two_spaces_app() -> App {
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
    test_app(workspaces, vec![])
}

fn type_str(app: &mut App, s: &str) {
    for ch in s.chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
}

#[test]
fn filter_slash_on_workspaces_pane_opens_filter() {
    let mut app = two_spaces_app();
    assert_eq!(app.focus, Pane::Left);

    app.handle_key(key(KeyCode::Char('/')));
    assert!(
        matches!(app.screen, Screen::FilterWorkspace(_)),
        "/ on the workspaces pane must open the space filter, got {:?}",
        app.screen
    );
}

#[test]
fn filter_slash_on_repos_pane_opens_repo_search() {
    let mut app = two_spaces_app();
    app.focus = Pane::Right;

    app.handle_key(key(KeyCode::Char('/')));
    assert!(
        matches!(app.screen, Screen::RepoSearch(_)),
        "/ on the repos pane must open repo search, got {:?}",
        app.screen
    );
}

#[test]
fn filter_enter_selects_space_in_place() {
    let mut app = two_spaces_app();
    app.expanded_repos.insert(0);
    app.cursor_row = 3;
    app.selected_repo = 2;
    let gen_before = app.ws_generation;

    app.handle_key(key(KeyCode::Char('/')));
    type_str(&mut app, "bet");
    app.handle_key(key(KeyCode::Enter));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "Enter must return to the dashboard, got {:?}",
        app.screen
    );
    assert_eq!(app.selected_ws, 1, "the matched space must become selected");
    assert_eq!(
        app.focus,
        Pane::Left,
        "focus must stay on the workspaces pane"
    );
    assert!(!app.should_quit, "the filter must never quit the TUI");
    assert!(
        app.space_cd_target.is_none(),
        "the filter must not set a cd target"
    );
    // Repos pane reset, same as the repo-search landing.
    assert_eq!(app.selected_repo, 0);
    assert_eq!(app.cursor_row, 0);
    assert!(app.expanded_repos.is_empty());
    // Loaded immediately, no debounce.
    assert_eq!(
        app.ws_generation,
        gen_before + 1,
        "load must fire immediately"
    );
    assert!(
        app.nav_pending.is_none(),
        "no debounce timer for an explicit jump"
    );
    assert!(app.ws_loading, "repos pane must be loading");
}

#[test]
fn filter_esc_leaves_selection_untouched() {
    let mut app = two_spaces_app();
    app.expanded_repos.insert(0);
    app.cursor_row = 2;
    let gen_before = app.ws_generation;

    app.handle_key(key(KeyCode::Char('/')));
    type_str(&mut app, "bet");
    app.handle_key(key(KeyCode::Esc));

    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(app.selected_ws, 0, "Esc must not change the selected space");
    assert_eq!(app.focus, Pane::Left);
    assert_eq!(app.cursor_row, 2, "Esc must not reset the repos pane");
    assert!(app.expanded_repos.contains(&0));
    assert_eq!(app.ws_generation, gen_before, "Esc must not trigger a load");
    assert!(!app.ws_loading);
}

#[test]
fn filter_refuses_to_open_with_no_spaces() {
    let mut app = test_app(vec![], vec![]);

    app.handle_key(key(KeyCode::Char('/')));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "an empty space list must not open the filter, got {:?}",
        app.screen
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("No spaces yet, press c to create one")
    );
}

#[test]
fn filter_same_space_keeps_expanded_repos_and_cursor() {
    let mut app = test_app(vec![common::workspace_with_repos(&["a", "b"])], vec![]);
    app.expanded_repos.insert(1);
    app.cursor_row = 1;
    app.selected_repo = 1;
    let gen_before = app.ws_generation;

    app.handle_key(key(KeyCode::Char('/')));
    // The only space is highlighted; Enter re-selects it.
    app.handle_key(key(KeyCode::Enter));

    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(app.selected_ws, 0);
    assert_eq!(app.focus, Pane::Left);
    assert!(
        app.expanded_repos.contains(&1),
        "re-selecting the current space must keep expanded repos"
    );
    assert_eq!(
        app.cursor_row, 1,
        "re-selecting the current space must keep the cursor"
    );
    assert_eq!(app.selected_repo, 1);
    assert_eq!(
        app.ws_generation, gen_before,
        "no reload for the same space"
    );
    assert!(!app.ws_loading);
    assert_eq!(
        app.workspaces[0].repos.len(),
        2,
        "the loaded repos must survive (no skeleton reset)"
    );
}

#[test]
fn filter_no_match_keeps_picker_open_and_enter_does_nothing() {
    let mut app = two_spaces_app();

    app.handle_key(key(KeyCode::Char('/')));
    type_str(&mut app, "zzz");
    let rendered = render_text(&app, 80, 24);
    assert!(
        rendered.contains("0/2 matched"),
        "footer must read 0/N matched, got:\n{}",
        rendered
    );

    app.handle_key(key(KeyCode::Enter));
    assert!(
        matches!(app.screen, Screen::FilterWorkspace(_)),
        "Enter with no match must keep the picker open, got {:?}",
        app.screen
    );
    assert_eq!(app.selected_ws, 0);
    assert!(app.status_message.is_none(), "no error text on no match");
}

#[test]
fn filter_j_and_k_edit_the_query() {
    let mut app = two_spaces_app();

    app.handle_key(key(KeyCode::Char('/')));
    type_str(&mut app, "jk");
    match &app.screen {
        Screen::FilterWorkspace(st) => assert_eq!(st.picker.input.value(), "jk"),
        other => panic!("expected the space filter, got {:?}", other),
    }
}

#[test]
fn go_j_and_k_edit_the_query() {
    let mut app = two_spaces_app();

    app.handle_key(key(KeyCode::Char('g')));
    type_str(&mut app, "jk");
    match &app.screen {
        Screen::GoWorkspace(st) => assert_eq!(st.picker.input.value(), "jk"),
        other => panic!("expected the go picker, got {:?}", other),
    }
}

#[test]
fn filter_and_go_rows_show_repo_count_without_parent() {
    // Real directories: the count comes from a scan of the space's directory,
    // since the dashboard only loads repos for the selected space.
    let env = TestEnv::new();
    for repo in ["one", "two"] {
        std::fs::create_dir_all(env.workspaces_dir.join("alpha").join(repo).join(".git")).unwrap();
    }
    std::fs::create_dir_all(env.workspaces_dir.join("beta")).unwrap();

    for open_key in ['/', 'g'] {
        let workspaces = vec![
            Workspace {
                name: "alpha".to_string(),
                path: env.workspaces_dir.join("alpha"),
                repos: vec![],
            },
            Workspace {
                name: "beta".to_string(),
                path: env.workspaces_dir.join("beta"),
                repos: vec![],
            },
        ];
        let mut app = test_app_with_config(config_from_env(&env), workspaces, vec![]);
        app.handle_key(key(KeyCode::Char(open_key)));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains("2 repos"),
            "{} picker must show the repo count, got:\n{}",
            open_key,
            rendered
        );
        assert!(
            rendered.contains("0 repos"),
            "{} picker must show a zero count, got:\n{}",
            open_key,
            rendered
        );
        assert!(
            !rendered.contains("(workspaces)") && !rendered.contains("()"),
            "{} picker must not draw a parent, got:\n{}",
            open_key,
            rendered
        );
    }
}

#[test]
fn filter_prompt_wording() {
    let mut app = two_spaces_app();
    app.handle_key(key(KeyCode::Char('/')));
    let rendered = render_text(&app, 80, 24);
    assert!(
        rendered.contains("Filter spaces  ENTER=select  ESC=cancel"),
        "got:\n{}",
        rendered
    );
}

#[test]
fn filter_arrows_move_the_highlight_and_enter_selects_by_index() {
    let mut app = two_spaces_app();

    // Empty query: both spaces listed, Down moves the highlight to beta.
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(
        app.selected_ws, 1,
        "Down then Enter must select the second space"
    );

    // Up walks back to alpha.
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Down));
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.selected_ws, 0,
        "Down, Up then Enter must select the first space"
    );
}

#[test]
fn go_enter_sets_cd_target_and_quits() {
    let mut app = two_spaces_app();

    app.handle_key(key(KeyCode::Char('g')));
    type_str(&mut app, "bet");
    app.handle_key(key(KeyCode::Enter));

    assert_eq!(app.space_cd_target, Some(PathBuf::from("/tmp/beta")));
    assert!(app.should_quit, "go must quit the TUI");
    assert_eq!(
        app.selected_ws, 0,
        "go must not touch the dashboard selection"
    );
}

#[test]
fn filter_repo_count_is_excluded_from_matching() {
    let mut app = two_spaces_app();

    app.handle_key(key(KeyCode::Char('/')));
    type_str(&mut app, "repos");
    let rendered = render_text(&app, 80, 24);
    assert!(
        rendered.contains("0/2 matched"),
        "the count label must not match a query, got:\n{}",
        rendered
    );
}

#[test]
fn filter_rows_use_singular_for_one_repo() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.workspaces_dir.join("solo").join("only").join(".git")).unwrap();
    let workspaces = vec![Workspace {
        name: "solo".to_string(),
        path: env.workspaces_dir.join("solo"),
        repos: vec![],
    }];
    let mut app = test_app_with_config(config_from_env(&env), workspaces, vec![]);

    app.handle_key(key(KeyCode::Char('/')));
    let rendered = render_text(&app, 80, 24);
    assert!(
        rendered.contains("1 repo") && !rendered.contains("1 repos"),
        "got:\n{}",
        rendered
    );
}

#[test]
fn filter_help_registry_and_status_bar_wording() {
    use space::tui::keybindings::{all_groups, key_bar_bindings};

    let find = |group: &str, key: &str| -> Option<&'static str> {
        all_groups()
            .iter()
            .find(|g| g.name == group)
            .and_then(|g| g.bindings.iter().find(|b| b.key == key))
            .map(|b| b.desc)
    };
    assert_eq!(find("Workspace Pane", "/"), Some("Filter spaces"));
    assert_eq!(find("Repo Pane", "/"), Some("Search repos"));

    let bar = |pane: Pane| -> Option<&'static str> {
        key_bar_bindings(pane)
            .iter()
            .find(|b| b.key == "/")
            .map(|b| b.desc)
    };
    assert_eq!(bar(Pane::Left), Some("filter"));
    assert_eq!(bar(Pane::Right), Some("search"));
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

    #[test]
    fn terminal_shorter_than_the_dialog_minimum_has_no_row_range_in_the_footer() {
        // A 5-row terminal cannot hold the dialog's 10-row minimum: the list
        // gets zero rows and the visible window is empty. The footer must
        // not claim `rows 1–0 of 2`. The frame is wide enough (inner 82)
        // that the prefix would fit if it were emitted.
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a"), PathBuf::from("/r/b")]);
        report.finished(0, ok_outcome());
        report.finished(1, ok_outcome());
        report.finish();
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 120, 5);
        assert!(
            !rendered.contains("rows 1\u{2013}0"),
            "an empty list must not report a row range:\n{}",
            rendered
        );
        assert!(
            rendered.contains("ENTER continue"),
            "the footer still renders:\n{}",
            rendered
        );
    }

    #[test]
    fn minimum_height_dialog_marks_cut_pane_lines_for_one_repo() {
        // A 12-row terminal clamps the dialog to its 10-row minimum: inner 8,
        // body 5. One failing repo needs one list row, so the two rows the
        // list cannot use go to the pane, which shows the all-failed notice,
        // the header, and the marker for everything it had to cut (the
        // status line and the five stderr lines).
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a")]);
        report.finished(0, failed_outcome("err-1\nerr-2\nerr-3\nerr-4\nerr-5\n"));
        report.finish();
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 100, 12);
        for needle in [
            "Nothing was fetched. You can still continue;",
            "a  /r/a",
            "\u{2026} 6 more lines",
            "ENTER continue anyway",
        ] {
            assert!(
                rendered.contains(needle),
                "a pane with cut lines must show {:?}, got:\n{}",
                needle,
                rendered
            );
        }
        // The list row's DETAIL column shows the first stderr line; the
        // second appears only in the pane.
        assert!(
            !rendered.contains("err-2"),
            "the stderr lines were cut from the pane:\n{}",
            rendered
        );
    }

    #[test]
    fn minimum_height_dialog_keeps_status_line_for_two_repos() {
        // Two repos leave one spare list row for the pane: the header, the
        // status line, then the marker for the five stderr lines.
        let mut app = test_app(vec![], vec![]);
        let mut report = SyncReport::new(&[PathBuf::from("/r/a"), PathBuf::from("/r/b")]);
        report.finished(0, failed_outcome("err-1\nerr-2\nerr-3\nerr-4\nerr-5\n"));
        report.finished(1, ok_outcome());
        report.finish();
        assert_eq!(report.cursor, 0);
        app.screen = Screen::CreateWorkspace(create_state_on_report(report));

        let rendered = render_text(&app, 100, 12);
        for needle in [
            "a  /r/a",
            "fetch failed (git exit 128) \u{b7} branch picker will use local refs",
            "\u{2026} 5 more lines",
            "ENTER continue \u{b7} ESC back",
        ] {
            assert!(
                rendered.contains(needle),
                "a pane with cut lines must show {:?}, got:\n{}",
                needle,
                rendered
            );
        }
        assert!(
            !rendered.contains("err-2"),
            "the stderr lines were cut from the pane:\n{}",
            rendered
        );
    }
}

// ---------------------------------------------------------------------------
// Wave 0: navigation correctness
//
// 0.2: no picker with a text input binds j/k any more. In any screen with a
// text input, letters are text and only arrows move the highlight, whether the
// query is empty or not. These tests pin that deletion on all eight typed
// pickers (create PickRepos and PickBranch, add PickRepos and PickBranch, repo
// search, go, the rebase target and the switch-branch picker) so a repo or
// branch whose name contains 'j' or 'k' stays reachable and typing never jumps
// the list. List-only stages (diff viewer, config editor, git-ops menu and Log,
// sync report, the branch-strategy stages) keep j/k and are covered elsewhere.
// ---------------------------------------------------------------------------

/// Repo paths whose names contain the navigation letters, so a regression is
/// visible as a highlight jump rather than a filtered list.
fn jk_repo_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/tmp/jackal"),
        PathBuf::from("/tmp/kernel"),
        PathBuf::from("/tmp/mango"),
        // A second name matching both "j" and "jk" (the other three match
        // neither), so no keystroke narrows the list to one row and clamps the
        // highlight down off row 1.
        PathBuf::from("/tmp/jokkmokk"),
    ]
}

/// Same names, but under two distinct parent directories so `Ctrl-S` scope
/// cycling has more than one scope to move through.
fn jk_repo_paths_two_scopes() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/tmp/orga/jackal"),
        PathBuf::from("/tmp/orgb/kernel"),
        PathBuf::from("/tmp/orgb/mango"),
    ]
}

use space::tui::widgets::fuzzy_picker::FuzzyPicker;

/// The whole of rule 0.2 for one typed picker: `j` then `k` land in the query
/// and the highlight never moves. Taking the picker by fn pointer keeps every
/// per-screen test down to its fixture plus one call.
fn assert_jk_types_into_the_query(app: &mut App, picker_of: fn(&App) -> &FuzzyPicker, what: &str) {
    // Park the highlight on row 1 before typing. On row 0 "the highlight did
    // not move" would hold under the old bindings too (j down, k back up); from
    // row 1 a j that still navigates lands on row 2 and the assertion bites.
    app.handle_key(key(KeyCode::Down));
    let before = {
        let p = picker_of(app);
        assert!(
            p.input.value().is_empty(),
            "{}: the query must still be empty when j is pressed",
            what
        );
        assert!(
            p.filtered.len() >= 3,
            "{}: the fixture must give a stray j somewhere to jump to, got {}",
            what,
            p.filtered.len()
        );
        assert_eq!(
            p.highlighted, 1,
            "{}: the arrow must move the highlight off row 0",
            what
        );
        p.highlighted
    };

    // Every match below must keep at least two rows, or the picker clamps the
    // highlight for a legitimate reason and the assertion stops meaning
    // anything: the fixtures carry two "jk" names for exactly this.
    app.handle_key(key(KeyCode::Char('j')));
    {
        let p = picker_of(app);
        assert!(
            p.filtered.len() >= 2,
            "{}: the fixture must keep two rows matching \"j\", got {}",
            what,
            p.filtered.len()
        );
        assert_eq!(
            p.highlighted, before,
            "{}: j must not move the highlight",
            what
        );
        assert_eq!(
            p.input.value(),
            "j",
            "{}: j is a character even on an empty query",
            what
        );
    }

    app.handle_key(key(KeyCode::Char('k')));
    let p = picker_of(app);
    assert_eq!(
        p.input.value(),
        "jk",
        "{}: both letters must reach the filter",
        what
    );
    assert!(
        p.filtered.len() >= 2,
        "{}: the fixture must keep two rows matching \"jk\", got {}",
        what,
        p.filtered.len()
    );
    assert_eq!(
        p.highlighted, before,
        "{}: typing must not move the highlight",
        what
    );
}

/// The same rule once a query is already open: `a` first, then `j` and `k` are
/// plain characters that extend it. Each intermediate value is asserted so a
/// failure names the keystroke that went missing.
fn assert_jk_types_into_a_non_empty_query(
    app: &mut App,
    picker_of: fn(&App) -> &FuzzyPicker,
    what: &str,
) {
    // 'a' opens the query; from here j/k must be literal characters.
    app.handle_key(key(KeyCode::Char('a')));
    assert_eq!(
        picker_of(app).input.value(),
        "a",
        "{}: the query must be non-empty before j is pressed",
        what
    );

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(
        picker_of(app).input.value(),
        "aj",
        "{}: j must reach the filter once the query is non-empty",
        what
    );

    app.handle_key(key(KeyCode::Char('k')));
    assert_eq!(
        picker_of(app).input.value(),
        "ajk",
        "{}: k must reach the filter once the query is non-empty",
        what
    );
}

fn create_repos_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::CreateWorkspace(ref st) => &st.picker,
        _ => panic!("expected CreateWorkspace screen"),
    }
}

fn add_repos_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::AddRepos(ref st) => &st.picker,
        _ => panic!("expected AddRepos screen"),
    }
}

fn search_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::RepoSearch(ref st) => &st.picker,
        _ => panic!("expected RepoSearch screen"),
    }
}

fn go_workspace_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::GoWorkspace(ref st) => &st.picker,
        _ => panic!("expected GoWorkspace screen"),
    }
}

fn create_branch_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::CreateWorkspace(ref st) => {
            st.branch_picker.as_ref().expect("branch picker present")
        }
        _ => panic!("expected CreateWorkspace screen"),
    }
}

fn add_branch_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::AddRepos(ref st) => st.branch_picker.as_ref().expect("branch picker present"),
        _ => panic!("expected AddRepos screen"),
    }
}

fn rebase_target_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::GitOps(ref st) => st.rebase_picker.as_ref().expect("rebase picker present"),
        _ => panic!("expected the git-ops overlay"),
    }
}

fn switch_branch_picker(app: &App) -> &FuzzyPicker {
    match app.screen {
        Screen::SwitchBranch(ref st) => st.branch_picker.as_ref().expect("branch picker present"),
        _ => panic!("expected SwitchBranch screen"),
    }
}

/// Open the create flow and land in PickRepos with the given space name.
fn open_create_picker(app: &mut App, name: &str) {
    app.handle_key(key(KeyCode::Char('c')));
    for ch in name.chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));
    match &app.screen {
        Screen::CreateWorkspace(st) => assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickRepos
        ),
        _ => panic!("expected CreateWorkspace screen"),
    }
}

#[test]
fn create_pick_repos_jk_type_into_an_empty_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    open_create_picker(&mut app, "ws");

    assert_jk_types_into_the_query(&mut app, create_repos_picker, "create PickRepos");
}

#[test]
fn create_pick_repos_jk_type_into_a_non_empty_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    open_create_picker(&mut app, "ws");

    assert_jk_types_into_a_non_empty_query(&mut app, create_repos_picker, "create PickRepos");
}

#[test]
fn create_pick_repos_arrows_navigate_even_with_a_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    open_create_picker(&mut app, "ws");

    // A query that keeps at least two items in the filtered set.
    app.handle_key(key(KeyCode::Char('a')));
    let before = match app.screen {
        Screen::CreateWorkspace(ref st) => {
            assert!(
                st.picker.filtered.len() >= 2,
                "fixture must leave >=2 matches for 'a', got {}",
                st.picker.filtered.len()
            );
            st.picker.highlighted
        }
        _ => panic!("expected CreateWorkspace screen"),
    };

    app.handle_key(key(KeyCode::Down));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.picker.highlighted,
            before + 1,
            "arrows always navigate, query or not"
        );
        assert_eq!(
            st.picker.input.value(),
            "a",
            "arrows must not edit the query"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn create_pick_repos_tab_still_toggles_with_a_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    open_create_picker(&mut app, "ws");

    app.handle_key(key(KeyCode::Char('a')));
    app.handle_key(key(KeyCode::Tab));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.picker.toggled.len(),
            1,
            "Tab multi-select must be unaffected by the j/k guard"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

/// The j/k guard splits the navigation arms; `Ctrl-S` sits after them in the
/// same match. Pin that scope-cycling still reaches its arm with a non-empty
/// query rather than being swallowed as literal input.
#[test]
fn create_pick_repos_ctrl_s_still_cycles_scope_with_a_query() {
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    let mut app = test_app(vec![], jk_repo_paths_two_scopes());
    open_create_picker(&mut app, "ws");

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.picker.available_scopes,
            vec!["orga".to_string(), "orgb".to_string()],
            "fixture must provide two scopes to cycle through"
        );
        assert!(st.picker.scope.is_none(), "scope starts unset");
    }

    app.handle_key(key(KeyCode::Char('a')));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    if let Screen::CreateWorkspace(ref st) = app.screen {
        assert_eq!(
            st.picker.scope,
            Some("orga".to_string()),
            "Ctrl-S must still cycle scope while a query is active"
        );
        assert_eq!(
            st.picker.input.value(),
            "a",
            "Ctrl-S must not be typed into the query as a literal 's'"
        );
    } else {
        panic!("expected CreateWorkspace screen");
    }
}

#[test]
fn add_pick_repos_ctrl_s_still_cycles_scope_with_a_query() {
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

    let ws = common::workspace_with_repos(&["existing"]);
    let mut app = test_app(vec![ws], jk_repo_paths_two_scopes());
    app.handle_key(key(KeyCode::Char('a')));
    assert!(matches!(app.screen, Screen::AddRepos(_)));

    app.handle_key(key(KeyCode::Char('a')));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

    if let Screen::AddRepos(ref st) = app.screen {
        assert_eq!(
            st.picker.scope,
            Some("orga".to_string()),
            "Ctrl-S must still cycle scope while a query is active"
        );
        assert_eq!(st.picker.input.value(), "a");
    } else {
        panic!("expected AddRepos screen");
    }
}

#[test]
fn add_pick_repos_jk_type_into_a_non_empty_query() {
    let ws = common::workspace_with_repos(&["existing"]);
    let mut app = test_app(vec![ws], jk_repo_paths());

    app.handle_key(key(KeyCode::Char('a')));
    assert!(
        matches!(app.screen, Screen::AddRepos(_)),
        "expected AddRepos screen"
    );

    assert_jk_types_into_a_non_empty_query(&mut app, add_repos_picker, "add PickRepos");
}

#[test]
fn add_pick_repos_jk_type_into_an_empty_query() {
    let ws = common::workspace_with_repos(&["existing"]);
    let mut app = test_app(vec![ws], jk_repo_paths());

    app.handle_key(key(KeyCode::Char('a')));
    assert!(
        matches!(app.screen, Screen::AddRepos(_)),
        "expected AddRepos screen"
    );

    assert_jk_types_into_the_query(&mut app, add_repos_picker, "add PickRepos");
}

#[test]
fn repo_search_jk_type_into_a_non_empty_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    // Focus right so this stays valid once `/` is pane-gated in Wave 1.2.
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Char('/')));
    assert!(
        matches!(app.screen, Screen::RepoSearch(_)),
        "expected RepoSearch screen"
    );

    assert_jk_types_into_a_non_empty_query(&mut app, search_picker, "repo search");
}

#[test]
fn repo_search_jk_type_into_an_empty_query() {
    let mut app = test_app(vec![], jk_repo_paths());
    app.focus = Pane::Right;
    app.handle_key(key(KeyCode::Char('/')));
    assert!(
        matches!(app.screen, Screen::RepoSearch(_)),
        "expected RepoSearch screen"
    );

    assert_jk_types_into_the_query(&mut app, search_picker, "repo search");
}

fn jk_workspaces() -> Vec<Workspace> {
    ["jackal", "kernel", "mango", "jokkmokk"]
        .iter()
        .map(|n| Workspace {
            name: (*n).to_string(),
            path: PathBuf::from(format!("/tmp/{}", n)),
            repos: vec![],
        })
        .collect()
}

#[test]
fn go_picker_jk_type_into_a_non_empty_query() {
    let mut app = test_app(jk_workspaces(), vec![]);
    app.handle_key(key(KeyCode::Char('g')));
    assert!(
        matches!(app.screen, Screen::GoWorkspace(_)),
        "expected GoWorkspace screen"
    );

    assert_jk_types_into_a_non_empty_query(&mut app, go_workspace_picker, "go picker");
}

#[test]
fn go_picker_jk_type_into_an_empty_query() {
    let mut app = test_app(jk_workspaces(), vec![]);
    app.handle_key(key(KeyCode::Char('g')));
    assert!(
        matches!(app.screen, Screen::GoWorkspace(_)),
        "expected GoWorkspace screen"
    );

    assert_jk_types_into_the_query(&mut app, go_workspace_picker, "go picker");
}

/// Create the two `ajk-*` branches every typed branch picker fixture needs,
/// so pressing `j`/`k` after `a` has rows to filter against.
fn create_jk_branches(repo_path: &std::path::Path) {
    for branch in ["ajk-one", "ajk-two"] {
        let out = std::process::Command::new("git")
            .args(["branch", branch])
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
}

/// Drive the create flow as far as the PickBranch fuzzy picker, which is the
/// sixth older picker and needs a real repo behind it.
fn open_create_pick_branch(env: &TestEnv) -> App {
    let repo_path = env.create_repo("branch-picker-repo");
    create_jk_branches(&repo_path);

    let config = config_from_env(env);
    let mut app = test_app_with_config(config, vec![], vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("ws-branch".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
        st.recent_branches = vec![];
        st.branch_strategy_idx = 3; // "Pick a branch..." when there are no recents
    }
    app.handle_key(key(KeyCode::Enter));
    match app.screen {
        Screen::CreateWorkspace(ref st) => assert_eq!(
            st.stage,
            space::tui::screens::create::CreateStage::PickBranch,
            "fixture should have reached the branch picker"
        ),
        _ => panic!("expected CreateWorkspace screen"),
    }
    app
}

#[test]
fn create_pick_branch_jk_type_into_a_non_empty_query() {
    let env = TestEnv::new();
    let mut app = open_create_pick_branch(&env);

    assert_jk_types_into_a_non_empty_query(&mut app, create_branch_picker, "create PickBranch");
}

#[test]
fn create_pick_branch_jk_type_into_an_empty_query() {
    let env = TestEnv::new();
    let mut app = open_create_pick_branch(&env);

    assert_jk_types_into_the_query(&mut app, create_branch_picker, "create PickBranch");
}

/// The Add flow's branch picker is the sixth of the six older pickers. Its j/k
/// guard is currently byte-identical to the Create flow's, but identical-by-
/// inspection is not a regression net: this pins it independently so the two
/// cannot silently drift apart.
fn open_add_pick_branch(env: &TestEnv) -> App {
    let repo_path = env.create_repo("add-branch-picker-repo");
    create_jk_branches(&repo_path);

    let config = config_from_env(env);
    let workspaces = vec![Workspace {
        name: "ws-add-branch".to_string(),
        path: env.workspaces_dir.join("ws-add-branch"),
        repos: vec![],
    }];
    let mut app = test_app_with_config(config, workspaces, vec![repo_path.clone()]);

    app.handle_key(key(KeyCode::Char('a')));
    if let Screen::AddRepos(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.stage = space::tui::screens::add::AddStage::PickBranchStrategy;
        st.recent_branches = vec![];
        st.branch_strategy_idx = 3; // "Pick a branch..." when there are no recents
    } else {
        panic!("expected AddRepos screen");
    }
    app.handle_key(key(KeyCode::Enter));
    match app.screen {
        Screen::AddRepos(ref st) => assert_eq!(
            st.stage,
            space::tui::screens::add::AddStage::PickBranch,
            "fixture should have reached the add-flow branch picker"
        ),
        _ => panic!("expected AddRepos screen"),
    }
    app
}

#[test]
fn add_pick_branch_jk_type_into_a_non_empty_query() {
    let env = TestEnv::new();
    let mut app = open_add_pick_branch(&env);

    assert_jk_types_into_a_non_empty_query(&mut app, add_branch_picker, "add PickBranch");
}

#[test]
fn add_pick_branch_jk_type_into_an_empty_query() {
    let env = TestEnv::new();
    let mut app = open_add_pick_branch(&env);

    assert_jk_types_into_the_query(&mut app, add_branch_picker, "add PickBranch");
}

/// A repo with two extra `j`/`k` branches, in a workspace, focused on its repo
/// row: the shared fixture for the last two typed pickers, both of which are
/// reached from a repo row rather than from a create/add flow.
fn jk_branch_repo_app(env: &TestEnv, name: &str) -> App {
    let repo_path = env.create_repo(name);
    create_jk_branches(&repo_path);

    let ws = Workspace {
        name: "jk-branch-ws".to_string(),
        path: env.workspaces_dir.clone(),
        repos: vec![WorkspaceRepo {
            name: name.to_string(),
            path: repo_path.clone(),
            branch: "main".to_string(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }],
    };
    let config = config_from_env(env);
    let mut app = test_app_with_config(config, vec![ws], vec![repo_path]);
    app.load_selected_workspace_detail();
    app.focus = Pane::Right; // cursor_row = 0 is the repo row
    app
}

/// The rebase target picker: `G` opens the git-ops menu, `r` the rebase
/// pre-flight, `Enter` on a clean tree the picker itself.
fn open_rebase_pick_target(env: &TestEnv) -> App {
    use space::tui::screens::gitops::GitOpsStage;
    let mut app = jk_branch_repo_app(env, "rebase-jk-repo");

    app.handle_key(key(KeyCode::Char('G')));
    app.handle_key(key(KeyCode::Char('r')));
    app.handle_key(key(KeyCode::Enter));

    match app.screen {
        Screen::GitOps(ref st) => assert_eq!(
            st.stage,
            GitOpsStage::RebasePickTarget,
            "fixture should have reached the rebase target picker"
        ),
        _ => panic!("expected the git-ops overlay"),
    }
    app
}

/// The rebase target picker is one of the two that used to guard j/k on an
/// empty query. The guard is gone, so it follows the same rule as the rest.
#[test]
fn gitops_rebase_target_jk_type_into_an_empty_query() {
    let env = TestEnv::new();
    let mut app = open_rebase_pick_target(&env);

    assert_jk_types_into_the_query(&mut app, rebase_target_picker, "rebase target");
}

/// The switch-branch picker: `b` on a repo row, then the last strategy row
/// ("Pick a branch...") opens the fuzzy list of every branch.
fn open_switch_branch_pick_branch(env: &TestEnv) -> App {
    use space::tui::screens::switch_branch::SwitchBranchStage;
    let mut app = jk_branch_repo_app(env, "switch-jk-repo");

    app.handle_key(key(KeyCode::Char('b')));
    if let Screen::SwitchBranch(ref mut st) = app.screen {
        st.strategy_idx = st.max_idx();
    } else {
        panic!("expected SwitchBranch screen");
    }
    app.handle_key(key(KeyCode::Enter));

    match app.screen {
        Screen::SwitchBranch(ref st) => assert_eq!(
            st.stage,
            SwitchBranchStage::PickBranch,
            "fixture should have reached the switch-branch picker"
        ),
        _ => panic!("expected SwitchBranch screen"),
    }
    app
}

/// The switch-branch picker is the other formerly guarded one.
#[test]
fn switch_branch_pick_branch_jk_type_into_an_empty_query() {
    let env = TestEnv::new();
    let mut app = open_switch_branch_pick_branch(&env);

    assert_jk_types_into_the_query(&mut app, switch_branch_picker, "switch branch");
}

/// `q` is a character in a typed picker, not an exit. The create, add and
/// rebase-target pickers already treat it that way; this pins the same rule on
/// the switch-branch picker so a branch named `queue` stays reachable. Esc
/// keeps its meaning as the way out.
#[test]
fn switch_branch_pick_branch_q_types_into_the_query() {
    use space::tui::screens::switch_branch::SwitchBranchStage;
    let env = TestEnv::new();
    let mut app = open_switch_branch_pick_branch(&env);

    app.handle_key(key(KeyCode::Char('q')));

    match app.screen {
        Screen::SwitchBranch(ref st) => {
            assert_eq!(
                st.stage,
                SwitchBranchStage::PickBranch,
                "q must not leave the branch picker"
            );
            let bp = st.branch_picker.as_ref().expect("branch picker present");
            assert_eq!(
                bp.input.value(),
                "q",
                "q must reach the filter like every other letter"
            );
        }
        _ => panic!("expected SwitchBranch screen; q must not exit the flow"),
    }
}

/// The new-branch name field is a text input, so `q` is typed there too: a
/// branch called `quick` must be nameable.
#[test]
fn switch_branch_enter_branch_name_types_q() {
    use space::tui::screens::switch_branch::SwitchBranchStage;
    let env = TestEnv::new();
    let mut app = jk_branch_repo_app(&env, "switch-name-repo");

    app.handle_key(key(KeyCode::Char('b')));
    match app.screen {
        // Row 0 of the strategy list is "New branch", so Enter opens the name field.
        Screen::SwitchBranch(ref st) => assert_eq!(st.strategy_idx, 0),
        _ => panic!("expected SwitchBranch screen"),
    }
    app.handle_key(key(KeyCode::Enter));

    for ch in "quick".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }

    match app.screen {
        Screen::SwitchBranch(ref st) => {
            assert_eq!(
                st.stage,
                SwitchBranchStage::EnterBranchName,
                "q must not leave the branch-name field"
            );
            assert_eq!(
                st.branch_name_input.value(),
                "quick",
                "every letter, q included, belongs to the branch name"
            );
        }
        _ => panic!("expected SwitchBranch screen; q must not exit the flow"),
    }
}

// ---------------------------------------------------------------------------
// 0.3: `g` is gated to the workspace pane
// ---------------------------------------------------------------------------

#[test]
fn g_on_left_pane_opens_the_go_picker() {
    let mut app = test_app(jk_workspaces(), vec![]);
    assert_eq!(app.focus, Pane::Left);

    app.handle_key(key(KeyCode::Char('g')));

    assert!(
        matches!(app.screen, Screen::GoWorkspace(_)),
        "g keeps its documented workspace-pane meaning"
    );
}

/// The pane guard makes the `g` arm fall through. No later arm in the dashboard
/// match binds lowercase `g` (the nearest is `G`, a distinct char), so it must
/// reach the wildcard and do nothing at all, not merely "not open the picker".
#[test]
fn g_on_right_pane_is_inert() {
    let ws = common::workspace_with_repos(&["repo-a", "repo-b"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.cursor_row = 1;

    app.handle_key(key(KeyCode::Char('g')));

    assert!(
        matches!(app.screen, Screen::Dashboard),
        "g while browsing repo rows must not jump to the workspace picker"
    );
    assert!(!app.should_quit, "g must not quit");
    assert_eq!(app.focus, Pane::Right, "g must not move focus");
    assert_eq!(app.cursor_row, 1, "g must not move the cursor");
    assert!(
        app.expanded_repos.is_empty(),
        "g must not expand or collapse anything"
    );
}

#[test]
fn shift_g_on_right_pane_still_opens_git_ops() {
    let ws = common::workspace_with_repos(&["repo-a"]);
    let mut app = test_app(vec![ws], vec![]);
    app.focus = Pane::Right;
    app.cursor_row = 0;

    app.handle_key(shift_key(KeyCode::Char('G')));

    assert!(
        matches!(app.screen, Screen::GitOps(_)),
        "G must remain the git-ops menu; the g gate must not disturb it"
    );
}

// ---------------------------------------------------------------------------
// 0.4: delete confirm defaults to No
// ---------------------------------------------------------------------------

/// A workspace the app lists but whose directory does not exist under the
/// (temporary) workspaces dir. Confirming therefore fails loudly instead of
/// deleting anything, which is exactly what lets the confirm case below prove
/// that a delete was attempted at all.
fn delete_confirm_app(env: &TestEnv) -> App {
    let workspaces = vec![Workspace {
        name: "keep-me".to_string(),
        path: env.workspaces_dir.join("keep-me"),
        repos: vec![],
    }];
    test_app_with_config(config_from_env(env), workspaces, vec![])
}

/// `ScreenAction::Back` sets no status message, so an empty status line is the
/// evidence that the decline key never reached `remove_workspace`. Screen and
/// workspace-count assertions alone cannot tell the two apart: the fixture
/// workspace is not on disk, so a delete that *did* run would also land back on
/// the dashboard with the list untouched.
fn assert_delete_was_declined(app: &App, key_name: &str) {
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "{} must return to the dashboard",
        key_name
    );
    assert_eq!(
        app.workspaces.len(),
        1,
        "{}: nothing may be deleted",
        key_name
    );
    assert_eq!(
        app.workspaces[0].name, "keep-me",
        "{}: the workspace must survive intact",
        key_name
    );
    assert_eq!(
        app.status_message, None,
        "{}: declining must not attempt a delete, and any attempt sets a status message",
        key_name
    );
}

#[test]
fn delete_enter_cancels_instead_of_deleting() {
    let env = TestEnv::new();
    let mut app = delete_confirm_app(&env);

    app.handle_key(key(KeyCode::Char('d')));
    assert!(matches!(app.screen, Screen::ConfirmDelete(_)));

    app.handle_key(key(KeyCode::Enter));

    // Enter declines, matching the [y/N] pattern used by push and rebase.
    assert_delete_was_declined(&app, "Enter");
}

#[test]
fn delete_uppercase_y_deletes_and_uppercase_n_cancels() {
    use space::tui::actions::StatusKind;

    let env = TestEnv::new();

    let mut app = delete_confirm_app(&env);
    app.handle_key(key(KeyCode::Char('d')));
    app.handle_key(shift_key(KeyCode::Char('N')));
    assert_delete_was_declined(&app, "N");

    // q cancels too, the same set of decline keys as the push and rebase
    // confirmations (they share one default-No helper).
    let mut app = delete_confirm_app(&env);
    app.handle_key(key(KeyCode::Char('d')));
    app.handle_key(key(KeyCode::Char('q')));
    assert_delete_was_declined(&app, "q");

    // Y is accepted as confirmation; the workspace path does not exist on disk,
    // so the delete fails loudly rather than silently doing nothing. That error
    // status is the proof the keypress reached `remove_workspace`.
    let mut app = delete_confirm_app(&env);
    app.handle_key(key(KeyCode::Char('d')));
    app.handle_key(shift_key(KeyCode::Char('Y')));
    assert!(
        !matches!(app.screen, Screen::ConfirmDelete(_)),
        "Y must be treated as confirmation, not swallowed"
    );
    assert_eq!(
        app.status_message.as_deref(),
        Some("Delete failed: workspace 'keep-me' not found"),
        "Y must reach the delete, which reports the missing directory"
    );
    assert_eq!(app.status_kind, StatusKind::Error);
}

// ---------------------------------------------------------------------------
// ---- 1.3 List paging on the workspace and repo lists ----
// ---------------------------------------------------------------------------

mod paging_tests {
    use super::*;

    /// `n` bare spaces, the first selected, focus on the workspaces pane.
    fn spaces_app(n: usize) -> App {
        let workspaces = (0..n)
            .map(|i| Workspace {
                name: format!("space-{:02}", i),
                path: PathBuf::from(format!("/tmp/space-{:02}", i)),
                repos: vec![],
            })
            .collect();
        test_app(workspaces, vec![])
    }

    /// One space holding `repo-a`, expanded over `files` unstaged file rows, so
    /// the flattened list is Repo, SectionHeader, then one File per name.
    fn expanded_repo_app(files: usize) -> App {
        use space::core::git::{FileEntry, FileStatus};
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.expanded_repos.insert(0);
        app.repo_file_cache.insert(
            0,
            (0..files)
                .map(|i| FileEntry {
                    path: format!("file-{:02}.rs", i),
                    status: FileStatus::Modified,
                    staged: false,
                    insertions: 1,
                    deletions: 0,
                })
                .collect(),
        );
        app
    }

    // --- left pane ---

    #[test]
    fn workspace_page_down_moves_by_one_page_and_page_up_returns() {
        let mut app = spaces_app(30);
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.selected_ws, 10, "PgDn pages by 10 rows");
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.selected_ws, 20);
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.selected_ws, 10, "PgUp pages back by 10 rows");
    }

    #[test]
    fn workspace_paging_clamps_at_both_ends() {
        let mut app = spaces_app(15);
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.selected_ws, 0, "PgUp at the top stays at the top");
        app.handle_key(key(KeyCode::PageDown));
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.selected_ws, 14, "PgDn clamps to the last space");
    }

    #[test]
    fn workspace_home_and_end_jump_to_the_ends() {
        let mut app = spaces_app(30);
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.selected_ws, 29);
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.selected_ws, 0);
    }

    #[test]
    fn workspace_paging_on_an_empty_list_is_a_noop() {
        let mut app = test_app(vec![], vec![]);
        for code in [
            KeyCode::PageDown,
            KeyCode::PageUp,
            KeyCode::End,
            KeyCode::Home,
        ] {
            app.handle_key(key(code));
            assert_eq!(
                app.selected_ws, 0,
                "{:?} on an empty list must not move",
                code
            );
        }
    }

    /// A jump that does not change the selection must not fire the repo-pane
    /// reset, so a reflex `End` on an already-bottom list keeps expansions.
    #[test]
    fn workspace_end_on_the_last_space_keeps_the_repo_pane_state() {
        let mut app = spaces_app(3);
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.selected_ws, 2);
        app.expanded_repos.insert(0);
        app.cursor_row = 4;

        app.handle_key(key(KeyCode::End));
        assert_eq!(app.selected_ws, 2);
        assert!(
            app.expanded_repos.contains(&0),
            "End on an already-last space must not reset the repo pane"
        );
        assert_eq!(app.cursor_row, 4, "nor move the repo cursor");
    }

    #[test]
    fn workspace_home_at_the_top_keeps_the_repo_pane_state() {
        let mut app = spaces_app(3);
        app.expanded_repos.insert(0);
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.selected_ws, 0);
        assert!(app.expanded_repos.contains(&0));
    }

    // --- right pane ---

    #[test]
    fn repo_page_down_moves_by_one_page_over_flattened_rows() {
        // rows: Repo(0), SectionHeader(1), File(2)..File(31)
        let mut app = expanded_repo_app(30);
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.cursor_row, 10, "PgDn pages over flattened rows");
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.cursor_row, 0);
    }

    #[test]
    fn repo_paging_clamps_at_both_ends() {
        let mut app = expanded_repo_app(5);
        // rows: Repo(0), SectionHeader(1), File(2)..File(6) => 7 rows
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.cursor_row, 0);
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.cursor_row, 6, "PgDn clamps to the last row");
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.cursor_row, 6);
    }

    #[test]
    fn repo_home_and_end_jump_to_the_ends() {
        let mut app = expanded_repo_app(12);
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.cursor_row, 13, "End lands on the last file row");
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.cursor_row, 0, "Home lands on the repo row");
    }

    /// A page must never leave the cursor on a section header, in either
    /// direction: those rows are not selectable by `j`/`k` either.
    /// One repo expanded over 9 unstaged then 9 staged files, so the "Staged"
    /// header sits at row 11 and a page from row 1 lands exactly on it.
    fn header_heavy_app() -> App {
        use space::core::git::{FileEntry, FileStatus};
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.expanded_repos.insert(0);
        let mut entries: Vec<FileEntry> = (0..9)
            .map(|i| FileEntry {
                path: format!("u-{}.rs", i),
                status: FileStatus::Modified,
                staged: false,
                insertions: 1,
                deletions: 0,
            })
            .collect();
        entries.extend((0..9).map(|i| FileEntry {
            path: format!("s-{}.rs", i),
            status: FileStatus::Modified,
            staged: true,
            insertions: 1,
            deletions: 0,
        }));
        app.repo_file_cache.insert(0, entries);
        app
    }

    #[test]
    fn repo_paging_never_rests_on_a_section_header() {
        let mut app = header_heavy_app();

        // Walk every landing spot a page can reach from every start.
        let total = app.flattened_rows().len();
        for start in 0..total {
            for code in [KeyCode::PageDown, KeyCode::PageUp] {
                app.cursor_row = start;
                app.handle_key(key(code));
                let rows = app.flattened_rows();
                assert!(
                    !matches!(
                        rows[app.cursor_row],
                        space::tui::app::RepoRow::SectionHeader { .. }
                    ),
                    "{:?} from row {} landed on a section header at row {}",
                    code,
                    start,
                    app.cursor_row
                );
            }
        }
    }

    /// Stronger than "never rests on a header": a page that lands on a header
    /// must step out the way it was travelling, so the landing row is on the
    /// far side of the target. Hardcoding the skip direction to `down` leaves
    /// the header assertion passing but moves a PgUp *towards* the cursor,
    /// which this catches.
    #[test]
    fn a_page_resolves_a_header_in_its_own_direction_of_travel() {
        use space::tui::app::RepoRow;
        let mut app = header_heavy_app();
        let total = app.flattened_rows().len();
        const PAGE: usize = 10;

        for start in 0..total {
            // PgUp: the target is `start - PAGE`; resolving a header there must
            // move further up, never back down towards `start`.
            app.cursor_row = start;
            let target = start.saturating_sub(PAGE);
            app.handle_key(key(KeyCode::PageUp));
            assert!(
                app.cursor_row <= target,
                "PgUp from {} targeted {} but resolved down to {}",
                start,
                target,
                app.cursor_row
            );

            // PgDn: the target is `start + PAGE` clamped; resolving a header
            // there must move further down.
            app.cursor_row = start;
            let target = (start + PAGE).min(total - 1);
            app.handle_key(key(KeyCode::PageDown));
            assert!(
                app.cursor_row >= target,
                "PgDn from {} targeted {} but resolved up to {}",
                start,
                target,
                app.cursor_row
            );
            let rows = app.flattened_rows();
            assert!(!matches!(
                rows[app.cursor_row],
                RepoRow::SectionHeader { .. }
            ));
        }
    }

    /// `cursor_row` can outlive the rows it indexes: `ScreenAction::StageFile`
    /// (the diff-viewer path) calls `do_stage` and returns to the dashboard
    /// without the `reposition_after_section_change` its `Message::StageFile`
    /// twin performs, so a refetch that returns fewer rows leaves the cursor
    /// past the end. A page from there must not index out of bounds.
    #[test]
    fn paging_from_a_stale_cursor_does_not_panic() {
        let mut app = expanded_repo_app(30);
        assert!(app.flattened_rows().len() > 20);

        // The repo's files vanish underneath the cursor (external commit, then
        // a stage from the diff viewer refetches an empty list).
        app.cursor_row = 25;
        app.repo_file_cache.insert(0, vec![]);
        assert_eq!(app.flattened_rows().len(), 1, "only the repo row is left");

        // Arrows too, not just the paging keys: `skip_headers` reads the raw
        // cursor, so `k`/Up is the likeliest way a user meets this.
        for code in [
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('k'),
            KeyCode::Char('j'),
        ] {
            app.cursor_row = 25;
            app.handle_key(key(code));
            assert!(
                app.cursor_row < app.flattened_rows().len(),
                "{:?} left the cursor at {} for {} rows",
                code,
                app.cursor_row,
                app.flattened_rows().len()
            );
        }
    }

    #[test]
    fn repo_paging_on_an_empty_workspace_is_a_noop() {
        let ws = Workspace {
            name: "empty".into(),
            path: PathBuf::from("/tmp/empty"),
            repos: vec![],
        };
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        for code in [
            KeyCode::PageDown,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::Home,
        ] {
            app.handle_key(key(code));
            assert_eq!(
                app.cursor_row, 0,
                "{:?} on an empty repo list must not move",
                code
            );
        }
    }

    // --- pane gating of c / a / d, and r staying general ---

    #[test]
    fn create_add_and_delete_are_ignored_on_the_repo_pane() {
        for k in ['c', 'a', 'd'] {
            let ws = common::workspace_with_repos(&["repo-a"]);
            let mut app = test_app(vec![ws], vec![]);
            app.focus = Pane::Right;
            app.handle_key(key(KeyCode::Char(k)));
            assert!(
                matches!(app.screen, Screen::Dashboard),
                "`{}` on the repo pane must not leave the dashboard",
                k
            );
        }
    }

    #[test]
    fn create_add_and_delete_still_fire_on_the_workspaces_pane() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.handle_key(key(KeyCode::Char('d')));
        assert!(
            matches!(app.screen, Screen::ConfirmDelete(_)),
            "`d` on the workspaces pane still opens the delete confirm"
        );
    }

    /// `r` is a general key, not a workspace-pane key: it also reloads the repo
    /// pane, so it must keep working from the repo pane.
    #[test]
    fn rescan_still_fires_on_the_repo_pane() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.expanded_repos.insert(0);
        app.handle_key(key(KeyCode::Char('r')));
        assert!(
            app.status_message.is_some(),
            "`r` on the repo pane must still rescan and report"
        );
        assert!(
            app.expanded_repos.is_empty(),
            "`r` still resets the repo pane it is looking at"
        );
    }

    // --- g and G keep their shipped meanings (closed open question 2) ---

    #[test]
    fn g_and_shift_g_are_not_paging_keys() {
        let mut app = spaces_app(30);
        app.handle_key(shift_key(KeyCode::Char('G')));
        assert_eq!(
            app.selected_ws, 0,
            "`G` on the workspaces pane is not go-to-bottom"
        );
        assert!(
            matches!(app.screen, Screen::Dashboard),
            "`G` on the workspaces pane opens nothing"
        );

        let mut app = expanded_repo_app(30);
        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.cursor_row, 0, "`g` on the repo pane is not go-to-top");
        assert!(
            matches!(app.screen, Screen::Dashboard),
            "`g` on the repo pane still opens nothing (Wave 0 gate)"
        );
    }
}

// ---------------------------------------------------------------------------
// ---- 1.4 `?` help inside overlays, and a complete registry ----
// ---------------------------------------------------------------------------

mod help_overlay_tests {
    use super::*;
    use space::tui::keybindings;

    /// A dashboard app sitting on the git-ops menu for the first repo.
    fn gitops_menu_app() -> App {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(shift_key(KeyCode::Char('G')));
        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "fixture must reach the git-ops overlay"
        );
        app
    }

    // --- opening from mid-flow, and returning to it ---

    #[test]
    fn question_mark_opens_help_from_a_gitops_stage_and_returns_to_it() {
        let mut app = gitops_menu_app();
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_some(), "? must open help from the git-ops menu");
        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "the git-ops state must still be there while help is open"
        );

        app.handle_key(key(KeyCode::Esc));
        assert!(app.help.is_none());
        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "closing help must return to the exact prior screen, not the dashboard"
        );
    }

    #[test]
    fn question_mark_opens_help_from_the_delete_confirm_and_returns_to_it() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.handle_key(key(KeyCode::Char('d')));
        assert!(matches!(app.screen, Screen::ConfirmDelete(_)));

        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_some());
        app.handle_key(key(KeyCode::Esc));
        assert!(
            matches!(app.screen, Screen::ConfirmDelete(_)),
            "help must not cancel the confirmation it was opened over"
        );
    }

    #[test]
    fn question_mark_opens_help_from_the_diff_viewer() {
        use space::tui::screens::diff::DiffViewerState;
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.screen = Screen::DiffViewer(DiffViewerState {
            repo_index: 0,
            repo_name: "repo-a".into(),
            repo_path: PathBuf::from("/tmp/test-ws/repo-a"),
            file_path: "a.rs".into(),
            staged: false,
            diff: Err("no diff".into()),
            scroll_offset: 0,
            total_lines: 1,
        });
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_some());
        app.handle_key(key(KeyCode::Char('q')));
        assert!(
            matches!(app.screen, Screen::DiffViewer(_)),
            "q closes help and leaves the diff viewer open"
        );
    }

    // --- ? stays text where text is being typed; F1 always works ---

    #[test]
    fn question_mark_in_a_picker_query_is_typed_not_help() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.handle_key(key(KeyCode::Char('/')));
        assert!(
            matches!(
                app.screen,
                Screen::RepoSearch(_) | Screen::FilterWorkspace(_)
            ),
            "fixture must open a typed picker"
        );
        app.handle_key(key(KeyCode::Char('?')));
        assert!(
            app.help.is_none(),
            "? is a legitimate character in a query and must not open help"
        );
        let query = match &app.screen {
            Screen::RepoSearch(s) => s.picker.input.value().to_string(),
            Screen::FilterWorkspace(s) => s.picker.input.value().to_string(),
            other => panic!("unexpected screen {:?}", std::mem::discriminant(other)),
        };
        assert_eq!(query, "?", "? must reach the query");
    }

    #[test]
    fn f1_opens_help_from_a_picker_query_without_disturbing_it() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('r')));
        app.handle_key(key(KeyCode::F(1)));
        assert!(
            app.help.is_some(),
            "F1 must open help even while a query is being typed"
        );
        app.handle_key(key(KeyCode::Esc));
        let query = match &app.screen {
            Screen::RepoSearch(s) => s.picker.input.value().to_string(),
            other => panic!("unexpected screen {:?}", std::mem::discriminant(other)),
        };
        assert_eq!(query, "r", "the query must survive the help round trip");
    }

    /// `?` must reach the input in every stage that types, not just the one
    /// picker the first test covered. The per-screen stage lists are hand
    /// maintained, so adding a stage to the wrong list would otherwise swallow
    /// a typed `?` with the suite still green.
    #[test]
    fn question_mark_types_in_every_text_stage() {
        let env = TestEnv::new();
        let repo = env.create_repo("repo-a");
        let cfg = || config_from_env(&env);

        // create: EnterName
        let mut app = test_app_with_config(cfg(), vec![], vec![repo.clone()]);
        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "create EnterName: ? must type");
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(st.ws_name.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // create: PickRepos query
        app.handle_key(key(KeyCode::Char('w')));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "create PickRepos: ? must type");
        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(st.picker.input.value(), "?", "query must take the ?")
            }
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // git-ops: Committing (single-line commit message)
        let mut app = gitops_menu_app();
        if let Screen::GitOps(st) = &mut app.screen {
            st.stage = space::tui::screens::gitops::GitOpsStage::Committing;
        }
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "git-ops Committing: ? must type");
        match &app.screen {
            Screen::GitOps(st) => assert_eq!(st.message_input.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // create: EnterBranchName. The stage is set directly, as for the
        // git-ops and switch-branch cases below: what is under test is `?`
        // dispatch at that stage, not the route to it.
        let mut app = test_app_with_config(cfg(), vec![], vec![repo.clone()]);
        app.handle_key(key(KeyCode::Char('c')));
        if let Screen::CreateWorkspace(st) = &mut app.screen {
            st.stage = space::tui::screens::create::CreateStage::EnterBranchName;
        }
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "create EnterBranchName: ? must type");
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(st.branch_name_input.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // add: PickRepos query and EnterBranchName share the same shape.
        let mut app = test_app_with_config(cfg(), vec![], vec![repo.clone()]);
        app.workspaces.push(space::core::workspace::Workspace {
            name: "ws".into(),
            path: PathBuf::from("/tmp/ws"),
            repos: vec![],
        });
        app.handle_key(key(KeyCode::Char('a')));
        assert!(
            matches!(app.screen, Screen::AddRepos(_)),
            "fixture must open the add flow"
        );
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "add PickRepos: ? must type");
        match &app.screen {
            Screen::AddRepos(st) => assert_eq!(st.picker.input.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }
        if let Screen::AddRepos(st) = &mut app.screen {
            st.stage = space::tui::screens::add::AddStage::EnterBranchName;
        }
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "add EnterBranchName: ? must type");
        match &app.screen {
            Screen::AddRepos(st) => assert_eq!(st.branch_name_input.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // switch-branch: EnterBranchName
        let mut app = test_app_with_config(cfg(), vec![], vec![]);
        app.screen =
            Screen::SwitchBranch(space::tui::screens::switch_branch::SwitchBranchState::new(
                "repo-a".into(),
                PathBuf::from("/tmp/repo-a"),
            ));
        if let Screen::SwitchBranch(st) = &mut app.screen {
            st.stage = space::tui::screens::switch_branch::SwitchBranchStage::EnterBranchName;
        }
        app.handle_key(key(KeyCode::Char('?')));
        assert!(
            app.help.is_none(),
            "switch-branch EnterBranchName: ? must type"
        );
        match &app.screen {
            Screen::SwitchBranch(st) => assert_eq!(st.branch_name_input.value(), "?"),
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }

        // config editor while editing
        let env2 = TestEnv::new();
        let mut app = test_app_with_config(config_from_env(&env2), vec![], vec![]);
        app.handle_key(shift_key(KeyCode::Char('S')));
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_none(), "config editor editing: ? must type");
        match &app.screen {
            Screen::ConfigEditor(st) => {
                assert!(
                    st.input.value().ends_with('?'),
                    "got {:?}",
                    st.input.value()
                )
            }
            other => panic!("unexpected {:?}", std::mem::discriminant(other)),
        }
    }

    // --- ADR 0001: help must not cancel work running behind it ---

    /// The reason help is an overlay layer rather than a `Screen` variant.
    /// `poll_sync_result` cancels the worker and drops the receiver whenever
    /// the current screen is not a Syncing stage, so a design that moved the
    /// screen into a Help variant would kill the sync it was showing.
    #[test]
    fn help_over_a_running_sync_leaves_the_sync_running() {
        let env = TestEnv::new();
        let repo = env.create_repo("repo-a");
        let config = config_from_env(&env);
        let mut app = test_app_with_config(config, vec![], vec![repo]);

        app.handle_key(key(KeyCode::Char('c')));
        for c in "ws".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter)); // EnterName -> PickRepos
        app.handle_key(key(KeyCode::Tab)); // toggle the repo
        app.handle_key(key(KeyCode::Enter)); // PickRepos -> Syncing, starts the worker
        assert!(
            matches!(&app.screen, Screen::CreateWorkspace(st)
                if st.stage == space::tui::screens::create::CreateStage::Syncing),
            "fixture must reach the Syncing stage"
        );

        app.handle_key(key(KeyCode::Char('?')));
        assert!(
            app.help.is_some(),
            "? must open help over the Syncing stage"
        );

        // Pump the loop the way run_loop does while the overlay is up.
        for _ in 0..200 {
            app.poll_sync_result();
        }
        app.handle_key(key(KeyCode::Esc));
        assert!(app.help.is_none());

        drain_sync(&mut app);
        let report_done = match &app.screen {
            Screen::CreateWorkspace(st) => st.report.done,
            other => panic!("unexpected screen {:?}", std::mem::discriminant(other)),
        };
        assert!(
            report_done,
            "the sync must still finish after help was opened over it"
        );
    }

    /// ADR 0001's invariant, enforced by the renderer rather than by the `?`
    /// gate. `F1` deliberately opens help from inside text inputs, which is
    /// exactly where the three `set_cursor_position` paths live, and ratatui
    /// 0.30 cannot unset a cursor once a frame has set one. So the screen
    /// beneath the overlay must not set it in the first place.
    #[test]
    fn no_cursor_is_drawn_beneath_the_help_overlay() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        /// Draw once without help to prove this screen does place a cursor,
        /// then park the terminal cursor on a sentinel and draw again with
        /// help open. ratatui only moves the terminal cursor when the frame
        /// set one, so an unmoved sentinel proves the frame set none.
        fn cursor_with_help(mut app: App, label: &str) {
            use ratatui::layout::Position;
            const SENTINEL: Position = Position { x: 79, y: 23 };

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal.draw(|f| space::tui::ui::view(&app, f)).unwrap();
            let without_help = terminal.get_cursor_position().unwrap();
            assert_ne!(
                without_help, SENTINEL,
                "{}: fixture must be a screen that places a cursor",
                label
            );

            app.handle_key(key(KeyCode::F(1)));
            assert!(app.help.is_some(), "{}: F1 must open help", label);

            terminal.set_cursor_position(SENTINEL).unwrap();
            terminal.draw(|f| space::tui::ui::view(&app, f)).unwrap();
            assert_eq!(
                terminal.get_cursor_position().unwrap(),
                SENTINEL,
                "{}: the frame set a cursor while help was open, so it is \
                 painted over the overlay",
                label
            );
        }

        // 1. A fuzzy picker query (fuzzy_picker::render).
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('r')));
        cursor_with_help(app, "repo search picker");

        // 2. A text input dialog (render_text_input_dialog).
        let env = TestEnv::new();
        let repo = env.create_repo("repo-a");
        let mut app = test_app_with_config(config_from_env(&env), vec![], vec![repo]);
        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Char('w')));
        cursor_with_help(app, "create flow name input");

        // 3. The config editor while editing.
        let env = TestEnv::new();
        let mut app = test_app_with_config(config_from_env(&env), vec![], vec![]);
        app.handle_key(shift_key(KeyCode::Char('S')));
        app.handle_key(key(KeyCode::Enter));
        cursor_with_help(app, "config editor editing");
    }

    // --- scrolling ---

    #[test]
    fn help_scrolls_and_clamps_at_both_ends() {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        // Draw first: `viewport` is only written by the renderer, so without a
        // frame the handler falls back to a 1-row viewport and the real clamp
        // is never exercised.
        let _ = render_text(&app, 80, 24);

        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.help.as_ref().unwrap().scroll, 0, "Home reaches the top");

        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(
            app.help.as_ref().unwrap().scroll,
            1,
            "j scrolls down one row"
        );
        app.handle_key(key(KeyCode::Char('k')));
        assert_eq!(app.help.as_ref().unwrap().scroll, 0);

        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(
            app.help.as_ref().unwrap().scroll,
            0,
            "PgUp clamps at the top"
        );

        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(
            app.help.as_ref().unwrap().scroll,
            10,
            "PgDn pages by the shared PAGE_ROWS"
        );
    }

    /// Every group must be reachable at the documented minimum terminal size.
    #[test]
    fn every_group_is_reachable_by_scrolling_at_eighty_by_twenty_four() {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        app.handle_key(key(KeyCode::Home));

        let mut seen: Vec<&str> = Vec::new();
        for _ in 0..80 {
            let rendered = render_text(&app, 80, 24);
            for group in keybindings::all_groups() {
                if rendered.contains(group.name) && !seen.contains(&group.name) {
                    seen.push(group.name);
                }
            }
            app.handle_key(key(KeyCode::PageDown));
        }
        let missing: Vec<&str> = keybindings::all_groups()
            .iter()
            .map(|g| g.name)
            .filter(|n| !seen.contains(n))
            .collect();
        assert!(
            missing.is_empty(),
            "these groups can never be seen at 80x24: {:?}",
            missing
        );
    }

    #[test]
    fn help_opens_on_the_group_for_the_screen_it_was_reached_from() {
        let mut app = gitops_menu_app();
        app.handle_key(key(KeyCode::Char('?')));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains(keybindings::GIT_OPS_NAME),
            "help opened from the git-ops menu must land on its own group, got:\n{}",
            rendered
        );
    }

    #[test]
    fn help_opened_from_the_repo_pane_lands_on_the_repo_pane_group() {
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.focus = Pane::Right;
        app.handle_key(key(KeyCode::Char('?')));
        let rendered = render_text(&app, 80, 24);
        assert!(
            rendered.contains(keybindings::REPO_PANE_NAME),
            "help from the repo pane must land on the Repo Pane group, got:\n{}",
            rendered
        );
    }

    /// Every group `landing_group` can return must exist in the registry, or
    /// help silently opens at the top instead of on the screen's own group.
    /// The two share one set of constants, so this pins that they stay shared.
    #[test]
    fn every_landing_group_exists_in_the_registry() {
        let names: Vec<&str> = keybindings::all_groups().iter().map(|g| g.name).collect();
        for landing in [
            keybindings::WORKSPACE_PANE_NAME,
            keybindings::REPO_PANE_NAME,
            keybindings::REPO_PICKER_NAME,
            keybindings::SYNC_REPORT_NAME,
            keybindings::CREATING_LOG_NAME,
            keybindings::CREATE_ADD_FLOW_NAME,
            keybindings::SPACE_REPO_PICKERS_NAME,
            keybindings::DELETE_CONFIRM_NAME,
            keybindings::CONFIG_EDITOR_NAME,
            keybindings::DIFF_VIEWER_NAME,
            keybindings::SWITCH_BRANCH_NAME,
            keybindings::GIT_OPS_NAME,
        ] {
            assert!(
                names.contains(&landing),
                "landing group {:?} is not in the registry: {:?}",
                landing,
                names
            );
        }
    }

    /// Landing behaviour for screens the first two landing tests did not cover.
    #[test]
    fn help_lands_on_the_right_group_from_more_screens() {
        // Delete confirm.
        let ws = common::workspace_with_repos(&["repo-a"]);
        let mut app = test_app(vec![ws], vec![]);
        app.handle_key(key(KeyCode::Char('d')));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(
            render_text(&app, 80, 24).contains(keybindings::DELETE_CONFIRM_NAME),
            "help from the delete confirm must land on its own group"
        );

        // Config editor, not editing.
        let env = TestEnv::new();
        let mut app = test_app_with_config(config_from_env(&env), vec![], vec![]);
        app.handle_key(shift_key(KeyCode::Char('S')));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(
            render_text(&app, 80, 24).contains(keybindings::CONFIG_EDITOR_NAME),
            "help from the config editor must land on its own group"
        );
    }

    /// The overlay renders only the visible window, so the windowing has to be
    /// exact: an off-by-one shifts what is shown without changing anything a
    /// content-sampling test would notice. The reference walk here is written
    /// out independently of the implementation.
    #[test]
    fn the_help_window_shows_exactly_the_requested_rows() {
        use space::tui::keybindings::{help_rows, rendered_row_count, HelpRow};

        // An independent expansion of the whole registry.
        let mut expected: Vec<HelpRow> = Vec::new();
        for (i, g) in keybindings::all_groups().iter().enumerate() {
            if i > 0 {
                expected.push(HelpRow::Gap);
            }
            expected.push(HelpRow::Header(g.name));
            for b in g.bindings {
                expected.push(HelpRow::Binding(b));
            }
        }
        let total = rendered_row_count();
        assert_eq!(expected.len(), total, "reference walk must match the count");

        // Every window, at several sizes, must equal the matching slice.
        for visible in [1usize, 2, 7, 21, 40] {
            for offset in 0..=total {
                let got = help_rows(offset, visible);
                let want = &expected[offset.min(total)..(offset + visible).min(total)];
                assert_eq!(
                    got.len(),
                    want.len(),
                    "offset {} visible {}: wrong row count",
                    offset,
                    visible
                );
                assert_eq!(
                    got.as_slice(),
                    want,
                    "offset {} visible {}: wrong rows",
                    offset,
                    visible
                );
            }
        }

        // A zero-height window yields nothing rather than panicking.
        assert!(help_rows(0, 0).is_empty());
    }

    // --- registry completeness and layout ---

    /// Deliberately spelled with literals, not the shared constants: this is
    /// the tripwire that makes a group rename a visible, failing decision
    /// rather than a silent one.
    #[test]
    fn the_registry_documents_every_screen() {
        let names: Vec<&str> = keybindings::all_groups().iter().map(|g| g.name).collect();
        for expected in [
            "Navigation",
            "Workspace Pane",
            "Repo Pane",
            "Repo Picker",
            "Create / Add Flow",
            "Creating Log",
            "Delete Confirm",
            "Switch Branch",
            "Config Editor",
            "Space & Repo Pickers",
            "Git Operations",
            "Diff Viewer",
            "Sync Report",
            "Help Overlay",
            "General",
        ] {
            assert!(
                names.contains(&expected),
                "the registry is missing the {:?} group; it has {:?}",
                expected,
                names
            );
        }
    }

    #[test]
    fn f1_is_documented_as_a_general_key() {
        let general = keybindings::all_groups()
            .iter()
            .find(|g| g.name == "General")
            .unwrap();
        assert!(
            general.bindings.iter().any(|b| b.key.contains("F1")),
            "F1 works everywhere, so it belongs in General"
        );
    }

    #[test]
    fn git_operations_is_documented_in_the_readme_and_the_guide() {
        let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
            .expect("README.md");
        let guide = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/GUIDE.md"))
            .expect("docs/GUIDE.md");
        for (name, text) in [("README.md", &readme), ("docs/GUIDE.md", &guide)] {
            assert!(
                text.contains("| `G` |"),
                "{} must document the G git-operations key",
                name
            );
        }
    }

    /// The overlay pads the key column, so a key and its description can never
    /// run together, whatever the key's length.
    #[test]
    fn every_row_keeps_a_gap_between_key_and_description() {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        app.handle_key(key(KeyCode::Home));
        for _ in 0..40 {
            let rendered = render_text(&app, 100, 40);
            for group in keybindings::all_groups() {
                for binding in group.bindings {
                    if let Some(line) = rendered
                        .lines()
                        .find(|l| l.contains(binding.key) && l.contains(binding.desc))
                    {
                        let joined = format!("{}{}", binding.key, binding.desc);
                        assert!(
                            !line.contains(&joined),
                            "{:?} runs into its description: {:?}",
                            binding.key,
                            line.trim()
                        );
                    }
                }
            }
            app.handle_key(key(KeyCode::PageDown));
        }
    }

    /// The registry test asserts a 54-column budget. That is only meaningful if
    /// the dialog is never narrower than 56, which is what the width floor now
    /// guarantees at every width where the overlay is drawn.
    #[test]
    fn the_dialog_is_never_narrower_than_the_budget_the_registry_test_asserts() {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        for width in [56u16, 60, 72, 80, 120] {
            let rendered = render_text(&app, width, 40);
            let widest = rendered
                .lines()
                .filter(|l| l.contains('│') || l.contains('╭'))
                .map(|l| l.trim_end().chars().count())
                .max()
                .unwrap_or(0);
            assert!(
                widest >= 56,
                "at {} columns the help dialog is only {} wide, under the 56 the registry test assumes",
                width,
                widest
            );
        }
    }

    /// The footer carries the only on-screen hint that the list scrolls, so it
    /// must never be clipped by the dialog it sits in.
    #[test]
    fn the_overlay_footer_is_never_clipped() {
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        for jump in [KeyCode::Home, KeyCode::PageDown, KeyCode::End] {
            app.handle_key(key(jump));
            let rendered = render_text(&app, 80, 24);
            let footer = rendered
                .lines()
                .find(|l| l.contains("close"))
                .unwrap_or_else(|| panic!("no footer after {:?}", jump));
            assert!(
                footer.contains("Esc/q/?/F1 close") || footer.contains("Esc / q / ? / F1 to close"),
                "footer clipped after {:?}: {:?}",
                jump,
                footer.trim_end()
            );
        }
    }

    /// The overlay is modal, but Ctrl-C is checked before it: the documented
    /// force quit must not become unreachable behind help.
    #[test]
    fn ctrl_c_still_quits_from_under_the_overlay() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit, "Ctrl-C must still quit while help is open");
    }

    /// The footer carries the only on-screen hint that the list scrolls, so it
    /// must fit the narrowest dialog the code can draw. Derived from the
    /// registry rather than asserted in prose: the widest form uses the largest
    /// row numbers `rendered_row_count()` can produce, so this fails if the
    /// registry grows past the digits the footer budgeted for, or if anyone
    /// adds a word to the hint.
    #[test]
    fn overlay_footer_fits_the_dialog() {
        let total = keybindings::rendered_row_count();
        // The widest scrolled form: "rows <start>-<total> of <total>" with the
        // largest numbers, plus the separators and the close hint.
        let widest = format!(
            "rows {}-{} of {}  \u{b7}  \u{2191}\u{2193} scroll  \u{b7}  Esc/q/?/F1 close",
            total, total, total
        );
        let interior = 54; // 56-column dialog floor, minus two border columns
        assert!(
            UnicodeWidthStr::width(widest.as_str()) <= interior,
            "the widest footer is {} columns, over the {}-column interior: {:?}",
            UnicodeWidthStr::width(widest.as_str()),
            interior,
            widest
        );

        // And it is not clipped in practice, including on a short terminal
        // where the row numbers are largest.
        let mut app = test_app(vec![], vec![]);
        app.handle_key(key(KeyCode::Char('?')));
        for (w, h) in [(80u16, 24u16), (80, 10), (80, 8)] {
            app.handle_key(key(KeyCode::End));
            let rendered = render_text(&app, w, h);
            let footer = rendered
                .lines()
                .find(|l| l.contains("close"))
                .unwrap_or_else(|| panic!("no footer at {}x{}", w, h));
            assert!(
                footer.contains("Esc/q/?/F1 close") || footer.contains("Esc / q / ? / F1 to close"),
                "footer clipped at {}x{}: {:?}",
                w,
                h,
                footer.trim_end()
            );
        }
    }

    // --- the key bar ---

    #[test]
    fn the_key_bar_always_shows_the_help_key_at_eighty_columns() {
        for pane in [Pane::Left, Pane::Right] {
            let mut app = test_app(vec![common::workspace_with_repos(&["repo-a"])], vec![]);
            app.focus = pane;
            let rendered = render_text(&app, 80, 24);
            let bar = rendered.lines().last().unwrap();
            assert!(
                bar.contains("? help"),
                "the {:?} key bar drops the help key at 80 columns: {:?}",
                pane,
                bar.trim_end()
            );
            assert!(
                bar.trim_end().chars().count() <= 80,
                "the key bar must not overflow the terminal"
            );
        }
    }

    /// The fit arithmetic, tested through the pure seam rather than through a
    /// render. `render_dashboard` returns before the bar below 80 columns and
    /// ratatui clips a `Paragraph` at its area, so a test that renders and
    /// measures can never observe an overflow: it would pass whatever the
    /// arithmetic did.
    #[test]
    fn the_key_bar_fits_the_width_and_drops_the_gateways_last() {
        use space::tui::ui::{fit_key_bar, key_bar_width};

        for pane in [Pane::Left, Pane::Right] {
            let bindings = keybindings::key_bar_bindings(pane);
            let gateways = 2; // `?` and `q`

            for width in [5usize, 12, 20, 40, 60, 79, 80, 90, 100, 140, 200] {
                let entries = fit_key_bar(bindings, width);
                let rendered = key_bar_width(&entries);

                // The gateways are always present, and are the only entries
                // allowed to exceed the width (the terminal clips them).
                assert_eq!(
                    entries
                        .iter()
                        .filter(|b| b.key == "?" || b.key == "q")
                        .count(),
                    gateways,
                    "{:?} @{}: a gateway key was dropped: {:?}",
                    pane,
                    width,
                    entries.iter().map(|b| b.key).collect::<Vec<_>>()
                );
                if entries.len() > gateways {
                    assert!(
                        rendered <= width,
                        "{:?} @{}: bar is {} columns wide: {:?}",
                        pane,
                        width,
                        rendered,
                        entries.iter().map(|b| b.key).collect::<Vec<_>>()
                    );
                }
                // Entries are admitted in registry order, never reordered.
                let order: Vec<&str> = entries
                    .iter()
                    .filter(|b| b.key != "?" && b.key != "q")
                    .map(|b| b.key)
                    .collect();
                let expected: Vec<&str> = bindings
                    .iter()
                    .filter(|b| b.key != "?" && b.key != "q")
                    .map(|b| b.key)
                    .take(order.len())
                    .collect();
                assert_eq!(order, expected, "{:?} @{}: entries reordered", pane, width);
            }

            // Wide enough for everything: nothing is dropped.
            let all = fit_key_bar(bindings, 400);
            assert_eq!(
                all.len(),
                bindings.len(),
                "{:?}: 400 columns fits all",
                pane
            );
        }
    }
}
