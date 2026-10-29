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

/// Drive the Creating worker to the end, as the run loop does each frame.
fn pump_creating(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.creating_in_flight() {
        app.poll_create_result();
        assert!(std::time::Instant::now() < deadline, "the run must end");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// The branch a worktree is on, or `HEAD` when detached.
fn head_of(worktree: &Path) -> String {
    assert!(worktree.exists(), "{:?} was not created", worktree);
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree)
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?} is not a worktree", worktree);
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn render(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| space::tui::ui::view(app, frame))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
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
    let mut app = create_past_sync(&env, "ws", std::slice::from_ref(&alpha));
    assert_eq!(
        create_stage(&app),
        CreateStage::PickBranchStrategy,
        "one repo goes straight to the strategy picker"
    );
    assert_eq!(
        create_state(&app)
            .plan
            .picker_title(std::slice::from_ref(&alpha), 28),
        " Branch Strategy "
    );
    assert_eq!(
        plans_of(detached(&mut app)).len(),
        1,
        "and from it straight to the run, with no review table"
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
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);
    let st = create_state(&app);
    assert_eq!(
        st.plan.plans(&st.selected_repos),
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

    // The table opens on the row that starts the run.
    press(&mut app, KeyCode::Enter);
    pump_creating(&mut app);
    assert!(matches!(app.screen, Screen::Dashboard), "the run completes");
    let space = env.workspaces_dir.join("ws");
    assert_eq!(head_of(&space.join("alpha")), "feat-a");
    assert_eq!(head_of(&space.join("bravo")), "topic");
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
    assert!(matches!(detached(&mut each), ScreenAction::Continue));
    assert_eq!(create_stage(&each), CreateStage::ReviewPlan);
    let each_plans = plans_of(create_action(&mut each, KeyCode::Enter));

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

/// Per repo, through to the review table: alpha detached, bravo on a new
/// branch named after the space.
fn create_on_review(env: &TestEnv, repos: &[PathBuf]) -> App {
    let mut app = create_past_sync(env, "ws", repos);
    per_repo(&mut app);
    detached(&mut app);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);
    app
}

/// Acceptance: Enter on a row of the table reopens that repo's choice where
/// it was left, and choosing returns to the table; Esc on the reopened
/// picker returns to the table with the choice unchanged.
#[test]
fn enter_on_a_review_row_reopens_that_repos_choice() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_on_review(&env, &repos);
    assert_eq!(create_state(&app).plan.review_cursor, 2, "on the start row");

    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::PickBranchStrategy);
    assert_eq!(st.branch_strategy_idx, 2, "alpha's detached HEAD row");
    assert_eq!(
        st.plan.picker_title(&st.selected_repos, 28),
        " Branch Strategy: alpha (1 of 2) "
    );

    press(&mut app, KeyCode::Esc);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);
    let st = create_state(&app);
    assert_eq!(
        st.plan.strategy_of(&repos[0]),
        Some(&BranchStrategy::DetachedHead),
        "Esc changed nothing"
    );

    // Reopen alpha and give it a new branch instead.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::EnterBranchName);
    press(&mut app, KeyCode::Enter);
    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::ReviewPlan, "back on the table");
    assert_eq!(
        st.plan.strategy_of(&repos[0]),
        Some(&BranchStrategy::NewBranch("ws".to_string()))
    );
    assert_eq!(
        st.plan.strategy_of(&repos[1]),
        Some(&BranchStrategy::NewBranch("ws".to_string())),
        "bravo is as it was"
    );
}

/// Decision 1: Esc on the table steps back to the last repo's choice.
#[test]
fn esc_on_the_review_table_goes_back_to_the_last_repo() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_on_review(&env, &repos);
    press(&mut app, KeyCode::Esc);
    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::PickBranchStrategy);
    assert_eq!(st.plan.current, 1, "bravo, the last repo");
    assert_eq!(st.branch_strategy_idx, 0, "on the row bravo was left on");
    press(&mut app, KeyCode::Esc);
    assert_eq!(
        create_state(&app).plan.current,
        0,
        "and from there the loop walks back as usual"
    );
}

/// Acceptance: the table lists every repo with its strategy and reads at the
/// documented 80-column minimum. A long repo name is cut to a third of the
/// dialog so the strategy beside it is still on the row, and the start row
/// is there in full.
#[test]
fn the_review_table_reads_at_eighty_columns() {
    let env = TestEnv::new();
    let long = "the-repository-with-a-name-far-longer-than-the-column";
    let repos = [env.create_repo("api"), env.create_repo(long)];
    let mut app = create_on_review(&env, &repos);
    press(&mut app, KeyCode::Up);

    let rows = render(&app, 80, 24);
    let dialog_row = |needle: &str| {
        rows.iter()
            .find(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("no row shows {:?}:\n{}", needle, rows.join("\n")))
            .clone()
    };
    // Each row's text sits between two of the dialog's borders, whatever the
    // dashboard draws behind it.
    let cell = |row: &str, needle: &str| -> String {
        row.split('\u{2502}')
            .find(|s| s.contains(needle))
            .unwrap_or_else(|| panic!("{:?} is not inside the dialog: {:?}", needle, row))
            .to_string()
    };

    assert!(dialog_row(" Review: 2 repos ").contains("Review"));
    let api = cell(&dialog_row("  api "), "api");
    assert!(api.contains("detached HEAD"), "got {:?}", api);
    let long_row = dialog_row("the-repository-wi");
    let long_cell = cell(&long_row, "the-repository-wi");
    assert!(
        long_cell.starts_with("> the-repository-wi") && long_cell.contains("..."),
        "the long name is cut, got {:?}",
        long_cell
    );
    assert!(
        long_cell.contains("new branch 'ws'"),
        "the strategy is still on the row, got {:?}",
        long_cell
    );
    assert!(cell(&dialog_row("Create 2 repos"), "Create").contains("Create 2 repos"));
    assert!(dialog_row("ESC back").contains("ENTER change the repo, or start"));
    assert!(rows
        .iter()
        .all(|r| unicode_width::UnicodeWidthStr::width(r.as_str()) <= 80));
}

