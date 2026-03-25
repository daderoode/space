# Inline Recent Branches — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Show the top 5 most recently committed-to branches inline in the Branch Strategy dialog, with relative timestamps, so users can select a branch without opening the fuzzy picker.

**Architecture:** Extend `BranchInfo` with commit timestamps from git2, add a `recent_branches` field to both `CreateState` and `AddState` (populated at stage transition), update the shared `render_branch_strategy_picker()` to render the branch list with a "Show more..." fallback, and extend the key handlers' navigation and Enter logic to handle the dynamic item count.

**Tech Stack:** Rust, git2, ratatui

**Safety net:** 98 existing tests. Run `cargo test` after every task. Run `cargo clippy -- -D warnings && cargo fmt --check` before every commit.

---

### Task 1: Extend BranchInfo with last_commit_time

**Files:**
- Modify: `src/core/git.rs:12-17` (BranchInfo struct)
- Modify: `src/core/git.rs:78-94` (list_branches loop)
- Modify: `tests/git_test.rs:38-77` (lists_branches test)

**Step 1: Add the field to BranchInfo**

In `src/core/git.rs`, add `last_commit_time` to the struct:

```rust
#[derive(Debug)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
    pub last_commit_time: i64,
}
```

**Step 2: Populate it in list_branches**

In `src/core/git.rs`, inside the `for branch_result in repo.branches(None)?` loop (line 78), after computing `is_current`, resolve the commit time:

```rust
let last_commit_time = branch
    .get()
    .peel_to_commit()
    .map(|c| c.time().seconds())
    .unwrap_or(0);
branches.push(BranchInfo {
    name,
    is_remote,
    is_current,
    last_commit_time,
});
```

**Step 3: Update the existing test**

In `tests/git_test.rs`, the `lists_branches` test (line 38) asserts on `BranchInfo` fields. Add assertions for the new field after the existing assertions:

```rust
// Both branches should have a commit time (from init_repo's initial commit)
assert!(
    main_branch.last_commit_time > 0,
    "main branch should have a commit time"
);
assert!(
    feature_branch.last_commit_time > 0,
    "feature-x branch should have a commit time (inherited from main)"
);
```

**Step 4: Run tests**

Run: `cargo test`
Expected: all 98 tests pass

**Step 5: Commit**

```
git add src/core/git.rs tests/git_test.rs
git commit -m "feat: add last_commit_time to BranchInfo"
```

---

### Task 2: Add relative_time helper with unit tests

**Files:**
- Modify: `src/core/git.rs` (add function + `#[cfg(test)]` module)

**Step 1: Write the failing tests**

Add a `#[cfg(test)]` module at the bottom of `src/core/git.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn now_ts() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    #[test]
    fn relative_time_just_now() {
        assert_eq!(relative_time(now_ts()), "just now");
    }

    #[test]
    fn relative_time_minutes() {
        assert_eq!(relative_time(now_ts() - 300), "5 minutes ago");
    }

    #[test]
    fn relative_time_singular_minute() {
        assert_eq!(relative_time(now_ts() - 90), "1 minute ago");
    }

    #[test]
    fn relative_time_hours() {
        assert_eq!(relative_time(now_ts() - 7200), "2 hours ago");
    }

    #[test]
    fn relative_time_days() {
        assert_eq!(relative_time(now_ts() - 259200), "3 days ago");
    }

    #[test]
    fn relative_time_weeks() {
        assert_eq!(relative_time(now_ts() - 1209600), "2 weeks ago");
    }

    #[test]
    fn relative_time_months() {
        assert_eq!(relative_time(now_ts() - 7776000), "3 months ago");
    }

    #[test]
    fn relative_time_years() {
        assert_eq!(relative_time(now_ts() - 63072000), "2 years ago");
    }

    #[test]
    fn relative_time_zero_timestamp() {
        // Epoch 0 should produce a "years ago" result
        let result = relative_time(0);
        assert!(result.contains("years ago"), "got: {}", result);
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p space --lib git::tests`
Expected: FAIL — `relative_time` not found

**Step 3: Write the implementation**

Add this function to `src/core/git.rs`, above the `#[cfg(test)]` module:

