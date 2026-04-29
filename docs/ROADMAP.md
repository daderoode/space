# space Roadmap

Prioritised backlog for maturing space into a workspace-aware git manager.
Ordered so each tier builds on the previous one.

---

## Tier 1 -- Foundation ✓ Complete

### 1. Test coverage (golden sample) ✓

126 tests across 9 test files (cli, config, git, mcp, repo, tui, workspace),
all green. Covers CLI commands, TUI create/add flows, MCP tools, cache TTL
enforcement, branch strategy navigation, and recent branches. No gaps.

---

### 2. Split app.rs into per-screen key handlers ✓

Screen handlers extracted into their state structs. Each screen module owns its
`handle_key()` method. `app.rs` dispatches to the active screen and processes
returned `ScreenAction` values. Borrow-checker gymnastics eliminated.

---

## Keybinding Help Overlay ✓ Complete

Standalone UX feature — not gated on any git manager work, low effort.

### 3. Help overlay ✓

Users can now press `?` from the dashboard to open a full-screen help overlay
listing keybindings grouped by context. The overlay closes with `Esc`, `q`, or
`?`, and the status bar consumes the same shared keybinding registry so hints
stay aligned.

**Files:** `src/tui/keybindings.rs`, `src/tui/screens/help.rs`,
`src/tui/app.rs`, `src/tui/ui.rs`, `tests/tui_test.rs`

---

## Tier 1.6 -- Create-flow polish

UX improvements from user feedback. Small, self-contained changes to the
workspace-creation flow. Can be picked off independently.

### 3a. Reorder workspace-creation steps (#7)

**Problem:** The current creation order is: select repos, enter workspace name,
pick branch strategy. Users expect to name the workspace first, then select
repos — the name provides context for the subsequent choices.

**Approach:**

- Reorder `CreateStage` variants: `EnterName` → `PickRepos` → `PickBranchStrategy`
  → `Creating`.
- Workspace name is available earlier, so it can be used as the default
  branch name in `PickBranchStrategy`.
- `AddStage` is unaffected (workspace already exists).

**Files:** `src/tui/screens/create.rs`, `src/tui/ui.rs`
**Done when:** `c` opens a name-first flow and existing tests pass.

---

### 3b. Custom new-branch name (#6)

**Problem:** "New branch" always uses the workspace name as the branch name.
Users want to specify a different branch name (e.g. `feature/DEV-1234`).

**Approach:**

- After selecting "New branch" in `PickBranchStrategy`, show an editable
  text field pre-filled with the workspace name.
- `BranchStrategy::NewBranch(String)` already carries the name — wire the
  input value through instead of defaulting to the workspace name.
- Same field applies in both Create and Add flows.

**Files:** `src/tui/screens/create.rs`, `src/tui/screens/add.rs`,
`src/tui/ui.rs`
**Done when:** Users can edit the branch name before worktree creation.

---

### 3c. Repo metadata and tree view in picker (#5)

**Problem:** The repo picker shows repos as `<dir-name> (<parent-dir-name>)`.
No branch, URL, or hierarchical grouping. Hard to distinguish repos with
similar names under different organisations.

**Approach:**

- Enrich `PickerItem` with optional metadata fields (current branch, remote
  URL) populated at picker construction time via `git2`.
- Render metadata as a dimmed suffix or second column in the picker.
- Optionally group repos by parent directory in a collapsible tree view.
  Tree view is stretch — the metadata suffix alone is high value.

**Files:** `src/tui/widgets/fuzzy_picker.rs`, `src/tui/screens/create.rs`,
`src/tui/screens/add.rs`, `src/tui/ui.rs`
**Done when:** Repos show branch and remote URL in the picker. Tree grouping
is a bonus.

---

## Tier 2 -- Git Visibility

Turn `space` from a workspace navigator into a workspace-aware git viewer.
Users can see exactly what changed, at every level of detail, without leaving
the TUI.

### 4. Fold/unfold repos with file-level diffs (Phase 1) ✓

**Problem:** The repo pane shows aggregate status counts (e.g. `2m 1s`) but no
file-level detail. Users must leave `space` to see what actually changed.

**Approach:**

- `Enter` or `→` on a repo row expands it to show changed files as child rows.
- Each file row shows: status letter (M/A/D/R/?), file path, `+N -M` line
  counts, and a `[staged]`/`[unstaged]` indicator.
- `T` (Shift+T) toggles diff target between HEAD (uncommitted changes) and
  base branch (total divergence from main/master).
- Navigation: `←`/`Esc` collapses expanded repos or refocuses the workspaces
  pane. `↑`/`↓` moves through the flattened repo+file row list.