#[test]
fn question_mark_on_the_review_table_opens_its_help() {
    let env = TestEnv::new();
    let repos = [env.create_repo("alpha"), env.create_repo("bravo")];
    let mut app = create_on_review(&env, &repos);
    press(&mut app, KeyCode::Char('?'));
    assert!(app.help.is_some(), "the table types nothing, so ? is help");
    assert_eq!(
        space::tui::screens::help::landing_group(&app.screen, app.focus),
        space::tui::keybindings::PLAN_REVIEW_NAME
    );
}

/// Three repos with `topic`, checked out in bravo's source only, and the
/// create flow for space `topic` past its sync report.
fn topic_refused_by_bravo(env: &TestEnv) -> (App, [PathBuf; 3]) {
    let repos = [
        env.create_repo("alpha"),
        env.create_repo("bravo"),
        env.create_repo("charlie"),
    ];
    git(&repos[0], &["branch", "topic"]);
    git(&repos[1], &["checkout", "-q", "-b", "topic"]);
    git(&repos[2], &["branch", "topic"]);
    (create_past_sync(env, "topic", &repos), repos)
}

/// One strategy for all three: "Existing branch 'topic'", the space name.
fn existing_topic_for_all(app: &mut App) {
    press(app, KeyCode::Enter);
    down(app, 1);
    press(app, KeyCode::Enter);
    pump_creating(app);
}

/// Decision 4, end to end on the installed git. A repo refused because its
/// branch is checked out elsewhere does not stop the run: charlie, after
/// the refused bravo, is created too. The run returns to the review table
/// with bravo marked; alpha and charlie are in place and closed to change;
/// and the retry after bravo is given another strategy creates bravo alone.
#[test]
fn a_refused_repo_does_not_stop_the_run_and_the_retry_creates_only_what_is_missing() {
    use space::tui::screens::branch_plan::RowOutcome;
    let env = TestEnv::new();
    let (mut app, [alpha, bravo, charlie]) = topic_refused_by_bravo(&env);
    existing_topic_for_all(&mut app);

    let space = env.workspaces_dir.join("topic");
    assert_eq!(head_of(&space.join("alpha")), "topic");
    assert_eq!(
        head_of(&space.join("charlie")),
        "topic",
        "the repo after the refusal is created: the refusal did not stop the run"
    );
    assert!(!space.join("bravo").exists());

    let st = create_state(&app);
    assert_eq!(st.stage, CreateStage::ReviewPlan);
    assert_eq!(st.plan.outcome_of(&alpha), Some(&RowOutcome::Placed));
    assert_eq!(st.plan.outcome_of(&bravo), Some(&RowOutcome::Refused));
    assert_eq!(st.plan.outcome_of(&charlie), Some(&RowOutcome::Placed));
    assert_eq!(st.plan.review_cursor, 1, "on bravo");

    let rows = render(&app, 80, 24).join("\n");
    assert!(rows.contains("Retry 1 repos"), "got:\n{}", rows);
    assert!(
        rows.contains("refused \u{b7} branch 'topic'"),
        "got:\n{}",
        rows
    );
    assert!(
        rows.contains("in place \u{b7} branch 'topic'"),
        "got:\n{}",
        rows
    );

    // charlie is in place: Enter on it reopens nothing.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);
    assert!(create_state(&app)
        .error
        .as_deref()
        .is_some_and(|e| e.contains("'charlie' is already in the space")));

    // bravo, detached this time.
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::PickBranchStrategy);
    down(&mut app, 1);
    press(&mut app, KeyCode::Enter);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);
    press(&mut app, KeyCode::Enter);
    pump_creating(&mut app);

    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(
        app.status_message.as_deref(),
        Some("Created workspace 'topic'"),
        "the retry created bravo and attempted nothing already in place"
    );
    assert_eq!(head_of(&space.join("bravo")), "HEAD", "bravo is detached");
    assert_eq!(head_of(&space.join("alpha")), "topic");
}

/// Esc on the table after a run leaves the flow and says what is in place.
#[test]
fn esc_on_the_table_after_a_run_leaves_with_what_is_in_place() {
    let env = TestEnv::new();
    let (mut app, _) = topic_refused_by_bravo(&env);
    existing_topic_for_all(&mut app);
    assert_eq!(create_stage(&app), CreateStage::ReviewPlan);

    press(&mut app, KeyCode::Esc);
    assert!(matches!(app.screen, Screen::Dashboard));
    assert_eq!(
        app.status_message.as_deref(),
        Some("2 of 3 repos in place. Press a to add the rest.")
    );
    assert!(
        app.workspaces.iter().any(|w| w.name == "topic"),
        "the dashboard lists the partial space"
    );
}