```rust
/// Human-readable relative time string from a unix timestamp.
pub fn relative_time(unix_ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let delta = now - unix_ts;
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let m = delta / 60;
        if m == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", m)
        }
    } else if delta < 86400 {
        let h = delta / 3600;
        if h == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", h)
        }
    } else if delta < 604800 {
        let d = delta / 86400;
        if d == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", d)
        }
    } else if delta < 2592000 {
        let w = delta / 604800;
        if w == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", w)
        }
    } else if delta < 31536000 {
        let m = delta / 2592000;
        if m == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", m)
        }
    } else {
        let y = delta / 31536000;
        if y == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", y)
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p space --lib git::tests`
Expected: all 9 new tests pass

**Step 5: Run full test suite**

Run: `cargo test`
Expected: all tests pass (98 + 9 new = 107)

**Step 6: Commit**

```
git add src/core/git.rs
git commit -m "feat: add relative_time() helper for human-readable timestamps"
```

---

### Task 3: Add recent_branches field to state structs and populate on stage entry

**Files:**
- Modify: `src/tui/screens/create.rs:15-25` (struct), `src/tui/screens/create.rs:54-64` (constructor), `src/tui/screens/create.rs:132-164` (handle_name_workspace)
- Modify: `src/tui/screens/add.rs:15-25` (struct), `src/tui/screens/add.rs:47-57` (constructor), `src/tui/screens/add.rs:82-125` (handle_pick_repos)
- Modify: `tests/tui_test.rs` (new test)

**Step 1: Write the failing test for Create flow**

Add to `tests/tui_test.rs`, after `create_empty_name_rejected` (line 259):

```rust
#[test]
fn create_populates_recent_branches_on_strategy_entry() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("branchy-repo");

    // Create a second branch with a commit so there are 2 branches
    let out = std::process::Command::new("git")
        .args(["checkout", "-b", "feature-recent"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let out = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "feature commit"])
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
```

**Step 2: Run test to verify it fails**

Run: `cargo test create_populates_recent_branches`
Expected: FAIL — `recent_branches` field doesn't exist

**Step 3: Add field to CreateState**

In `src/tui/screens/create.rs`, add the field to the struct (after line 22, `picked_branch`):

```rust
pub recent_branches: Vec<crate::core::git::BranchInfo>,
```

And in the constructor `new()` (in the `Self { ... }` block, after `picked_branch: None,`):

```rust
recent_branches: vec![],
```

**Step 4: Add field to AddState**

In `src/tui/screens/add.rs`, same changes — add field to struct (after `picked_branch`) and initialize to `vec![]` in constructor.

**Step 5: Populate in CreateState::handle_name_workspace**

In `src/tui/screens/create.rs`, in `handle_name_workspace()`, after `self.stage = CreateStage::PickBranchStrategy;` (line 154), add:

```rust
// Fetch recent branches from first selected repo
if let Some(repo_path) = self.selected_repos.first() {
    if let Ok(mut branches) = crate::core::git::list_branches(repo_path) {
        branches.sort_by(|a, b| b.last_commit_time.cmp(&a.last_commit_time));
        branches.truncate(5);
        self.recent_branches = branches;
    }
}
```

**Step 6: Populate in AddState::handle_pick_repos**

In `src/tui/screens/add.rs`, in `handle_pick_repos()`, after `self.stage = AddStage::PickBranchStrategy;` (line 98), add the same block:

```rust
if let Some(repo_path) = self.selected_repos.first() {
    if let Ok(mut branches) = crate::core::git::list_branches(repo_path) {
        branches.sort_by(|a, b| b.last_commit_time.cmp(&a.last_commit_time));
        branches.truncate(5);
        self.recent_branches = branches;
    }
}
```

**Step 7: Run the test**

Run: `cargo test create_populates_recent_branches`
Expected: PASS

**Step 8: Run full test suite**

Run: `cargo test`
Expected: all tests pass

**Step 9: Commit**

```
git add src/tui/screens/create.rs src/tui/screens/add.rs tests/tui_test.rs
git commit -m "feat: add recent_branches field and populate on PickBranchStrategy entry"
```

---

### Task 4: Update navigation and Enter handling in handle_branch_strategy

**Files:**
- Modify: `src/tui/screens/create.rs:166-230` (handle_branch_strategy)
- Modify: `src/tui/screens/add.rs:127-184` (handle_branch_strategy)
- Modify: `tests/tui_test.rs` (new tests)

**Step 1: Write the failing test for selecting a recent branch**

Add to `tests/tui_test.rs`:

```rust
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

    // Set up CreateWorkspace at PickBranchStrategy with recent_branches populated
    app.handle_key(key(KeyCode::Char('c')));
    if let Screen::CreateWorkspace(ref mut st) = app.screen {
        st.selected_repos = vec![repo_path];
        st.ws_name = tui_input::Input::default().with_value("ws-branch".to_string());
        st.stage = space::tui::screens::create::CreateStage::PickBranchStrategy;
        // Simulate populated recent_branches
        st.recent_branches = vec![
            space::core::git::BranchInfo {
                name: "feature-pick-me".to_string(),
                is_remote: false,
                is_current: false,
                last_commit_time: 1000,
            },
        ];
        st.branch_strategy_idx = 3; // First recent branch
    }

    // Press Enter on the recent branch
    app.handle_key(key(KeyCode::Enter));

    // Should create workspace with ExistingBranch("feature-pick-me")
    assert!(
        matches!(app.screen, Screen::Dashboard),
        "expected Dashboard after selecting recent branch, got {:?}",
        std::mem::discriminant(&app.screen)
    );
    assert!(env.workspaces_dir.join("ws-branch").join("branch-select-repo").exists());
}
```

**Step 2: Write the failing test for navigation bounds**

Add to `tests/tui_test.rs`:

```rust
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
        // max_idx = 3 + 2 = 5 ("Show more...")
        assert_eq!(st.branch_strategy_idx, 5, "should clamp at max_idx (3 + 2 branches)");
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
```

**Step 3: Run tests to verify they fail**

Run: `cargo test create_select_recent_branch create_branch_strategy_navigation`
Expected: FAIL — max index still clamped at 3, Enter on idx 3 opens picker instead of selecting branch

**Step 4: Update CreateState::handle_branch_strategy**

Replace the body of `handle_branch_strategy` in `src/tui/screens/create.rs` (lines 166-230):

```rust
fn handle_branch_strategy(
    &mut self,
    key: ratatui::crossterm::event::KeyEvent,
    ctx: &crate::tui::actions::ScreenContext,
) -> crate::tui::actions::ScreenAction {
    use crate::tui::actions::{ScreenAction, WorktreeParams};
    use ratatui::crossterm::event::KeyCode;

    let n = self.recent_branches.len();
    let max_idx = 3 + n; // last item is "Show more..." (or "Pick a branch" if n==0)

    match key.code {
        KeyCode::Esc => {
            self.error = None;
            self.stage = CreateStage::NameWorkspace;
            ScreenAction::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            self.error = None;
            if self.branch_strategy_idx > 0 {
                self.branch_strategy_idx -= 1;
            }
            ScreenAction::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            self.error = None;
            if self.branch_strategy_idx < max_idx {
                self.branch_strategy_idx += 1;
            }
            ScreenAction::Continue
        }
        KeyCode::Enter => {
            if self.branch_strategy_idx == max_idx {
                // "Show more..." / "Pick a branch..." — open fuzzy picker
                let repo_path = self.selected_repos.first().cloned();
                if let Some(repo_path) = repo_path {
                    let repo_name = repo_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match crate::tui::app::build_branch_picker(&repo_path, &repo_name) {
                        Some(picker) => {
                            self.picked_branch = None;
                            self.error = None;
                            self.branch_picker = Some(picker);
                            self.stage = CreateStage::PickBranch;
                        }
                        None => {
                            self.error =
                                Some(format!("Could not list branches for {}", repo_name));
                        }
                    }
                }
                ScreenAction::Continue
            } else if self.branch_strategy_idx >= 3 && n > 0 {
                // Selected a recent branch directly
                let branch_name =
                    self.recent_branches[self.branch_strategy_idx - 3].name.clone();
                self.stage = CreateStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.ws_name.value().to_string(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: crate::core::workspace::BranchStrategy::ExistingBranch(
                        branch_name,
                    ),
                    is_new: true,
                })
            } else {
                // idx 0, 1, or 2 — fixed options
                self.stage = CreateStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.ws_name.value().to_string(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: self.branch_strategy(),
                    is_new: true,
                })
            }
        }
        _ => ScreenAction::Continue,
    }
}
```

**Step 5: Update AddState::handle_branch_strategy**

Apply the identical pattern to `src/tui/screens/add.rs` `handle_branch_strategy` (lines 127-184). The only differences:
- Esc goes to `AddStage::PickRepos` (not NameWorkspace)
- `workspace_name` comes from `self.workspace_name.clone()` (not `self.ws_name.value()`)
- `is_new: false`

