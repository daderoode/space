# Design: Safe Rebase Flow (Tier 3, Item 7)

**Date:** 2026-07-27
**Status:** Approved

## Problem

Rebasing a worktree branch is risky without visibility into the current state
(dirty files, detached HEAD, divergence, conflicts). The Git Operations Menu
(item 6) ships with a `rebase` entry that is a disabled placeholder. This item
turns it into a real, guarded sub-flow that never leaves a worktree
half-rebased.

## Solution

Enable the `r` menu entry. Firing it runs a synchronous pre-flight, then walks
the user through target selection and an explicit confirmation before executing
the rebase on the existing git-ops background worker. On conflict the rebase is
auto-aborted and the branch restored, exactly as `pull_repo` handles a
conflicting merge.

## Scope Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Entry | `r` in the git-ops menu (was disabled) | The menu already reserves the slot; no new keybinding. |
| Pre-flight blockers | Detached HEAD, dirty working tree (tracked modified/staged/conflicted) | These are the states `git rebase` itself refuses. Catch them early with a clear message instead of a raw git error. Untracked files do not block (git allows them). |
| Fetch | Best-effort inside `rebase_repo`, non-fatal | Honors "fetch latest" so `origin/*` targets are current before replay, but an offline rebase onto a local target still works. Matches the app-wide single-`origin` convention. |
| Target selection | Reuse `build_branch_picker` (local + remote branches) | Same picker the switch-branch flow uses; fuzzy-filterable. |
| Confirmation | Explicit `[y/N]`, `y` required, Enter/Esc/`n` decline | Consistent with `ConfirmPush`. Rebase rewrites local history, so default-No is the safe default even though it is reflog-recoverable. |
| Ahead/behind preview | Computed synchronously vs the picked target at confirm time | Informational. It may be one fetch stale (the rebase re-fetches before replaying); acceptable for a preview panel. |
| Conflict handling | Auto-abort (`git rebase --abort`), restore branch, stay open | Never leave the worktree mid-rebase from inside `space`. Interactive conflict resolution is out of scope. |
| Execution | The existing git-ops worker + `Running` stage + auto-close | Reuses the proven fetch/pull/push async path verbatim. |

## Architecture

No new screen. The rebase sub-flow adds three stages to `GitOpsStage` plus a new
`GitOp::Rebase { onto }` variant that flows through the same
`ExecuteGitOp` → `run_gitop_worker` → `poll_gitop_result` path as fetch/pull/push.

### New `GitOpsStage` variants

- `RebasePreflight`: summary panel. Shows the branch and either the blocking
  reason (Esc only) or a "ready" message (Enter → target picker).
- `RebasePickTarget`: the reused `FuzzyPicker` over branches.
- `RebaseConfirm`: `[y/N]` prompt with the ahead/behind preview.

Execution reuses the existing `Running` stage (`running_op = GitOp::Rebase`).

### New `GitOpsState` fields

- `rebase_block: Option<String>` — `Some(reason)` when pre-flight fails.
- `rebase_picker: Option<FuzzyPicker>`.
- `rebase_onto: Option<String>` — the picked target.
- `rebase_ahead_behind: Option<(usize, usize)>` — confirm-stage preview.

### `GitOp` becomes non-Copy

`GitOp::Rebase { onto: String }` carries an owned target, so `GitOp` drops
`Copy` and keeps `Clone`. Ripple: `label(self)` → `label(&self)`; the ui.rs
header uses `running_op.as_ref().map(|op| op.label())`; `start_network_op` clones
`op` into `running_op`. Fetch/Pull/Push call sites are unchanged.

## Core Functions

### `core/git.rs::ahead_behind_vs(repo_path, target) -> Option<(usize, usize)>`

git2 `graph_ahead_behind(HEAD_oid, target_oid)` after `revparse_single(target)`.
Returns `(ahead, behind)`: `ahead` = commits on HEAD that will be replayed;
`behind` = commits the target has that HEAD does not.

### `core/workspace.rs::rebase_repo(repo_path, onto) -> RebaseResult`

Mirrors `pull_repo`'s shape:

1. Detached-HEAD guard (defensive; pre-flight already blocks it).
2. Best-effort `git fetch --quiet origin` (non-fatal).
3. `git rebase <onto>`.
4. On success: `Rebased` (or `UpToDate` when git reports "up to date").
5. On failure: run `git rebase --abort`. If the abort **succeeds**, a rebase was
   in progress → `Conflicted` (branch restored). If the abort **fails** ("no
   rebase in progress"), it was an immediate failure (bad target) → `Failed`.
   Using the abort result to classify is worktree-safe (no `.git/rebase-merge`
   path probing).

`RebaseOutcome`: `Rebased | UpToDate | Conflicted | Failed | DetachedHead`.
`RebaseResult { outcome, message }` with `success()` true for `Rebased`/`UpToDate`.

## Flow

1. `r` → `start_rebase()`: compute detached/dirty, set `rebase_block`, enter
   `RebasePreflight`.
2. `RebasePreflight` + Enter (when unblocked) → build picker, enter
   `RebasePickTarget`. Esc → menu.
3. `RebasePickTarget` + Enter on a branch → store `rebase_onto`, compute
   `rebase_ahead_behind`, enter `RebaseConfirm`. Esc → `RebasePreflight`.
4. `RebaseConfirm` + `y` → `start_network_op(GitOp::Rebase { onto })` (the
   `Running` stage). Enter/Esc/`n` → back to `RebasePickTarget`.
5. Worker runs `rebase_repo`, streams the summary line(s), ends with
   `Done { success }`. Success auto-closes after ~3s and refreshes the repo
   pane; failure stays open with the abort/error message.

## Files to Modify

| File | Change |
|---|---|
| `src/tui/actions.rs` | `GitOp`: drop `Copy`, add `Rebase { onto }`, `label(&self)` + arm. |
| `src/core/git.rs` | `ahead_behind_vs`. |
| `src/core/workspace.rs` | `RebaseOutcome`, `RebaseResult`, `rebase_repo` + tests. |
| `src/tui/screens/gitops.rs` | Three stages, four fields, enable item 5, `start_rebase`, three key handlers, clone in `start_network_op`. |
| `src/tui/app.rs` | `run_gitop_worker`: `GitOp::Rebase` arm. |
| `src/tui/ui.rs` | Render the three new stages; fix the `label` call for non-Copy `GitOp`. |
| `src/tui/keybindings.rs` | Add `r` = Rebase to the `GIT_OPS` help group. |
| `docs/ROADMAP.md` | Mark Tier 3 item 7 complete. |

## Testing

- `rebase_repo`: linear replay onto an advanced target (`Rebased`); target is an
  ancestor (`UpToDate`); conflicting rebase aborts and restores the branch
  (`Conflicted`, clean worktree, HEAD unchanged); unknown target (`Failed`);
  detached HEAD reports without acting.
- `ahead_behind_vs`: known counts relative to a target ref.
- TUI: `r` enters `RebasePreflight`; a dirty/fake repo shows the block message
  and Enter does not proceed; the menu still lists `rebase` (replacing the old
  "always disabled" test).

## Out of Scope

- Interactive/`--interactive` rebase, squash, reword, autosquash.
- Force-push after rebase (surfaced by the existing push flow; the user pushes
  separately and handles the rejection).
- Multi-repo / workspace-wide rebase (Tier 4).
- Conflict resolution UI (auto-abort only).
- Non-`origin` remotes (app-wide single-`origin` convention; see `docs/REVIEW.md`
  and issue #24 SR-3).
