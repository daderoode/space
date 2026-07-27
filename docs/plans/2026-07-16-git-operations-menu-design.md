# Design: Git Operations Menu (Tier 3, Item 6)

**Date:** 2026-07-16
**Status:** Approved

## Problem

Users must leave `space` to perform routine git operations on individual repos
within a workspace: fetching, pulling, pushing, committing staged work, and
viewing recent history. `space` already shows file-level diffs and supports
staging (Tier 2), but the moment the user wants to act on that state (commit it,
push it, pull upstream), they drop back to a terminal.

## Solution

A per-repo git operations overlay, opened with `G` (Shift+g) on the selected
repo in the dashboard repo pane. The overlay presents a menu of actions: fetch, pull, push,
commit, and log. Rebase appears in the menu but is disabled (a placeholder for
Tier 3 item 7, the dedicated safe-rebase flow).

Network operations (fetch, pull, push) run on a background thread with live
output, reusing the async worker pattern established by the repo-sync feature
(#21). Local operations (commit, log) resolve synchronously.

## Scope Decisions (settled during brainstorming)

| Decision | Choice | Rationale |
|---|---|---|
| Target | Single selected repo | Matches roadmap and every existing per-repo op (`s`, switch-branch). Workspace-wide ops have a dedicated home in Tier 4 `space sync`. |
| Entry key | `G` (Shift+g) | `b` is already bound to Switch-branch (#20) and `g` to the Go fuzzy picker, both added after the roadmap was written. `G` is free and mnemonic for Git. Both existing bindings are untouched. |
| Menu input | Letter keys AND arrow/`j`/`k`+Enter | Fast for power users, discoverable for others. |
| Network-op UX | Spinner while running; auto-close ~3s after success; stay open on error | Long output stays readable on failure; success gets out of the way. |
| Pull semantics | Fast-forward when behind; real merge when diverged | Handles the common behind case cleanly and the diverged case functionally. |
| Merge conflict | Auto-abort (`git merge --abort`), restore clean state, report | Never leave the worktree half-merged from inside `space`. Full conflict-resolution UI is out of scope. |
| Commit | Staged-only, single-line message | Respects the existing `s`/`S`/`U` staging model. Disabled when nothing is staged. |
| Push | Auto set-upstream on first push, plain push after, never force. Confirm before publishing a branch with no upstream. | The worktree workflow creates new branches constantly; publishing one is an explicit choice. |
| Log | Scrollable list of ~50 recent commits, read-only | Useful and bounded; no checkout/cherry-pick/detail drilldown. |
| Async model | Dedicated git-ops worker mirroring the sync infra | Reuses a proven, tested pattern; leaves the shipped sync code untouched. |

## Architecture

New screen module `src/tui/screens/gitops.rs` following the `switch_branch.rs`
precedent: a `GitOpsState` struct with `handle_key()` dispatching on an internal
`GitOpsStage` enum. Registered as `Screen::GitOps(GitOpsState)` and rendered by
a new `render_gitops_overlay` in `ui.rs` (dashboard underneath, overlay on top).

### `GitOpsStage`

- `Menu`: the action list.
- `Committing`: single-line commit-message input.
- `Log`: scrollable commit list.
- `Running`: a network op in progress, showing live output lines.
- `ConfirmPush`: confirmation shown only when the branch has no upstream.

### Action dispatch

Actions reach the app through new `ScreenAction` variants:

- `ExecuteGitOp { repo_path, op: GitOp }` where `GitOp` is `Fetch | Pull | Push`.
  Spawns the background worker.
- `CommitRepo { repo_path, message }`. Resolves synchronously in
  `process_action`.

Log data is loaded synchronously when the user opens the Log stage (small,
read-only revwalk), stored on `GitOpsState`.

## Async Worker (network ops)

Direct replica of the sync worker (`app.rs` `run_sync_worker` /
`poll_sync_result` / `ExecuteSyncFlow`, lines 918-1025, 1265-1301):

- `GitOpProgress` enum: `Line(String)` for an output line, `Done { success: bool }`
  as the terminal signal (unlike `SyncProgress::Done`, this carries success so
  the UI can decide auto-close vs stay-open).
- `App` gains `gitop_rx: Option<mpsc::Receiver<GitOpProgress>>` and
  `gitop_cancel: Option<Arc<AtomicBool>>`.
- `run_gitop_worker(repo_path, op, tx, cancel)` shells out via
  `Command::new("git")`, streaming stdout/stderr lines as `Line`, ending with
  `Done { success }`.
- `poll_gitop_result()` is called each frame next to `poll_sync_result`
  (app.rs:1794). It drains the channel into the overlay's output buffer, and on
  `Done` sets the success/failure state that drives the auto-close timer.
- `process_action` handles `ExecuteGitOp` exactly like `ExecuteSyncFlow`:
  cancel any live previous worker, create a bounded `sync_channel(64)`, store rx
  + cancel, spawn the thread.

### Auto-close timer

On `Done { success: true }`, record an `Instant`. `poll_gitop_result` (or the
run loop) checks it each frame; ~3s later it returns to the dashboard and
refreshes (`reset_repo_pane_state` + `load_selected_workspace_detail`). `Esc`
closes sooner. On `Done { success: false }`, no timer: the overlay stays open
until the user presses `Esc`, so the error output remains readable.

## Pull Logic

New `core/workspace.rs::pull_repo(repo_path) -> PullResult`, shelling out:

1. `git fetch origin`.
2. Compute ahead/behind for the current branch vs its upstream (git2
   `graph_ahead_behind`, as `branches_behind_upstream` already does).
3. If behind and not ahead: fast-forward (`git merge --ff-only`).
4. If diverged (ahead and behind): attempt `git merge`. If it exits with
   conflicts, run `git merge --abort` and return a `Diverged`/`Conflict` result.
5. If up to date or only ahead: no-op, report accordingly.

`PullResult` is a struct/enum capturing: fetch outcome, action taken
(fast-forwarded / merged / aborted / up-to-date), and any message. The worker
turns it into `Line` output plus the final `Done { success }`.

## Push Logic

New `core/workspace.rs::push_repo(repo_path, set_upstream: bool) -> Result`,
shelling out:

- If the branch has an upstream: `git push`.
- If not: the menu action first routes to `ConfirmPush`. On confirm, the worker
  runs `git push -u origin <branch>`.
- Never `--force`. A rejected push (remote ahead) surfaces the git error and the
  overlay stays open (user pulls first). Force-with-lease belongs to the rebase
  flow (item 7).

Detecting "no upstream" uses the same ref-resolution the codebase already does
(`refname_to_id("refs/remotes/origin/<branch>")` returning Err, or git2 upstream
lookup).

## Commit Logic

`GitOpsStage::Committing`:

- Single-line input via `tui_input::Input` + `key_to_input_request` (same as
  switch_branch's branch-name entry).
- Staged-file summary listed above the input, from existing `file_diff` filtered
  to `staged == true`.
- Enter with a non-empty message shells out `git commit -m <message>` (matches
  the write-op convention; git handles tree/parent/signature/gpg). Empty message
  blocked with an inline error.
- The menu entry is disabled when there are no staged files, with the hint
  "stage files first with s/S".
- On success: `reset_repo_pane_state()` + `load_selected_workspace_detail()`,
  return to dashboard with a success status (the `SwitchRepoBranch` arm pattern,
  app.rs:1072-1078).

## Log Logic

New `core/git.rs::recent_commits(repo_path, limit) -> Vec<CommitInfo>` using a
git2 revwalk (`push_head()` + time sort). No revwalk exists in the codebase
today, so this is net-new. `CommitInfo` carries short hash, author, commit time,
and subject. Rendering reuses the existing `relative_time` helper for dates.

`GitOpsStage::Log` renders a scrollable list copying the `DiffViewerState`
scroll model (`u16 scroll_offset` + `total_lines`, saturating add/sub), but with
a viewport-aware upper clamp (`len - viewport_height`) to avoid the over-scroll
noted in the diff viewer. `j`/`k`/arrows/PgUp/PgDn/Home/End scroll; `Esc`
returns to the menu.

## Menu

The `Menu` stage renders the repo name and current branch as a title, then the
action list with letter + label:

```
f  fetch
p  pull
P  push
c  commit        (dimmed when nothing staged)
l  log
r  rebase        (dimmed, coming in item 7)
```

Both input modes are active: pressing a letter fires that action directly;
`↑`/`↓`/`j`/`k` move a highlight and `Enter` fires the highlighted action.
`Esc`/`q` closes to the dashboard.

## Phasing

The design ships incrementally. Each phase is planned and implemented on its
own; this document is the shared reference for all of them.

1. **Skeleton.** Screen module, `Screen::GitOps`, `Message::StartGitOps`, the `G`
   entry key (repo pane, Right focus, on a `RepoRow::Repo`), the menu with dual
   input, `render_gitops_overlay`, keybinding-registry entries (new overlay
   group + bump `GROUPS` to 6; add `G` to `REPO_PANE` and the status-bar `RIGHT`
   array, bumping its length). Rebase shown-disabled. Every action routes to a
   handler that sets a "not yet implemented" status, so the wiring is testable
   before the sub-flows land.
2. **Fetch.** Stands up the whole async path: `GitOpProgress`, `gitop_rx`/
   `gitop_cancel`, `run_gitop_worker`, `poll_gitop_result`, `ExecuteGitOp`, plus
   `git fetch`. Includes the `Running` stage render and the auto-close timer.
3. **Pull.** `pull_repo` with fast-forward / merge / auto-abort.
4. **Push.** `push_repo` with set-upstream and the `ConfirmPush` stage.
5. **Commit.** The `Committing` stage and `CommitRepo` action.
6. **Log.** `recent_commits` and the scrollable `Log` stage.

## Files to Modify

| File | Change |
|---|---|
| `src/tui/screens/gitops.rs` | New. `GitOpsState`, `GitOpsStage`, `handle_key`. |
| `src/tui/screens/mod.rs` | Add `pub mod gitops;`. |
| `src/tui/app.rs` | `Screen::GitOps` variant + dispatch; `Message::StartGitOps` variant + handler; `G` key arm (Right pane, repo row); `GitOpProgress`; `gitop_rx`/`gitop_cancel` fields (+ `None` in constructors/test helpers); `run_gitop_worker`; `poll_gitop_result` (+ call in run loop); `ExecuteGitOp`/`CommitRepo` in `process_action`. |
| `src/tui/actions.rs` | `ExecuteGitOp { repo_path, op }` and `CommitRepo { repo_path, message }` variants; `GitOp` enum. |
| `src/tui/ui.rs` | `Screen::GitOps` render arm; `render_gitops_overlay` and per-stage renderers. |
| `src/tui/keybindings.rs` | `g` in `REPO_PANE` and status-bar `RIGHT` (bump length); new GitOps overlay group (bump `GROUPS` to 6). |
| `src/core/git.rs` | `recent_commits` (revwalk) + `CommitInfo`. |
| `src/core/workspace.rs` | `pull_repo` (+ `PullResult`), `push_repo`, and a commit helper (or inline `git commit` in the worker/handler). |

## Testing

Per phase, mirroring the existing suite:

- Worker lifecycle tests like `run_sync_worker_honors_preset_cancel_flag` and
  the cancel/re-entrancy tests (`poll_*` behavior when leaving the stage).
- `pull_repo` against real bare-remote + helper-clone fixtures, like the
  `sync_repo` tests: behind (fast-forwards), diverged (merges or aborts),
  up-to-date (no-op).
- `push_repo`: no-upstream sets upstream; rejected push reports failure.
- `recent_commits` on a real repo with known commits.
- TUI stage-transition tests: `g` opens the menu on a repo row; menu dispatch;
  commit disabled with nothing staged; `Esc` closes.

## Edge Cases

- **Cursor not on a repo row** (file row / section header): `G` does nothing,
  matching how `b`/`StartSwitchBranch` guards on `RepoRow::Repo`.
- **Repo with no remote:** fetch/pull/push report failure cleanly and stay open;
  the worker never panics (sends are `let _ = tx.send(...)`).
- **Detached HEAD:** push and pull report a clear message rather than acting.
- **Nothing staged:** commit menu entry disabled with a hint.
- **Empty repo (unborn HEAD):** log shows an empty list; commit still works via
  shell `git commit`.
- **User closes the overlay mid-network-op:** `poll_gitop_result` sets the
  cancel flag and drops the receiver, exactly as `poll_sync_result` does when
  leaving the Syncing stage.
- **Re-entering a git op while one is live:** cancel the previous worker first
  (the `ExecuteSyncFlow` re-entrancy pattern).
- **Merge conflict on pull:** auto-abort, report, stay open.

## Out of Scope

- Rebase (Tier 3 item 7): shown disabled in the menu only.
- Force push / force-with-lease (belongs with rebase).
- Workspace-wide multi-repo git ops (Tier 4 `space sync`).
- Interactive conflict resolution.
- Multi-line commit messages (subject + body).
- Amend, cherry-pick, stash, reset, tag, or any commit-detail drilldown from the
  log.
- Changing the existing switch-branch (`b`) or staging (`s`/`S`/`U`) flows.