- File diffs computed via `git2` crate: `diff_tree_to_index()` for staged,
  `diff_index_to_workdir()` for unstaged, `diff_tree_to_tree()` for base mode.
- Diffs fetched lazily on expand, cached per-repo, invalidated on workspace
  switch or refresh.

**Files:** `src/core/git.rs`, `src/tui/app.rs`, `src/tui/ui.rs`
**Done when:** Repos expand/collapse with file-level diffs, toggle between HEAD
and base mode works, navigation through flattened rows is smooth.

---

### 5. File-level diff viewer with staging (Phase 2)

**Problem:** After seeing which files changed, users want to view the actual diff
content and act on it — stage or unstage individual files — without leaving the
TUI.

**Approach — Diff viewer:**

- `Enter` on a file row opens a scrollable diff overlay showing the full
  unified diff for that file (hunks with context lines).
- Uses `git2::Patch::from_diff()` or `diff.print()` to generate diff content.
- Syntax-aware coloring: green for additions, red for deletions, dimmed for
  context.
- `↑`/`↓`/`j`/`k` scrolls within the diff. `Esc` returns to the repo list.

**Approach — Stage/unstage:**

- `s` on a file row in the expanded repo view toggles staging for that file.
  Staged files become unstaged, unstaged files become staged.
- Uses `git2::Index::add_path()` to stage and `git2::Repository::reset_default()`
  (or equivalent) to unstage.
- `[staged]`/`[unstaged]` indicator updates immediately. Diff cache is
  invalidated for that repo.
- `S` (Shift+S) on a repo row stages/unstages all files in that repo.

**Files:** `src/core/git.rs` (new diff content + stage/unstage functions),
`src/tui/app.rs`, `src/tui/ui.rs`, `src/tui/screens/diff.rs` (new)
**Done when:** Users can view full file diffs in a scrollable overlay and
stage/unstage individual files or all files in a repo from the expanded view.

---

## Tier 3 -- Git Operations

Perform routine git operations without leaving the TUI. Each operation is a
self-contained sub-flow within a git operations overlay.

### 6. Git operations menu

**Problem:** Users must leave `space` to perform routine git operations on
individual repos within a workspace.

**Approach:**

- `b` on a repo (folded or unfolded) opens a git operations overlay.
- Available actions: `f` fetch, `p` pull, `P` push, `c` commit, `r` rebase,
  `l` log (recent commits).
- **Commit flow:** Opens a text input for the commit message. Shows a summary
  of staged files above the input. Commits on Enter, aborts on Esc. Only
  available when there are staged changes.
- Each other action is a self-contained sub-flow within the overlay.
- Operations shell out to git (matching the existing pattern for write ops).
- Results shown in a progress/output panel within the overlay.

**Files:** `src/core/git.rs` (new git operation functions), `src/tui/app.rs`,
`src/tui/ui.rs`, `src/tui/screens/gitops.rs` (new)
**Done when:** Users can fetch, pull, push, commit, and view logs for individual
repos without leaving the TUI.

---

### 7. Safe rebase flow

**Problem:** Rebasing a worktree branch is risky without visibility into the
current state (dirty files, divergence, conflicts).

**Approach:**

- `r` from the git operations menu triggers a rebase flow.
- Pre-flight checks: dirty working tree, detached HEAD, fetch latest. Abort
  with clear message if unsafe.
- Summary panel shows: current branch, ahead/behind, warnings.
- Branch picker (reuses existing `build_branch_picker()`) for selecting the
  rebase target.
- Confirmation panel, then execute via `git rebase <target>`.
- On conflict: auto-abort (`git rebase --abort`) and instruct user to rebase
  manually.

**Files:** `src/core/git.rs`, `src/tui/screens/gitops.rs`
**Done when:** Users can safely rebase a single repo's branch with pre-flight
checks and clean abort on conflict.

---

## Tier 4 -- Workflow

Multi-repo productivity features — templates, automation, and sync.

### 8. Repo groups / templates

**Problem:** Users repeatedly create workspaces with the same repo combinations.
Every time they must re-select the same repos.

**Approach:**

- Add a `[groups]` table to `config.toml`:
  ```toml
  [groups]
  payments = ["payment-gateway", "transaction-service", "ledger"]
  frontend = ["web-app", "component-lib", "design-tokens"]
  ```
- CLI: `space create --group payments` pre-selects those repos, skipping the picker.
- TUI: New stage before PickRepos offering "Pick group or manual select."
- MCP: `create_workspace` gains an optional `group` parameter.
- Groups reference repo names (matched against cache, same fuzzy logic as MCP's
  `resolve_repos`).

