#![allow(clippy::disallowed_methods)] // fixtures start git directly (ADR 0002)

//! The create and add flows with a strategy chosen per repo (ticket 55),
//! driven through the App on real repos: the fork after the sync report, the
//! loop over the repos, and what each repo's worktree ends up on.

mod common;

use common::{key, test_app_with_config, TestEnv};
use ratatui::crossterm::event::KeyCode;
use space::core::config::{RepoConfig, SpaceConfig, WorkspaceConfig};
use space::core::workspace::BranchStrategy;
use space::tui::actions::{RepoPlan, ScreenAction, ScreenContext};
use space::tui::app::{App, Screen};
use space::tui::screens::add::{AddStage, AddState};
use space::tui::screens::create::CreateStage;
use std::path::{Path, PathBuf};

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A local branch `name` whose tip is committed at `date`, so the recent
/// list's order is fixed: it sorts by last commit time.
fn dated_branch(repo: &Path, name: &str, date: &str) {
    git(repo, &["checkout", "-q", "-b", name]);
    let out = std::process::Command::new("git")
        .args(["commit", "-q", "--allow-empty", "-m", name])
        .env("GIT_COMMITTER_DATE", date)
        .env("GIT_AUTHOR_DATE", date)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    git(repo, &["checkout", "-q", "main"]);
}

fn app_for(env: &TestEnv, repos: &[PathBuf]) -> App {
    let config = SpaceConfig {
        repos: RepoConfig {
            roots: vec![env.repos_dir.clone()],
            max_depth: 3,
            cache_age_secs: 3600,
        },
        workspaces: WorkspaceConfig {
            dir: env.workspaces_dir.clone(),
        },
    };
    test_app_with_config(config, vec![], repos.to_vec())
}

/// Wait for the sync report to finish, then press Enter on it.
fn finish_sync(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        app.poll_sync_result();
        let done = match &app.screen {
            Screen::CreateWorkspace(st) => st.stage != CreateStage::Syncing || st.report.done,
            Screen::AddRepos(st) => st.stage != AddStage::Syncing || st.report.done,
            _ => true,
        };
        if done {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the sync must end");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    app.handle_key(key(KeyCode::Enter));
}

/// The create flow for space `ws` with `repos` picked, past the sync report.
fn create_past_sync(env: &TestEnv, ws: &str, repos: &[PathBuf]) -> App {
    let mut app = app_for(env, repos);
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.ws_name = tui_input::Input::default().with_value(ws.to_string());
        st.stage = CreateStage::PickRepos;
        st.picker.toggle_paths(repos);
    }
    app.handle_key(key(KeyCode::Enter));
    finish_sync(&mut app);
    app
}

/// The add flow for space `ws` with `repos` picked, past the sync report.
fn add_past_sync(env: &TestEnv, ws: &str, repos: &[PathBuf]) -> App {
    let mut app = app_for(env, repos);
    let mut st = AddState::new(ws.to_string(), repos.to_vec(), vec![]);
    st.picker.toggle_paths(repos);
    app.screen = Screen::AddRepos(st);
    app.handle_key(key(KeyCode::Enter));
    finish_sync(&mut app);
    app
}

fn create_stage(app: &App) -> CreateStage {
    match &app.screen {
        Screen::CreateWorkspace(st) => st.stage.clone(),
        _ => panic!("expected the create screen"),
    }
}

fn create_state(app: &App) -> &space::tui::screens::create::CreateState {
    match &app.screen {
        Screen::CreateWorkspace(st) => st,
        _ => panic!("expected the create screen"),
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(key(code));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn recent_names(app: &App) -> Vec<String> {
    create_state(app)
        .recent_branches
        .iter()
        .map(|b| b.name.clone())
        .collect()
}

/// Press a key on the create screen directly and return its action, so a
/// run's parameters can be read without the App spawning the worker.
fn create_action(app: &mut App, code: KeyCode) -> ScreenAction {
    let config = app.config.clone();
    let ctx = ScreenContext {
        config: &config,
        creating_in_flight: false,
    };
    match &mut app.screen {
        Screen::CreateWorkspace(st) => st.handle_key(key(code), &ctx),
        _ => panic!("expected the create screen"),
    }
}

fn plans_of(action: ScreenAction) -> Vec<RepoPlan> {
    match action {
        ScreenAction::ExecuteWorktreeFlow(p) => p.plans,
        _ => panic!("expected the run to start"),
    }
}

/// Down `n` times on the strategy picker.
fn down(app: &mut App, n: usize) {
    for _ in 0..n {
        press(app, KeyCode::Down);
    }
}

/// Choose per repo on the fork.
fn per_repo(app: &mut App) {
    assert_eq!(create_stage(app), CreateStage::PickPlanMode);
    press(app, KeyCode::Down);
    press(app, KeyCode::Enter);
    assert_eq!(create_stage(app), CreateStage::PickBranchStrategy);
}

/// Detached HEAD for the repo the picker is on.
fn detached(app: &mut App) -> ScreenAction {
    assert_eq!(create_stage(app), CreateStage::PickBranchStrategy);
    down(app, 2);
    assert_eq!(create_state(app).branch_strategy_idx, 2);
    create_action(app, KeyCode::Enter)
}

#[test]
fn a_single_repo_create_shows_no_fork() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let app = create_past_sync(&env, "ws", std::slice::from_ref(&alpha));
    assert_eq!(
        create_stage(&app),
        CreateStage::PickBranchStrategy,
        "one repo goes straight to the strategy picker"
    );
}

#[test]
fn the_fork_esc_returns_to_the_repo_picker() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_past_sync(&env, "ws", &repos);
    assert_eq!(create_stage(&app), CreateStage::PickPlanMode);
    press(&mut app, KeyCode::Esc);
    assert_eq!(create_stage(&app), CreateStage::PickRepos);
}

