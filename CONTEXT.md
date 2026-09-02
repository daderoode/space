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
Bringing a selected repo's local view of its remote up to date before a branch is chosen: fetch, then fast-forward only the local branches that are strictly behind. Branches that are ahead, diverged, or checked out are left as they are. Sync never waits indefinitely for a person: a repo whose remote would ask for a credential, passphrase or host-key answer fails instead of pausing.
_Avoid_: pull, update, refresh

**Sync report**:
The screen shown before the branch picker that lists the sync outcome of every selected repo and whether the run as a whole worked.
_Avoid_: progress log, syncing dialog

**Sync outcome**:
The per-repo result inside a sync report: whether the fetch worked, which branches were fast-forwarded, and which were skipped and why.
_Avoid_: sync result, repo result

**Space filter**:
Fuzzy selection of a space from the dashboard that changes the selected space and stays in the dashboard.
_Avoid_: search, go

**Go**:
Jumping into a space's directory in the shell, which leaves the dashboard.
_Avoid_: navigate, switch

**Repo search**:
Fuzzy search across the repo list, landing on the space that contains the chosen repo, if any does.
_Avoid_: filter
