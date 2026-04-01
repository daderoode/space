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

## Tier 2 -- Git Manager

LazyGit does not handle worktrees well. This tier turns `space` from a workspace
navigator into a workspace-aware git manager, delivered incrementally across 4
phases. See `docs/plans/2026-03-31-git-manager-design.md` for the full Phase 1
design.

### 3. Fold/unfold repos with file-level diffs (Phase 1)

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

### 4. File-level diff viewer (Phase 2)

**Problem:** After seeing which files changed, users want to view the actual diff
content without leaving the TUI.

**Approach:**

- `Enter` on a file row opens a scrollable diff overlay showing the full
  unified diff for that file (hunks with context lines).
- Uses `git2::Patch::from_diff()` or `diff.print()` to generate diff content.
- Syntax-aware coloring: green for additions, red for deletions, dimmed for
  context.
- `↑`/`↓`/`j`/`k` scrolls within the diff. `Esc` returns to the repo list.

**Files:** `src/core/git.rs` (new diff content function), `src/tui/app.rs`,
`src/tui/ui.rs`, `src/tui/screens/diff.rs` (new)
**Done when:** Users can view full file diffs in a scrollable overlay from the
expanded repo view.

---

### 5. Git operations menu (Phase 3)

**Problem:** Users must leave `space` to perform routine git operations on
individual repos within a workspace.

**Approach:**

- `b` on a repo (folded or unfolded) opens a git operations overlay.
- Available actions: `f` fetch, `p` pull, `P` push, `r` rebase, `d` view diff,
  `l` log (recent commits).
- Each action is a self-contained sub-flow within the overlay.
- Operations shell out to git (matching the existing pattern for write ops).
- Results shown in a progress/output panel within the overlay.

**Files:** `src/core/git.rs` (new git operation functions), `src/tui/app.rs`,
`src/tui/ui.rs`, `src/tui/screens/gitops.rs` (new)
**Done when:** Users can fetch, pull, push, and view logs for individual repos
without leaving the TUI.

---

### 6. Safe rebase flow (Phase 4)

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

## Tier 3 -- Core Features

### 7. Repo groups / templates

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

### 8. Post-create hooks

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

### 9. space sync

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

## Tier 4 -- Power Features

### 10. space diff

**Problem:** Before creating PRs for a multi-repo feature, there's no way to see the
total change footprint across all repos at once.

**Approach:**

- New CLI command: `space diff [name]`
  - For each repo: run `git diff --stat` (or use libgit2 diff APIs).
  - Output aggregate summary: files changed, insertions, deletions per repo.
  - Optional `--name-only` for file list only.
- TUI: Keybind on dashboard (e.g., `d`) to show diff summary overlay.
- MCP: New `workspace_diff` tool returning structured diff stats.

**Files:** `src/core/git.rs` (new diff stat function), `src/cli/diff.rs` (new),
`src/tui/app.rs`, `src/tui/ui.rs`, `src/mcp/mod.rs`
**Done when:** `space diff` shows per-repo and total change stats for a workspace.

---

### 11. Stale worktree cleanup

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

### 12. Bash and fish completions

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
