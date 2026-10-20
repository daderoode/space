# Changelog

## [Unreleased]

## [0.15.0] - 2026-10-20

### Highlights
- `space rm` no longer deletes uncommitted work. If any repo in the space has changes, removal stops and tells you what is there; pass `--force --discard-changes` (or `discard_changes: true` over MCP, or answer a second `y` in the TUI) to remove it anyway. (#75, #80)

### Breaking changes
- Removing a space refuses uncommitted work instead of destroying it. `space rm <name> --force`, MCP `remove_workspace` and the TUI delete read every repo's status first; if any repo has modified, staged, untracked or conflicted files (ignored files do not count), commits on a detached HEAD that no branch holds, or a rebase, merge, cherry-pick, revert or bisect in progress, or its status cannot be read (including a worktree whose source repository is gone), nothing is removed and the error names each repo with its counts. `space rm --force` exits 1; `remove_workspace` fails with `internal_error`. Before this, every removal passed `git worktree remove --force` and that work was lost without a word. (#75, #80)
  - CLI: add `--discard-changes` (with `--force`) to remove the space anyway. `--force` alone still only skips the confirmation.
  - MCP: `remove_workspace` takes an optional `discard_changes` (default `false`). Clients that relied on it removing a dirty workspace must now pass `discard_changes: true`.

### New / improved
- The TUI delete dialog shows what each repo would lose (`alpha  (2 modified, 1 untracked)`) and, when any repo has uncommitted work, asks a second, default-No question before discarding it; if the repos changed in between, it shows the new state and asks again. A clean space still deletes on one `y`. (#75, #80)

### Fixes
- Rebase passes `--` before its target, so a target starting with `-` (for example `--exec=...`) is read as a branch name and can never run as a git option. Such targets were already refused; this is a second defence. (#79)
- Sync passes `--` before the branch it fast-forwards, so a local branch named like an option (for example `-M`) can no longer make git rename the checked-out branch. (#82, #83)

### Internal
- ADR 0002 corrected on how crossterm 0.28 reaches the build (#77)
- Removed 18 tests that pinned nothing another test does not (#78)

**Full Changelog**: https://github.com/daderoode/space/compare/v0.14.0...v0.15.0

## [0.14.0] - 2026-10-15

### Highlights
- Pushing a new branch can no longer overwrite `main` on origin. New branches track nothing until first published; spaces that still track their base get a prompt (default No). (#65)
- Removing a space no longer throws away work you can't get back. `space rm` / MCP `remove_workspace` keep anything git did not cleanly remove, exit non-zero and explain why. (#44, #61, #66)
- Pull no longer aborts a merge you had in progress; it only ever aborts a merge it started. (#54)
- Creating a space no longer freezes the screen, and Esc cancels. Fetches can't prompt for credentials or hang; a sync report shows each repo's fetch result. (#26, #28, #31)
- Branches from any remote work properly: `upstream/feat` becomes a local tracking branch, and pull, sync, status and switch-branch follow the branch's real upstream. (#59, #63, #64)
- MCP `create_workspace` and `add_repos` are safe to retry: repos already in the space are skipped and listed. (#38)

### Breaking changes
- MCP: `add_repos` with `strategy: "existing"` must include `branch`. (#53)
- CLI/MCP: `space rm --force` and `remove_workspace` fail (exit 1 / `internal_error`) and keep the space when git refuses a removal or a worktree copy is protected. (#44, #61, #66)
- CLI: `space create` / `space add` repo arguments are exact, case-sensitive directory names; unknown or ambiguous names exit 1. (#56)
- Space names that are empty, contain `/`, `\` or control/invisible characters, or start with `-` or `.` are refused. (#36)

### New / improved
- `Ctrl-R` rescans repo roots in the create and add pickers. (#26)
- `/` on the workspaces pane opens a fuzzy space filter. (#26)
- `PgUp`/`PgDn`/`Home`/`End` page the dashboard lists; `?` help works inside overlays and pickers. (#29)
- `space create a b` / `space add ws a b` pre-select those repos in the picker. (#56)
- Create skips repos already in the space and shows them as `(already created)`. (#34)
- The "already checked out elsewhere" prompt works again on git 2.42+. (#32)
- The pre-create fetch is skipped when no remote ref is read, and after a slow sync fetch. (#33, #37)
- Reftable and `worktree.useRelativePaths` repos are recognised. (#48)
- The "New branch" label always shows the name Enter will create. (#42, #45, #58)
- Long text in the config editor, picker query and commit box scrolls with a visible cursor. (#49, #51)
- A `/` in a branch picker search is part of the branch name. (#67)

### Fixes
- A tag named like a branch no longer hijacks create or switch-branch. (#57, #63)
- Space names like `..` can no longer make `rm` or `status` touch directories outside the workspaces folder; dash-leading branch names can no longer be read as git options. (#36)
- A worktree whose checkout was interrupted is refused instead of reported complete. (#48)
- Very wide terminals no longer crash the TUI or squash dialogs. (#46, #47)
- Fixed occasional hangs when several git processes started at once. (#41)
- Git output can no longer corrupt the MCP JSON-RPC stream. (#44)
- A file whose type changed (a regular file replaced by a symlink, or the reverse) no longer reads as clean in `space status`, `space list`, the dashboard and the rebase check, so you can see that work before removing a space. (#74)

### Other behaviour changes
- Esc on the workspaces pane no longer quits; `j`/`k` type in pickers; delete confirmation defaults to No. (#27)
- Create pauses on a sync report; press Enter to continue. (#26)
- Git errors are always shown in English. (#52)

### Internal
- Faster builds (binary reuses the library's modules) (#68)
- Test isolation and flake fixes (#35, #39, #43, #50, #55, #60, #62)
- Em dash guard (#69)
- Docs corrections (#30, #40)
- Dropped the unused direct crossterm 0.28 dependency (#70)
- Tests pinning that sync leaves a diverged branch alone (#71)
- Clippy runs on every target in CI

**Full Changelog**: https://github.com/daderoode/space/compare/v0.13.0...v0.14.0

## Earlier releases

Earlier releases: see [GitHub Releases](https://github.com/daderoode/space/releases).
