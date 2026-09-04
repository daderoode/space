# space

A terminal workspace manager for multi-repo git worktrees. A user groups repositories into named spaces, each checked out on a branch, and moves between them from one dashboard.

## Language

**Space**:
A named group of repositories, each present as a git worktree, that a user works on together.
_Avoid_: workspace (the code type and older docs use it; new user-facing text says space)

**Repo list**:
The set of repositories found by scanning the configured roots; the pool a user picks from when adding repos to a space.
_Avoid_: repo cache, available repos

**Rescan**:
Re-walking the configured roots to rebuild the repo list.
_Avoid_: refresh, reload

**Sync**:
Bringing a selected repo's local view of its remote up to date before a branch is chosen: fetch, then fast-forward only the local branches that are strictly behind. Branches that are ahead, diverged, or checked out are left as they are. The fetch is an **unattended run**, so it never waits indefinitely for a person, and it runs on a worker, so the report keeps painting and Esc leaves at once while it does.
_Avoid_: pull, update, refresh

**Sync report**:
The screen shown before the branch picker that lists the sync outcome of every selected repo and whether the run as a whole worked.
_Avoid_: progress log, syncing dialog

**Sync outcome**:
The per-repo result inside a sync report: whether the fetch worked, which branches were fast-forwarded, and which were skipped and why.
_Avoid_: sync result, repo result

**Unattended run**:
A git subprocess the app starts with nobody attending it: its own session with no controlling terminal, stdin closed, and a wall-clock limit after which its process group is stopped. With no terminal and no askpass helper configured, a prompt for a credential, passphrase or host key is refused rather than waited on; where the user has an askpass helper, git and ssh call that instead and the run waits on its dialog until the limit. Either way the limit is what guarantees the end. Both the sync and the fetch before a worktree is created run this way, in the TUI and over MCP. The limit bounds the wait; it says nothing about whether the app stays responsive during it, and the two call sites differ there: sync runs on a worker, so its report keeps painting and Esc leaves at once, while the fetch before a worktree is created runs on the UI thread, so nothing repaints and no key is read until it ends.
_Avoid_: background job, detached fetch, headless git

**Space filter**:
Fuzzy selection of a space from the dashboard that changes the selected space and stays in the dashboard.
_Avoid_: search, go

**Go**:
Jumping into a space's directory in the shell, which leaves the dashboard.
_Avoid_: navigate, switch

**Repo search**:
Fuzzy search across the repo list, landing on the space that contains the chosen repo, if any does.
_Avoid_: filter

**Help overlay**:
The keybinding reference layered over whatever screen is showing, listing every group in the binding registry. It never replaces the screen beneath it.
_Avoid_: help screen, help page, cheatsheet

**Binding registry**:
The single list of key groups that both the help overlay and the key bar are rendered from, so documented keys and real keys cannot drift apart.
_Avoid_: keymap, help text, key table

**Key bar**:
The always-visible bottom row listing the keys available on the focused pane.
_Avoid_: status bar, keybindings bar, hint bar

**Status message**:
The transient one-line notice shown above the key bar and cleared after five seconds.
_Avoid_: status bar, toast, flash