```rust
fn handle_branch_strategy(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
    let n = self.recent_branches.len();
    let max_idx = 3 + n;

    match key.code {
        KeyCode::Esc => {
            self.error = None;
            self.stage = AddStage::PickRepos;
            ScreenAction::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            self.error = None;
            if self.branch_strategy_idx > 0 {
                self.branch_strategy_idx -= 1;
            }
            ScreenAction::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            self.error = None;
            if self.branch_strategy_idx < max_idx {
                self.branch_strategy_idx += 1;
            }
            ScreenAction::Continue
        }
        KeyCode::Enter => {
            if self.branch_strategy_idx == max_idx {
                let repo_path = self.selected_repos.first().cloned();
                if let Some(repo_path) = repo_path {
                    let repo_name = repo_path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match crate::tui::app::build_branch_picker(&repo_path, &repo_name) {
                        Some(picker) => {
                            self.picked_branch = None;
                            self.error = None;
                            self.branch_picker = Some(picker);
                            self.stage = AddStage::PickBranch;
                        }
                        None => {
                            self.error =
                                Some(format!("Could not list branches for {}", repo_name));
                        }
                    }
                }
                ScreenAction::Continue
            } else if self.branch_strategy_idx >= 3 && n > 0 {
                let branch_name =
                    self.recent_branches[self.branch_strategy_idx - 3].name.clone();
                self.stage = AddStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.workspace_name.clone(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: BranchStrategy::ExistingBranch(branch_name),
                    is_new: false,
                })
            } else {
                self.stage = AddStage::Creating;
                ScreenAction::ExecuteWorktreeFlow(WorktreeParams {
                    workspace_name: self.workspace_name.clone(),
                    workspace_dir: ctx.config.workspaces.dir.clone(),
                    repos: self.selected_repos.clone(),
                    branch_strategy: self.branch_strategy(),
                    is_new: false,
                })
            }
        }
        _ => ScreenAction::Continue,
    }
}
```

**Step 6: Run the new tests**

Run: `cargo test create_select_recent_branch create_branch_strategy_navigation`
Expected: PASS

**Step 7: Run full test suite**

Run: `cargo test`
Expected: all tests pass (including existing `create_strategy_new_branch_creates` which uses idx 0)

**Step 8: Commit**

```
git add src/tui/screens/create.rs src/tui/screens/add.rs tests/tui_test.rs
git commit -m "feat: update branch strategy navigation and Enter handling for recent branches"
```

---

### Task 5: Update render_branch_strategy_picker for recent branches

**Files:**
- Modify: `src/tui/ui.rs:238-250` (render_create_overlay call site)
- Modify: `src/tui/ui.rs:302-369` (render_branch_strategy_picker)
- Modify: `src/tui/ui.rs:425-430` (render_add_overlay call site)

**Step 1: Update the function signature**

Change `render_branch_strategy_picker` in `src/tui/ui.rs` (line 302) to accept recent branches:

```rust
fn render_branch_strategy_picker(
    frame: &mut Frame,
    workspace_name: &str,
    strategy_idx: usize,
    error: Option<&str>,
    recent_branches: &[crate::core::git::BranchInfo],
)
```

**Step 2: Update both call sites**

In `render_create_overlay` (line 245), add `&state.recent_branches`:

```rust
CreateStage::PickBranchStrategy => render_branch_strategy_picker(
    frame,
    state.ws_name.value(),
    state.branch_strategy_idx,
    state.error.as_deref(),
    &state.recent_branches,
),
```

In `render_add_overlay` (line 425), add `&state.recent_branches`:

```rust
AddStage::PickBranchStrategy => render_branch_strategy_picker(
    frame,
    &state.workspace_name,
    state.branch_strategy_idx,
    state.error.as_deref(),
    &state.recent_branches,
),
```

**Step 3: Rewrite the function body**

Replace the body of `render_branch_strategy_picker` with:

