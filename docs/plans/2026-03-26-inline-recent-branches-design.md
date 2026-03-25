# Inline Recent Branches in Branch Strategy Dialog -- Design

## Problem

When creating or adding repos to a workspace, the Branch Strategy dialog offers 4 options: New branch, Existing branch, Detached HEAD, and "Pick a branch...". To select a specific existing branch, you must choose "Pick a branch..." which opens a fuzzy picker -- an extra navigation step for the common case where you just want a recent branch.

## Solution

Show the top 5 most recently committed-to branches inline in the Branch Strategy dialog, ordered by last commit date with relative timestamps. A "Show more..." option at the bottom opens the full fuzzy picker for less common branches.

## UI Layout

```
+-- Branch Strategy ------------------------------------------------+
|  New branch 'DEV-5782-omari-address-update'                       |
|  Existing branch 'DEV-5782-...' (if present)                      |
|  Detached HEAD                                                    |
|  Pick a branch...                                                 |
|  > feature/auth                                    2 hours ago    |
|    fix/login-bug                                   3 days ago     |
|    origin/dev                                      1 week ago     |
|    Show more...                                                   |
+-------------------------------------------------------------------+
```

- "Pick a branch..." is a non-selectable visual header (dimmed style, never gets `>` prefix).
- Branch items are indented, selected one gets `> ` prefix.
- Relative time is right-aligned.
- Remote branches display with their `origin/` prefix (already in `name` field).
- "Show more..." opens the existing fuzzy picker.
- 5 branches shown, local + remote, sorted by last commit date descending.

## Navigation

Flat vertical navigation (Up/Down/j/k) across all selectable items. The "Pick a branch..." header is skipped:

| Index | Item | Action on Enter |
|-------|------|-----------------|
| 0 | New branch '<name>' | `BranchStrategy::NewBranch(name)` |
| 1 | Existing branch '<name>' | `BranchStrategy::ExistingBranch(name)` |
| 2 | Detached HEAD | `BranchStrategy::DetachedHead` |
| -- | *"Pick a branch..." (header)* | *skipped in navigation* |
| 3 | recent_branches[0] | `BranchStrategy::ExistingBranch(branch.name)` |
| 4 | recent_branches[1] | same |
| ... | ... | ... |
| 3+N-1 | recent_branches[N-1] | same |
| 3+N | Show more... | Opens fuzzy picker (current "Pick a branch" behavior) |

- Down from idx 2 jumps to idx 3 (skips header).
- Up from idx 3 jumps to idx 2 (skips header).
- Max index = `3 + recent_branches.len()`.
- If `recent_branches` is empty, max is 3 and idx 3 = "Show more..." (functionally identical to current "Pick a branch...").

## Data Layer Changes

### `BranchInfo` struct (`core/git.rs`)

Add `last_commit_time: i64` field. Populated in `list_branches()` by resolving each branch ref to a commit via `branch.get().peel_to_commit()` and reading `commit.time().seconds()`. Defaults to `0` if peel fails.

### `relative_time(unix_ts: i64) -> String` helper (`core/git.rs`)

Pure function, no external dependencies. Compares against current system time. Returns: "just now", "X minutes ago", "X hours ago", "X days ago", "X weeks ago", "X months ago", "X years ago".

## State Changes

Both `CreateState` and `AddState` gain:

```rust
recent_branches: Vec<BranchInfo>
```

Populated when entering `PickBranchStrategy` stage:
- `CreateState::handle_name_workspace()` -- after setting stage, fetch from `self.selected_repos.first()`
- `AddState::handle_pick_repos()` -- same trigger point

Fetch: call `list_branches()`, sort by `last_commit_time` descending, take first 5. If fetch fails, `recent_branches` stays empty (graceful degradation to 3 fixed options + "Show more").

## Rendering Changes

`render_branch_strategy_picker()` in `ui.rs` gains a `recent_branches: &[BranchInfo]` parameter. Dynamic height based on branch count. Both callers (`render_create_overlay`, `render_add_overlay`) pass `&state.recent_branches`.

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| Repo has 0 branches | `recent_branches` empty. Dialog shows 3 fixed options + "Show more..." only. |
| Repo has <= 5 branches | Show all inline. "Show more..." still appears (picker offers fuzzy search). |
| `list_branches()` fails | `recent_branches` empty. Same as 0 branches. No error. |
| Long branch name | Truncated by dialog width. Leave room for relative time. |
| `last_commit_time` is 0 | `relative_time(0)` returns a fallback string. |

## Files to Modify

- `src/core/git.rs` -- extend `BranchInfo`, update `list_branches()`, add `relative_time()`
- `src/tui/screens/create.rs` -- add field, populate, extend navigation + strategy mapping
- `src/tui/screens/add.rs` -- same
- `src/tui/ui.rs` -- extend `render_branch_strategy_picker()` signature + body, update call sites

## Not in Scope

- De-duplicating `CreateState`/`AddState` branch strategy fields
- Showing branches from all selected repos (uses first repo only, matching current behavior)
- Configurable N (hardcoded to 5)