**Files:** `src/core/config.rs`, `src/cli/mod.rs`, `src/tui/screens/create.rs`,
`src/tui/app.rs`, `src/mcp/mod.rs`
**Done when:** Groups are configurable, selectable in all 3 interfaces, and tested.

---

### 9. Post-create hooks

**Problem:** After creating worktrees, users manually run setup commands
(`npm install`, `make setup`, copy `.env`, etc.) in every repo.

**Approach:**

- Add a `[hooks]` section to config:
  ```toml
  [hooks]
  post_create = "make setup"
  ```
- After each worktree is successfully created, run the hook command in that
  worktree's directory.
- TUI shows hook output in the Creating stage's progress log.
- CLI prints hook output to stdout.
- MCP includes hook results in the response.
- Hooks are optional. If unset or empty, skip silently.
- Hooks run with the worktree directory as CWD and inherit the user's environment.

**Files:** `src/core/config.rs`, `src/core/workspace.rs`, `src/tui/app.rs`,
`src/mcp/mod.rs`
**Done when:** Hooks fire after worktree creation in all 3 interfaces and failures
are reported without aborting the workspace creation.

---

### 10. space sync

**Problem:** After working in a multi-repo workspace, repos can drift -- different
branches, unpushed commits, unfetched changes.

**Approach:**

- New CLI command: `space sync [name]`
  - Fetch all repos in the workspace.
  - Report branch consistency (all on same branch? any diverged?).
  - Report ahead/behind for each repo vs upstream.
  - Optional `--pull` flag to pull all repos (abort on conflicts, report which).
- TUI: Keybind on dashboard (e.g., `s`) to run sync for selected workspace.
  Show results as a transient overlay or status summary.
- MCP: New `sync_workspace` tool.

**Files:** `src/core/workspace.rs` (new sync function), `src/cli/sync.rs` (new),
`src/tui/app.rs`, `src/mcp/mod.rs`
**Done when:** `space sync` fetches, reports divergence, and optionally pulls across
all repos in a workspace.

---

## Tier 5 -- Polish

Refinements and quality-of-life improvements. Low effort individually, can be
picked off opportunistically between larger features.

### 11. space diff (CLI)

**Problem:** Before creating PRs for a multi-repo feature, there's no way to see the
total change footprint across all repos at once from the command line.

**Approach:**

- New CLI command: `space diff [name]`
  - For each repo: reuse existing `file_diff()` from `core/git.rs`.
  - Output aggregate summary: files changed, insertions, deletions per repo.
  - Optional `--name-only` for file list only.
- MCP: New `workspace_diff` tool returning structured diff stats.
- Note: TUI already shows this via fold/unfold (item 4). This adds the CLI and
  MCP interfaces.

**Files:** `src/cli/diff.rs` (new), `src/mcp/mod.rs`
**Done when:** `space diff` shows per-repo and total change stats for a workspace.

---

### 12. Stale worktree cleanup

**Problem:** Worktrees accumulate as features finish. Branches get merged or deleted
upstream, but the local worktrees remain.

**Approach:**

- New function in `core/workspace.rs`: detect worktrees whose branch has been
  merged into the base branch or deleted on the remote.
- TUI dashboard: Show a visual indicator (dimmed, strikethrough, or icon) for stale
  worktrees. Keybind to clean up all stale worktrees with confirmation.
- CLI: `space cleanup [--dry-run]` lists or removes stale worktrees.
- Detection logic: `git branch --merged <base>` for merged, check if upstream ref
  still exists for deleted.

**Files:** `src/core/workspace.rs`, `src/core/git.rs`, `src/cli/cleanup.rs` (new),
`src/tui/app.rs`, `src/tui/ui.rs`
**Done when:** Stale worktrees are detected, surfaced in the TUI, and removable via
CLI and TUI.

---

### 13. Bash and fish completions

**Problem:** Only zsh has completion support. Bash and fish users get nothing.

**Approach:**

- Use `clap_complete` to generate completions for bash and fish from the existing
  clap `Cli` definition.
- Add `space completions bash` and `space completions fish` subcommands.
- Write shell wrapper equivalents for bash and fish (the cd-target protocol needs
  a wrapper function in each shell).
- Update README and GUIDE with setup instructions for all 3 shells.

**Files:** `src/shell/mod.rs`, `src/shell/completions.rs`, new bash/fish wrapper
scripts, `README.md`, `docs/GUIDE.md`
**Done when:** All 3 shells have working completions and cd-wrapper functions.