```rust
{
    use ratatui::widgets::Clear;
    let has_error = error.is_some();
    let n = recent_branches.len();
    // 3 fixed options + branch section + borders + padding
    let branch_rows = if n > 0 { 1 + n as u16 + 1 } else { 1 }; // header + branches + "Show more" OR just "Pick a branch..."
    let content_rows = 3 + branch_rows;
    let height: u16 = content_rows + 2 + if has_error { 3 } else { 1 }; // +2 borders, +1 pad or +3 error
    let area = centered_rect_fixed(62, height, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if has_error {
        theme::border_danger()
    } else {
        theme::border_focused()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(" Branch Strategy ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = if has_error {
        Layout::vertical([
            Constraint::Length(content_rows),
            Constraint::Length(1), // separator
            Constraint::Length(2), // error
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(content_rows), Constraint::Min(0)]).split(inner)
    };

    // Build all visual rows (selectable items + non-selectable header)
    let mut items: Vec<ListItem> = Vec::new();

    // Fixed options (selectable indices 0, 1, 2)
    let fixed = [
        format!("New branch '{}'", workspace_name),
        format!("Existing branch '{}' (if present)", workspace_name),
        "Detached HEAD".to_string(),
    ];
    for (i, opt) in fixed.iter().enumerate() {
        if i == strategy_idx {
            items.push(ListItem::new(format!("> {}", opt)).style(theme::selected()));
        } else {
            items.push(ListItem::new(format!("  {}", opt)));
        }
    }

    if n > 0 {
        // "Pick a branch..." header (non-selectable, dimmed)
        items.push(ListItem::new("  Pick a branch...").style(theme::muted()));

        // Recent branches (selectable indices 3..3+n)
        for (i, branch) in recent_branches.iter().enumerate() {
            let sel_idx = 3 + i;
            let time_str = crate::core::git::relative_time(branch.last_commit_time);
            // Right-pad branch name, right-align time within available width
            // Available: 62 - 2 borders - 4 indent = 56 chars
            let max_name = 56_usize.saturating_sub(time_str.len() + 2); // 2 for spacing
            let display_name = if branch.name.len() > max_name {
                format!("{}...", &branch.name[..max_name.saturating_sub(3)])
            } else {
                branch.name.clone()
            };
            let padding = 56_usize.saturating_sub(display_name.len() + time_str.len());
            let line = format!(
                "{}{}{}",
                display_name,
                " ".repeat(padding),
                time_str
            );
            if sel_idx == strategy_idx {
                items.push(
                    ListItem::new(format!("  > {}", line)).style(theme::selected()),
                );
            } else {
                items.push(ListItem::new(format!("    {}", line)));
            }
        }

        // "Show more..." (selectable index 3+n)
        let show_more_idx = 3 + n;
        if show_more_idx == strategy_idx {
            items.push(
                ListItem::new("  > Show more...").style(theme::selected()),
            );
        } else {
            items.push(ListItem::new("    Show more..."));
        }
    } else {
        // No recent branches — show "Pick a branch..." as selectable (idx 3)
        if 3 == strategy_idx {
            items.push(
                ListItem::new("> Pick a branch...").style(theme::selected()),
            );
        } else {
            items.push(ListItem::new("  Pick a branch..."));
        }
    }

    frame.render_widget(List::new(items), sections[0]);

    if let Some(err) = error {
        frame.render_widget(
            Paragraph::new(format!("\u{26a0}  {}", err))
                .style(theme::error())
                .wrap(Wrap { trim: false }),
            sections[2],
        );
    }
}
```

**Step 4: Run full test suite**

Run: `cargo test`
Expected: all tests pass

**Step 5: Run clippy + fmt**

Run: `cargo clippy -- -D warnings && cargo fmt --check`

**Step 6: Commit**

```
git add src/tui/ui.rs
git commit -m "feat: render recent branches inline in Branch Strategy dialog"
```

---

### Task 6: Full verification and cleanup

**Files:**
- Possibly: any file with unused imports or dead code after changes

**Step 1: Run full verification**

Run: `cargo test && cargo clippy -- -D warnings && cargo fmt --check`
Expected: all pass with zero warnings

**Step 2: Fix any clippy warnings or format issues**

Address anything found.

**Step 3: Manual smoke test**

Run `cargo run` and:
1. Create a workspace — verify recent branches appear in the dialog with relative times
2. Navigate up/down through the full list — verify no skips or panics
3. Select a recent branch — verify correct worktree created
4. Select "Show more..." — verify fuzzy picker opens
5. Select a fixed option (New branch) — verify it still works
6. Test with a repo that has no branches (or list_branches failure) — verify graceful fallback

**Step 4: Final commit if any cleanup was needed**

```
git add -A
git commit -m "chore: cleanup after inline recent branches feature"
```

---

## Task Dependency Graph

```
Task 1 (BranchInfo + list_branches)
  └→ Task 2 (relative_time helper)
       └→ Task 3 (state fields + populate)
            └→ Task 4 (navigation + Enter handling)
                 └→ Task 5 (rendering)
                      └→ Task 6 (verification + cleanup)
```

## Verification Commands

After every task:
```bash
cargo test
```

Before every commit:
```bash
cargo clippy -- -D warnings && cargo fmt --check
```