/// Ticket 01 part (b). Each repo's recent branches, and the full picker
/// behind "Show more...", are that repo's own. On 8863c8b the one list was
/// the alphabetically first repo's: with alpha and bravo selected it read
/// `["alpha-only", "main"]` for both.
#[test]
fn each_repo_offers_its_own_branches() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let bravo = env.create_repo("bravo");
    git(&alpha, &["branch", "alpha-only"]);
    git(&bravo, &["branch", "bravo-only"]);
    let mut app = create_past_sync(&env, "ws", &[alpha, bravo]);
    per_repo(&mut app);

    let alpha_list = recent_names(&app);
    assert!(alpha_list.contains(&"alpha-only".to_string()));
    assert!(!alpha_list.contains(&"bravo-only".to_string()));

    detached(&mut app);
    assert_eq!(
        create_state(&app).plan.current,
        1,
        "the loop moved to bravo"
    );
    let bravo_list = recent_names(&app);
    assert!(
        bravo_list.contains(&"bravo-only".to_string()),
        "bravo's own branch is offered for bravo, got {:?}",
        bravo_list
    );
    assert!(
        !bravo_list.contains(&"alpha-only".to_string()),
        "alpha's branch is not offered for bravo, got {:?}",
        bravo_list
    );

    // "Show more..." opens bravo's branches too.
    let show_more = 3 + bravo_list.len();
    down(&mut app, show_more);
    press(&mut app, KeyCode::Enter);
    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::PickBranch);
    let picker = st.branch_picker.as_ref().expect("the picker opened");
    assert!(picker.prompt.contains("bravo"), "got {:?}", picker.prompt);
    let names: Vec<&str> = picker.all_items.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"bravo-only") && !names.contains(&"alpha-only"));
}

/// Failure mode 1. The cursor left on alpha's fifth recent branch (index 7)
/// would index past bravo's one-branch list on Enter. Each repo opens with
/// its own cursor, so Enter on bravo does what its highlighted row says.
#[test]
fn the_strategy_cursor_resets_between_repos() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let bravo = env.create_repo("bravo");
    for (i, name) in ["a1", "a2", "a3", "a4", "a5"].iter().enumerate() {
        dated_branch(&alpha, name, &format!("2090-01-0{}T00:00:00Z", i + 1));
    }
    let mut app = create_past_sync(&env, "ws", &[alpha, bravo]);
    per_repo(&mut app);
    assert_eq!(
        recent_names(&app).len(),
        5,
        "alpha lists five recent branches"
    );

    down(&mut app, 7);
    assert_eq!(create_state(&app).branch_strategy_idx, 7);
    press(&mut app, KeyCode::Enter);

    let st = create_state(&app);
    assert_eq!(st.plan.current, 1, "the loop moved to bravo");
    assert_eq!(st.recent_branches.len(), 1, "bravo has only main");
    assert_eq!(
        st.branch_strategy_idx, 0,
        "bravo opens on its first row, not on alpha's cursor"
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        create_stage(&app),
        CreateStage::EnterBranchName,
        "Enter on the first row opens the branch name stage"
    );
}

/// Failure mode 7, and the rule: each repo has its own New-branch field, and
/// a repo not chosen yet opens on the space name.
#[test]
fn a_name_typed_for_one_repo_is_not_the_next_repos() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_past_sync(&env, "ws", &repos);
    per_repo(&mut app);

    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::EnterBranchName);
    app.handle_key(ratatui::crossterm::event::KeyEvent::new(
        KeyCode::Char('u'),
        ratatui::crossterm::event::KeyModifiers::CONTROL,
    ));
    type_text(&mut app, "feat-x");
    press(&mut app, KeyCode::Enter);

    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::PickBranchStrategy);
    assert_eq!(st.plan.current, 1);
    assert_eq!(
        st.new_branch_name(),
        "ws",
        "bravo's New-branch row names the space, not alpha's typed name"
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_state(&app).branch_name_input.value(), "ws");
}

/// Failure mode 2. Esc at a repo boundary goes back one repo, to its picker
/// as it was left; at the first repo, to the fork.
#[test]
fn esc_steps_back_to_the_previous_repo_as_it_was_left() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let bravo = env.create_repo("bravo");
    dated_branch(&alpha, "a-new", "2090-01-01T00:00:00Z");
    let mut app = create_past_sync(&env, "ws", &[alpha, bravo]);
    per_repo(&mut app);

    // alpha: its second recent branch, "main" (index 4).
    down(&mut app, 4);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_state(&app).plan.current, 1);

    press(&mut app, KeyCode::Esc);
    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::PickBranchStrategy);
    assert_eq!(st.plan.current, 0, "back on alpha");
    assert_eq!(st.branch_strategy_idx, 4, "on the row alpha was left on");
    assert_eq!(
        st.plan.picker_title(&st.selected_repos, 28),
        " Branch Strategy: alpha (1 of 2) "
    );

    press(&mut app, KeyCode::Esc);
    assert_eq!(create_stage(&app), CreateStage::PickPlanMode);
}

#[test]
fn a_two_repo_space_gets_each_repo_its_own_strategy() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let bravo = env.create_repo("bravo");
    dated_branch(&bravo, "topic", "2090-01-01T00:00:00Z");
    let mut app = create_past_sync(&env, "ws", &[alpha.clone(), bravo.clone()]);
    per_repo(&mut app);

    // alpha: a new branch named in the name stage.
    press(&mut app, KeyCode::Enter);
    app.handle_key(ratatui::crossterm::event::KeyEvent::new(
        KeyCode::Char('u'),
        ratatui::crossterm::event::KeyModifiers::CONTROL,
    ));
    type_text(&mut app, "feat-a");
    press(&mut app, KeyCode::Enter);
    // bravo: its most recent branch, "topic".
    assert_eq!(recent_names(&app)[0], "topic");
    down(&mut app, 3);
    let plans = plans_of(create_action(&mut app, KeyCode::Enter));
    assert_eq!(
        plans,
        vec![
            RepoPlan {
                path: alpha,
                strategy: BranchStrategy::NewBranch("feat-a".to_string()),
            },
            RepoPlan {
                path: bravo,
                strategy: BranchStrategy::ExistingBranch("topic".to_string()),
            },
        ]
    );
}

/// Acceptance: one strategy for all behaves as it always has, and choosing
/// the same strategy repo by repo reaches the same run.
#[test]
fn one_strategy_for_all_and_per_repo_reach_the_same_run() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];

    let mut all = create_past_sync(&env, "ws", &repos);
    press(&mut all, KeyCode::Enter);
    assert_eq!(
        create_state(&all).plan.picker_title(&repos, 28),
        " Branch Strategy: all 2 repos "
    );
    let all_plans = plans_of(detached(&mut all));

    let mut each = create_past_sync(&env, "ws", &repos);
    per_repo(&mut each);
    assert!(matches!(detached(&mut each), ScreenAction::Continue));
    let each_plans = plans_of(detached(&mut each));

    assert_eq!(all_plans, each_plans);
    assert!(all_plans
        .iter()
        .all(|p| p.strategy == BranchStrategy::DetachedHead));
    assert_eq!(all_plans.len(), 2);
}

/// Failure mode 11: the add flow's fork counts the repos being added.
#[test]
fn the_add_flow_forks_on_the_repos_being_added_and_lists_each_repos_branches() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let bravo = env.create_repo("bravo");
    git(&bravo, &["branch", "bravo-only"]);

    let app = add_past_sync(&env, "ws", std::slice::from_ref(&alpha));
    match &app.screen {
        Screen::AddRepos(st) => assert_eq!(st.stage, AddStage::PickBranchStrategy),
        _ => panic!("expected the add screen"),
    }

    let mut app = add_past_sync(&env, "ws", &[alpha, bravo]);
    match &app.screen {
        Screen::AddRepos(st) => assert_eq!(st.stage, AddStage::PickPlanMode),
        _ => panic!("expected the add screen"),
    }
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    down(&mut app, 2);
    press(&mut app, KeyCode::Enter);
    match &app.screen {
        Screen::AddRepos(st) => {
            assert_eq!(st.plan.current, 1);
            assert!(st.recent_branches.iter().any(|b| b.name == "bravo-only"));
        }
        _ => panic!("expected the add screen"),
    }
}

#[test]
fn question_mark_opens_help_on_the_fork() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_past_sync(&env, "ws", &repos);
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help.is_some(), "the fork types nothing, so ? is help");

    let mut app = add_past_sync(&env, "ws", &repos);
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help.is_some());
}
