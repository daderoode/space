use crate::core::git::{self, RepoStatus};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, serde::Serialize)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
    pub repos: Vec<WorkspaceRepo>,
}

#[derive(Debug, serde::Serialize)]
pub struct WorkspaceRepo {
    pub name: String,
    /// Absolute path to the worktree on disk. Used by the TUI (v0.2.0).
    #[allow(dead_code)]
    pub path: PathBuf,
    pub branch: String,
    pub status: RepoStatus,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug)]
pub enum BranchStrategy {
    /// Create a new branch with this name off the repo's default branch.
    NewBranch(String),
    /// Checkout an existing branch (local or remote-tracking).
    ExistingBranch(String),
    /// Detached HEAD at the default branch.
    DetachedHead,
}

/// List all workspace directories inside `ws_dir`.
pub fn list_workspaces(ws_dir: &Path) -> Result<Vec<Workspace>> {
    let mut workspaces = Vec::new();
    if !ws_dir.exists() {
        return Ok(workspaces);
    }
    for entry in std::fs::read_dir(ws_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            workspaces.push(Workspace {
                name,
                path,
                repos: vec![],
            });
        }
    }
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workspaces)
}

/// Return lightweight repo stubs for a workspace without opening any git repos.
/// Used to populate the repos pane immediately on navigation while
/// `workspace_detail` loads in the background.
pub fn workspace_repo_skeletons(ws_dir: &Path, name: &str) -> Vec<WorkspaceRepo> {
    let ws_path = ws_dir.join(name);
    let mut repos = Vec::new();
    let Ok(entries) = std::fs::read_dir(&ws_path) else {
        return repos;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let repo_path = entry.path();
        if !repo_path.join(".git").exists() {
            continue;
        }
        let repo_name = entry.file_name().to_string_lossy().to_string();
        repos.push(WorkspaceRepo {
            name: repo_name,
            path: repo_path,
            branch: "...".to_string(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

/// Return a workspace with populated repo details (branch, status, ahead/behind).
pub fn workspace_detail(ws_dir: &Path, name: &str) -> Result<Workspace> {
    let t = std::time::Instant::now();
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
        tracing::warn!(kind = "not_found", "workspace_detail failed");
        anyhow::bail!("workspace '{}' not found", name);
    }
    let mut repos = Vec::new();
    for entry in std::fs::read_dir(&ws_path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let repo_path = entry.path();
        let repo_name = entry.file_name().to_string_lossy().to_string();
        if !repo_path.join(".git").exists() {
            continue;
        }
        let (branch, status, ahead, behind) = match git2::Repository::open(&repo_path) {
            Ok(repo) => {
                let branch =
                    git::current_branch_from_repo(&repo).unwrap_or_else(|_| "?".to_string());
                let status = git::repo_status_from_repo(&repo).unwrap_or_default();
                let (ahead, behind) = git::ahead_behind_from_repo(&repo).unwrap_or((0, 0));
                (branch, status, ahead, behind)
            }
            Err(_) => ("?".to_string(), git::RepoStatus::default(), 0, 0),
        };
        repos.push(WorkspaceRepo {
            name: repo_name,
            path: repo_path,
            branch,
            status,
            ahead,
            behind,
        });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    tracing::info!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        repo_count = repos.len(),
        "workspace_detail completed"
    );
    Ok(Workspace {
        name: name.to_string(),
        path: ws_path,
        repos,
    })
}

/// Create a git worktree for `repo_path` inside `ws_dir/<ws_name>/<repo_name>`.
/// Run a git command, capturing stdout+stderr. On non-zero exit, returns an
/// error that includes the first non-empty line of stderr so the TUI can show
/// the real git message (e.g. "branch already checked out at …").
fn git_worktree_add(args: &[&str], cwd: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| "failed to spawn git")?;

    if out.status.success() {
        return Ok(());
    }

    // Git writes progress ("Preparing worktree...") and errors ("fatal: ...") to
    // stderr. Prefer the fatal line; fall back to any non-empty line.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .find(|l| l.starts_with("fatal:"))
        .map(|l| l.trim_start_matches("fatal:").trim())
        .or_else(|| {
            stderr.lines().map(|l| l.trim()).find(|l| {
                !l.is_empty()
                    && !l.starts_with("Preparing worktree")
                    && !l.starts_with("HEAD is now")
            })
        })
        .unwrap_or("git worktree add failed");

    anyhow::bail!("{}", msg)
}

/// Helper: run a git command inside `cwd`, capturing stderr.
/// On non-zero exit, returns an error with the first meaningful git error line.
#[allow(dead_code)] // used by switch_worktree_branch; bin crate has private mod core
fn run_git_in(cwd: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| "failed to spawn git")?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .find(|l| l.starts_with("error:") || l.starts_with("fatal:"))
        .and_then(|l| l.split_once(':').map(|(_, r)| r.trim()))
        .or_else(|| stderr.lines().map(|l| l.trim()).find(|l| !l.is_empty()))
        .unwrap_or("git switch failed");
    anyhow::bail!("{}", msg)
}

/// Switch an existing worktree to a different branch.
///
/// - `new_branch = true`:  creates the branch from the current HEAD (`git switch -c <branch>`).
///   This works even from detached HEAD.
/// - `new_branch = false`: checks for a local branch first; if absent, looks for
///   `origin/<branch>` and creates a local tracking branch; if neither, passes through
///   to git (which will error with a clear message).
#[allow(dead_code)] // public API; called from integration tests and future callers
pub fn switch_worktree_branch(wt_path: &Path, branch: &str, new_branch: bool) -> Result<()> {
    if new_branch {
        return run_git_in(wt_path, &["switch", "-c", branch]);
    }

    // Normalize: if the caller passes "origin/<name>" (from the full branch picker),
    // strip the prefix so we check/create the local name and avoid "origin/origin/<name>".
    let (local_name, remote_ref) = if let Some(name) = branch.strip_prefix("origin/") {
        (name, branch.to_string())
    } else {
        (branch, format!("origin/{}", branch))
    };

    // Check local branch (refs/heads/ scopes the lookup to branches only, not tags)
    let local_ref = format!("refs/heads/{}", local_name);
    let local_exists = Command::new("git")
        .args(["rev-parse", "--verify", &local_ref])
        .current_dir(wt_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if local_exists {
        return run_git_in(wt_path, &["switch", "--", local_name]);
    }

    // Check remote branch
    let remote_exists = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(wt_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if remote_exists {
        return run_git_in(wt_path, &["switch", "-c", local_name, &remote_ref]);
    }

    // Let git provide the error message
    run_git_in(wt_path, &["switch", "--", local_name])
}

/// Result from syncing a single repo with its remote.
pub struct SyncRepoResult {
    pub fetch_ok: bool,
    pub forwarded: Vec<String>, // names of branches that were fast-forwarded
}

/// Fetch from `origin` and fast-forward all local branches that are strictly
/// behind their `origin/<branch>` ref (0 ahead, N behind); this assumes a single
/// remote named `origin` rather than each branch's configured upstream. Branches
/// with local commits ahead, diverged, or currently checked out are left
/// untouched.
/// If the fetch fails (offline / no remote), returns `fetch_ok: false` and an
/// empty `forwarded` list — the caller should warn and continue.
pub fn sync_repo(repo_path: &Path) -> SyncRepoResult {
    let fetch_ok = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !fetch_ok {
        return SyncRepoResult {
            fetch_ok: false,
            forwarded: vec![],
        };
    }

    let behind = git::branches_behind_upstream(repo_path);
    let mut forwarded = vec![];

    for branch in behind {
        let remote_ref = format!("origin/{}", branch);
        let success = Command::new("git")
            .args(["branch", "-f", &branch, &remote_ref])
            .current_dir(repo_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if success {
            forwarded.push(branch);
        }
        // Silently ignore failures (e.g. branch is currently checked out)
    }

    SyncRepoResult {
        fetch_ok: true,
        forwarded,
    }
}

/// The classification of what `pull_repo` did (or why it did nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
    /// `git fetch` failed (offline / no remote).
    FetchFailed,
    /// No current branch (detached HEAD) — nothing to pull onto.
    DetachedHead,
    /// The current branch has no `origin/<branch>` upstream to pull from.
    NoUpstream,
    /// Local already matches upstream (0 ahead, 0 behind).
    UpToDate,
    /// Local is only ahead of upstream — nothing to pull.
    Ahead,
    /// Local was behind and was fast-forwarded to the upstream.
    FastForwarded,
    /// Local and upstream diverged and were merged cleanly (merge commit).
    Merged,
    /// Local and upstream diverged with conflicts; the merge was aborted and
    /// the worktree restored to its pre-merge state.
    Conflicted,
    /// The pull could not be applied (e.g. a blocked fast-forward); git's
    /// error output is in the message.
    Failed,
}

/// Result of pulling a single repo: the outcome plus a human-readable message.
pub struct PullResult {
    pub outcome: PullOutcome,
    pub message: String,
}

impl PullResult {
    /// Whether the pull left the repo in a good state. Success outcomes let the
    /// worker auto-close the overlay; failures keep it open so the user can read
    /// the message.
    pub fn success(&self) -> bool {
        matches!(
            self.outcome,
            PullOutcome::UpToDate
                | PullOutcome::Ahead
                | PullOutcome::FastForwarded
                | PullOutcome::Merged
        )
    }
}

/// Pull the current branch of `repo_path` from its `origin/<branch>` upstream.
///
/// Fetches first, then classifies the branch state and acts:
/// - behind only  → fast-forward (`FastForwarded`)
/// - diverged     → real merge; on conflict, `git merge --abort` (`Merged`/`Conflicted`)
/// - up to date / only ahead → no-op (`UpToDate`/`Ahead`)
/// - detached HEAD / no upstream / fetch failure → report without acting.
pub fn pull_repo(repo_path: &Path) -> PullResult {
    // Detached HEAD is checked BEFORE the fetch: a detached-HEAD pull must
    // report without acting at all (not even mutating remote-tracking refs).
    let branch = match current_branch_name(repo_path) {
        Some(b) => b,
        None => {
            return PullResult {
                outcome: PullOutcome::DetachedHead,
                message: "Detached HEAD: no branch to pull.".to_string(),
            };
        }
    };

    // `.output()` (not `.status()`): capture stderr both to surface the real
    // failure cause (auth, DNS, missing remote) and to keep git from writing
    // to the inherited stderr, which would scribble over the raw-mode TUI.
    let fetch = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .output();
    let fetch_failed_message = match &fetch {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Some(if stderr.is_empty() {
                "Fetch failed: no remote or offline.".to_string()
            } else {
                format!("Fetch failed: {}", stderr)
            })
        }
        Err(err) => Some(format!("Fetch failed: {}", err)),
    };
    if let Some(message) = fetch_failed_message {
        return PullResult {
            outcome: PullOutcome::FetchFailed,
            message,
        };
    }

    // No `origin/<branch>` upstream to pull from (checked post-fetch so the
    // remote-tracking refs are fresh).
    let remote_exists = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{}", branch),
        ])
        .current_dir(repo_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !remote_exists {
        return PullResult {
            outcome: PullOutcome::NoUpstream,
            message: format!("{} has no upstream (origin/{}) to pull.", branch, branch),
        };
    }

    let (ahead, behind) = git::ahead_behind(repo_path).unwrap_or((0, 0));

    if behind > 0 && ahead == 0 {
        let remote_ref = format!("origin/{}", branch);
        let output = Command::new("git")
            .args(["merge", "--ff-only", &remote_ref])
            .current_dir(repo_path)
            .output();
        match output {
            Ok(o) if o.status.success() => {
                return PullResult {
                    outcome: PullOutcome::FastForwarded,
                    message: format!(
                        "Fast-forwarded {} to {} ({} commit(s)).",
                        branch, remote_ref, behind
                    ),
                };
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let detail = stderr.trim();
                let detail = if detail.is_empty() {
                    "git reported no error output"
                } else {
                    detail
                };
                return PullResult {
                    outcome: PullOutcome::Failed,
                    message: format!(
                        "Fast-forward of {} to {} failed: {}",
                        branch, remote_ref, detail
                    ),
                };
            }
            Err(err) => {
                return PullResult {
                    outcome: PullOutcome::Failed,
                    message: format!(
                        "Fast-forward of {} to {} failed: {}",
                        branch, remote_ref, err
                    ),
                };
            }
        }
    }

    if ahead > 0 && behind > 0 {
        let remote_ref = format!("origin/{}", branch);
        let merged = Command::new("git")
            .args(["merge", "--no-edit", &remote_ref])
            .current_dir(repo_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if merged {
            return PullResult {
                outcome: PullOutcome::Merged,
                message: format!(
                    "Merged {} into {} ({} ahead, {} behind).",
                    remote_ref, branch, ahead, behind
                ),
            };
        }
        // Merge left conflicts: restore the pre-merge worktree so `space` never
        // leaves the repo half-merged.
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo_path)
            .status();
        return PullResult {
            outcome: PullOutcome::Conflicted,
            message: format!(
                "Merge of {} into {} conflicted; aborted and restored a clean worktree.",
                remote_ref, branch
            ),
        };
    }

    if ahead > 0 && behind == 0 {
        return PullResult {
            outcome: PullOutcome::Ahead,
            message: format!(
                "{} is {} commit(s) ahead of upstream; nothing to pull.",
                branch, ahead
            ),
        };
    }

    PullResult {
        outcome: PullOutcome::UpToDate,
        message: format!("{} is already up to date.", branch),
    }
}

/// Result of pushing the current branch: whether git accepted the push, plus a
/// human-readable message. On rejection the message carries git's own output so
/// the non-fast-forward reason surfaces verbatim.
pub struct PushResult {
    pub success: bool,
    pub message: String,
}

/// Push the current branch of `repo_path` to `origin`.
///
/// - `set_upstream == true`  → `git push -u origin <branch>` (first publish of a
///   branch with no upstream; also records the tracking ref).
/// - `set_upstream == false` → `git push` (branch already has an upstream).
///
/// Never forces. A rejected push (remote ahead / non-fast-forward) returns
/// `success == false` with git's rejection text in `message`, so the overlay
/// stays open and the user can pull first.
pub fn push_repo(repo_path: &Path, set_upstream: bool) -> PushResult {
    let branch = match current_branch_name(repo_path) {
        Some(b) => b,
        None => {
            return PushResult {
                success: false,
                message: "Detached HEAD: no branch to push.".to_string(),
            };
        }
    };

    let args: Vec<String> = if set_upstream {
        vec![
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            branch.clone(),
        ]
    } else {
        vec!["push".to_string()]
    };

    let out = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) => {
            // git writes both its success summary and rejection details to stderr.
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let message = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("Pushed {}.", branch)
            };
            PushResult {
                success: o.status.success(),
                message,
            }
        }
        Err(e) => PushResult {
            success: false,
            message: format!("Failed to run git push: {}", e),
        },
    }
}

/// Result of committing staged changes: whether git accepted the commit, plus a
/// human-readable summary (git's own stdout/stderr) so the overlay can surface
/// the outcome verbatim on both success and failure.
pub struct CommitResult {
    pub success: bool,
    pub message: String,
}

/// Commit the currently staged changes in `repo_path` with `message`.
///
/// Shells out to `git commit -m <message>`, letting git build the tree, resolve
/// the parent (or create the initial commit on an unborn HEAD), and apply the
/// user's signature/gpg settings. A commit with nothing staged returns
/// `success == false` with git's "nothing to commit" text in `message`.
pub fn commit_repo(repo_path: &Path, message: &str) -> CommitResult {
    let out = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) => {
            // git writes the commit summary to stdout; refusals ("nothing to
            // commit") also land on stdout, so prefer it and fall back to stderr.
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let message = if !stdout.is_empty() {
                stdout
            } else if !stderr.is_empty() {
                stderr
            } else {
                "Committed.".to_string()
            };
            CommitResult {
                success: o.status.success(),
                message,
            }
        }
        Err(e) => CommitResult {
            success: false,
            message: format!("Failed to run git commit: {}", e),
        },
    }
}

/// The current checked-out branch name, or `None` when HEAD is detached (no
/// symbolic branch ref).
fn current_branch_name(repo_path: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Returns the path to the created worktree.
pub fn create_worktree(
    repo_path: &Path,
    ws_dir: &Path,
    ws_name: &str,
    strategy: &BranchStrategy,
) -> Result<PathBuf> {
    let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
    let wt_path = ws_dir.join(ws_name).join(repo_name.as_ref());

    std::fs::create_dir_all(wt_path.parent().unwrap())?;

    let base_branch = git::detect_base_branch(repo_path);
    let wt = wt_path.to_string_lossy();

    // Auto-fetch — ignore errors for offline use
    let _ = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .status();

    match strategy {
        BranchStrategy::NewBranch(branch_name) => {
            // 1. Local branch exists?
            let local_exists = Command::new("git")
                .args(["rev-parse", "--verify", branch_name])
                .current_dir(repo_path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            // 2. Remote branch exists?
            let remote_ref = format!("origin/{}", branch_name);
            let remote_exists = Command::new("git")
                .args(["rev-parse", "--verify", &remote_ref])
                .current_dir(repo_path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if local_exists {
                git_worktree_add(&["worktree", "add", &wt, branch_name], repo_path)?;
            } else if remote_exists {
                git_worktree_add(
                    &[
                        "worktree",
                        "add",
                        "--track",
                        "-b",
                        branch_name,
                        &wt,
                        &remote_ref,
                    ],
                    repo_path,
                )?;
            } else {
                // Prefer origin/<base> so the new branch starts at the remote tip rather than
                // a potentially stale local ref. Trade-off: if <base> has unpushed local
                // commits they are NOT included in the new worktree. That is intentional —
                // the sync step guarantees origin/<base> is the freshest shared state.
                // Fall back to local only if the remote ref doesn't exist (offline / no remote).
                let origin_base = format!("origin/{}", base_branch);
                let origin_base_exists = Command::new("git")
                    .args(["rev-parse", "--verify", &origin_base])
                    .current_dir(repo_path)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                let start_point: &str = if origin_base_exists {
                    &origin_base
                } else {
                    &base_branch
                };
                git_worktree_add(
                    &["worktree", "add", "-b", branch_name, &wt, start_point],
                    repo_path,
                )?;
            }
        }

        BranchStrategy::ExistingBranch(branch_name) => {
            let local = branch_name.strip_prefix("origin/").unwrap_or(branch_name);
            if branch_name.starts_with("origin/") {
                git_worktree_add(
                    &["worktree", "add", "--track", "-b", local, &wt, branch_name],
                    repo_path,
                )?;
            } else {
                git_worktree_add(&["worktree", "add", &wt, local], repo_path)?;
            }
        }

        BranchStrategy::DetachedHead => {
            git_worktree_add(
                &["worktree", "add", "--detach", &wt, &base_branch],
                repo_path,
            )?;
        }
    }

    Ok(wt_path)
}

/// Remove a workspace: call `git worktree remove` for each repo worktree,
/// then delete the directory.
pub fn remove_workspace(ws_dir: &Path, name: &str, force: bool) -> Result<()> {
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
        anyhow::bail!("workspace '{}' not found", name);
    }

    for entry in std::fs::read_dir(&ws_path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let wt_path = entry.path();
        if !wt_path.join(".git").exists() {
            continue;
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        let wt_str = wt_path.to_string_lossy().to_string();
        args.push(&wt_str);

        if let Some(main_repo) = find_main_repo(&wt_path) {
            Command::new("git")
                .args(&args)
                .current_dir(&main_repo)
                .status()
                .ok();
        }
    }

    std::fs::remove_dir_all(&ws_path)
        .with_context(|| format!("removing workspace directory {}", ws_path.display()))?;
    Ok(())
}

/// Given a worktree path, read its `.git` file to find the main repo root.
fn find_main_repo(wt_path: &Path) -> Option<PathBuf> {
    let git_file = wt_path.join(".git");
    if git_file.is_file() {
        let content = std::fs::read_to_string(&git_file).ok()?;
        let gitdir = content.trim().strip_prefix("gitdir: ")?;
        let path = PathBuf::from(gitdir);
        path.ancestors()
            .find(|p| p.join("config").exists() && p.ends_with(".git"))
            .and_then(|p| p.parent())
            .map(PathBuf::from)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;

    fn git(args: &[&str], dir: &Path) {
        let out = Cmd::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed:\n{}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_setup(dir: &Path) {
        git(&["config", "user.email", "t@local"], dir);
        git(&["config", "user.name", "T"], dir);
        git(&["config", "commit.gpgsign", "false"], dir);
    }

    fn get_sha(dir: &Path, refname: &str) -> String {
        let out = Cmd::new("git")
            .args(["rev-parse", refname])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Returns `(tmp, local_path)` where:
    /// - `local/main` is checked out and 1 behind `origin/main`
    /// - `local/dev` is NOT checked out and 1 behind `origin/dev`
    fn make_behind_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");

        Cmd::new("git")
            .args(["init", "--bare", "-b", "main", "origin.git"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["clone", "origin.git", "local"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&local);

        git(&["commit", "--allow-empty", "-m", "init"], &local);
        git(&["push", "-u", "origin", "main"], &local);

        git(&["checkout", "-b", "dev"], &local);
        git(&["commit", "--allow-empty", "-m", "dev-init"], &local);
        git(&["push", "-u", "origin", "dev"], &local);
        git(&["checkout", "main"], &local);

        let helper = tmp.path().join("helper");
        Cmd::new("git")
            .args(["clone", "origin.git", "helper"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&helper);

        git(&["commit", "--allow-empty", "-m", "main-remote"], &helper);
        git(&["push", "origin", "main"], &helper);

        Cmd::new("git")
            .args(["checkout", "-b", "dev", "origin/dev"])
            .current_dir(&helper)
            .output()
            .unwrap();
        git(&["commit", "--allow-empty", "-m", "dev-remote"], &helper);
        git(&["push", "origin", "dev"], &helper);

        // Do NOT fetch in local — sync_repo's internal fetch handles that
        (tmp, local)
    }

    /// Bare `origin.git` + a `local` clone with a single pushed commit on
    /// `main` (tracked as `origin/main`), containing `base.txt`. `local` is up
    /// to date with `origin/main` on return.
    fn origin_and_local() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");
        Cmd::new("git")
            .args(["init", "--bare", "-b", "main", "origin.git"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["clone", "origin.git", "local"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&local);
        std::fs::write(local.join("base.txt"), "base\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "init"], &local);
        git(&["push", "-u", "origin", "main"], &local);
        (tmp, local)
    }

    /// Clone `origin.git` (already created by `origin_and_local`) into a fresh
    /// `helper` worktree on `main`, run `edit` to stage/commit a change, and
    /// push it to `origin/main` — advancing the remote so `local` falls behind.
    fn advance_origin(tmp: &Path, edit: impl FnOnce(&Path)) {
        let helper = tmp.join("helper");
        Cmd::new("git")
            .args(["clone", "origin.git", "helper"])
            .current_dir(tmp)
            .output()
            .unwrap();
        git_setup(&helper);
        edit(&helper);
        git(&["push", "origin", "main"], &helper);
    }

    #[test]
    fn sync_repo_returns_fetch_failed_without_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(tmp.path());
        git(&["commit", "--allow-empty", "-m", "init"], tmp.path());

        let result = sync_repo(tmp.path());
        assert!(!result.fetch_ok, "fetch_ok must be false when no remote");
        assert!(
            result.forwarded.is_empty(),
            "forwarded must be empty when fetch fails"
        );
    }

    #[test]
    fn sync_repo_fast_forwards_non_checked_out_branch_behind_remote() {
        let (_tmp, local) = make_behind_repo();

        let sha_before = get_sha(&local, "dev");
        let result = sync_repo(&local);

        assert!(result.fetch_ok, "fetch must succeed");
        assert!(
            result.forwarded.contains(&"dev".to_string()),
            "dev must be fast-forwarded: {:?}",
            result.forwarded
        );

        let sha_after = get_sha(&local, "dev");
        let origin_sha = get_sha(&local, "origin/dev");
        assert_ne!(sha_before, sha_after, "dev must have advanced");
        assert_eq!(
            sha_after, origin_sha,
            "dev must equal origin/dev after fast-forward"
        );
    }

    #[test]
    fn sync_repo_does_not_fast_forward_checked_out_branch() {
        let (_tmp, local) = make_behind_repo();

        let result = sync_repo(&local);

        assert!(result.fetch_ok, "fetch must succeed");
        assert!(
            !result.forwarded.contains(&"main".to_string()),
            "main must not be fast-forwarded when checked out: {:?}",
            result.forwarded
        );
    }

    #[test]
    fn pull_repo_fast_forwards_checked_out_branch_behind_remote() {
        // make_behind_repo leaves `local` on `main`, one commit behind
        // origin/main (0 ahead). pull_repo fetches then fast-forwards.
        let (_tmp, local) = make_behind_repo();

        let sha_before = get_sha(&local, "main");
        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::FastForwarded),
            "behind branch must fast-forward, got {:?}: {}",
            result.outcome,
            result.message
        );
        let sha_after = get_sha(&local, "main");
        let origin_sha = get_sha(&local, "origin/main");
        assert_ne!(sha_before, sha_after, "main must have advanced");
        assert_eq!(
            sha_after, origin_sha,
            "main must equal origin/main after fast-forward"
        );
    }

    #[test]
    fn pull_repo_reports_fetch_failed_without_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(tmp.path());
        git(&["commit", "--allow-empty", "-m", "init"], tmp.path());

        let result = pull_repo(tmp.path());
        assert!(
            result.message.contains("origin"),
            "the fetch failure must surface git's actual error (naming the \
             missing remote), got: {}",
            result.message
        );
        assert!(
            matches!(result.outcome, PullOutcome::FetchFailed),
            "no remote must yield FetchFailed, got {:?}: {}",
            result.outcome,
            result.message
        );
    }

    #[test]
    fn pull_repo_reports_up_to_date_when_synced() {
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::UpToDate),
            "synced branch must report UpToDate, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "up-to-date pull must not move main"
        );
    }

    #[test]
    fn pull_repo_reports_ahead_when_local_has_unpushed_commit() {
        let (_tmp, local) = origin_and_local();
        let init_sha = get_sha(&local, "main");

        // Local advances but does not push — 1 ahead, 0 behind.
        git(&["commit", "--allow-empty", "-m", "local-only"], &local);
        let ahead_sha = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Ahead),
            "ahead-only branch must report Ahead, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            ahead_sha,
            get_sha(&local, "main"),
            "ahead-only pull must not move main"
        );
        assert_eq!(
            init_sha,
            get_sha(&local, "origin/main"),
            "ahead-only pull must not push (origin/main unchanged)"
        );
    }

    #[test]
    fn pull_repo_merges_diverged_non_conflicting_changes() {
        let (tmp, local) = origin_and_local();

        // Remote advances main with a new file the local side never touches.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("helper.txt"), "helper\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-side"], helper);
        });

        // Local advances main with a different, non-conflicting file.
        std::fs::write(local.join("local.txt"), "local\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-side"], &local);

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Merged),
            "clean diverge must merge, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            local.join("local.txt").exists() && local.join("helper.txt").exists(),
            "merged worktree must contain both sides' files"
        );
        // A real merge commit has two parents (HEAD^2 resolves).
        let two_parents = Cmd::new("git")
            .args(["rev-parse", "--verify", "HEAD^2"])
            .current_dir(&local)
            .output()
            .unwrap()
            .status
            .success();
        assert!(two_parents, "a merge commit (two parents) must exist");
    }

    #[test]
    fn pull_repo_aborts_conflicting_merge_and_leaves_clean_worktree() {
        let (tmp, local) = origin_and_local();

        // Remote and local change the SAME file to different content.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "helper-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "local-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-edit"], &local);
        let sha_before = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Conflicted),
            "conflicting diverge must report Conflicted, got {:?}: {}",
            result.outcome,
            result.message
        );
        // The merge must have been aborted: no MERGE_HEAD, clean status,
        // no conflict markers, and HEAD back where it started.
        assert!(
            !local.join(".git/MERGE_HEAD").exists(),
            "MERGE_HEAD must be gone after abort"
        );
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after merge --abort"
        );
        let base = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(
            !base.contains("<<<<<<<"),
            "base.txt must not contain conflict markers after abort"
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "aborted merge must leave main at its pre-merge commit"
        );
    }

    #[test]
    fn pull_repo_reports_failure_when_fast_forward_is_blocked() {
        let (tmp, local) = origin_and_local();
        // Remote advances base.txt; local has an UNCOMMITTED edit to the same
        // file, so `git merge --ff-only` refuses (local changes would be
        // overwritten). Reported by Copilot on PR #23: this used to fall
        // through to UpToDate and auto-close the overlay claiming success.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "remote-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "remote-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "dirty-local\n").unwrap();

        let result = pull_repo(&local);

        assert!(
            !result.success(),
            "a blocked fast-forward must not report success, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            !matches!(result.outcome, PullOutcome::UpToDate),
            "a blocked fast-forward must not report UpToDate"
        );
        assert!(
            !result.message.is_empty(),
            "the failure must surface git's error output"
        );
        // The dirty local edit must survive untouched.
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            "dirty-local\n",
            "the blocked pull must not clobber the local uncommitted change"
        );
    }

    #[test]
    fn pull_repo_reports_no_upstream_for_unpublished_branch() {
        let (_tmp, local) = origin_and_local();
        // A local-only branch: fetch succeeds (remote exists) but there is no
        // origin/feature to pull from.
        git(&["checkout", "-b", "feature"], &local);
        let sha_before = get_sha(&local, "feature");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::NoUpstream),
            "an unpublished branch must report NoUpstream (not DetachedHead), got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "NoUpstream is not a success");
        assert_eq!(
            sha_before,
            get_sha(&local, "feature"),
            "a NoUpstream pull must not move the branch"
        );
    }

    #[test]
    fn pull_repo_reports_detached_head_without_acting() {
        let (tmp, local) = origin_and_local();
        git(&["checkout", "--detach"], &local);
        let sha_before = get_sha(&local, "HEAD");
        // The remote advances after we detach; a detached-HEAD pull must
        // report WITHOUT acting (story 37), so not even the fetch may run —
        // local's origin/main remote-tracking ref must stay where it was.
        let origin_ref_before = get_sha(&local, "origin/main");
        advance_origin(tmp.path(), |helper| {
            git(
                &["commit", "--allow-empty", "-m", "remote-moves-on"],
                helper,
            );
        });

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::DetachedHead),
            "detached HEAD must report DetachedHead, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "HEAD"),
            "detached-HEAD pull must not move HEAD"
        );
        assert_eq!(
            origin_ref_before,
            get_sha(&local, "origin/main"),
            "detached-HEAD pull must not act at all — no fetch, so the \
             remote-tracking ref must be unchanged"
        );
    }

    #[test]
    fn push_repo_sets_upstream_on_new_branch() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // A new local branch with no upstream, carrying a distinct commit.
        git(&["checkout", "-b", "feature"], &local);
        git(&["commit", "--allow-empty", "-m", "feature-work"], &local);
        let feature_sha = get_sha(&local, "feature");

        let result = push_repo(&local, true);

        assert!(
            result.success,
            "pushing a new branch with -u must succeed: {}",
            result.message
        );
        // origin/feature now exists in the bare remote at the pushed commit.
        assert_eq!(
            feature_sha,
            get_sha(&bare, "refs/heads/feature"),
            "bare origin must have refs/heads/feature at the pushed commit"
        );
        // The local branch now tracks origin/feature.
        let upstream = Cmd::new("git")
            .args(["rev-parse", "--abbrev-ref", "feature@{upstream}"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&upstream.stdout).trim(),
            "origin/feature",
            "feature must track origin/feature after push -u"
        );
    }

    #[test]
    fn push_repo_plain_push_succeeds_when_ahead_with_upstream() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // Local advances main (already tracked) by one commit, then plain-pushes.
        git(&["commit", "--allow-empty", "-m", "local-ahead"], &local);
        let local_sha = get_sha(&local, "main");

        let result = push_repo(&local, false);

        assert!(
            result.success,
            "plain push of an ahead branch must succeed: {}",
            result.message
        );
        assert_eq!(
            local_sha,
            get_sha(&bare, "refs/heads/main"),
            "bare origin/main must equal local main after push"
        );
    }

    #[test]
    fn push_repo_reports_failure_on_rejected_non_fast_forward() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // Remote advances main via a helper clone (local never sees this commit).
        advance_origin(tmp.path(), |helper| {
            git(&["commit", "--allow-empty", "-m", "remote-ahead"], helper);
        });
        let remote_sha = get_sha(&bare, "refs/heads/main");

        // Local also advances main with its own commit → diverged / non-fast-forward.
        git(&["commit", "--allow-empty", "-m", "local-ahead"], &local);
        let local_sha = get_sha(&local, "main");

        let result = push_repo(&local, false);

        assert!(
            !result.success,
            "a non-fast-forward push must be rejected, message: {}",
            result.message
        );
        assert!(
            !result.message.is_empty(),
            "a rejected push must carry a message"
        );
        assert!(
            result.message.to_lowercase().contains("reject"),
            "message should include git's rejection, got: {}",
            result.message
        );
        // The bare remote's main must NOT have moved to local's commit.
        assert_eq!(
            remote_sha,
            get_sha(&bare, "refs/heads/main"),
            "rejected push must not move origin/main"
        );
        assert_ne!(
            local_sha,
            get_sha(&bare, "refs/heads/main"),
            "origin/main must not equal local's rejected commit"
        );
    }

    #[test]
    fn commit_repo_commits_staged_changes() {
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "HEAD");

        // Stage a new file, then commit it via commit_repo.
        std::fs::write(local.join("new.txt"), "hello\n").unwrap();
        git(&["add", "."], &local);

        let result = commit_repo(&local, "add new file");

        assert!(result.success, "commit must succeed: {}", result.message);
        let sha_after = get_sha(&local, "HEAD");
        assert_ne!(sha_before, sha_after, "HEAD must advance after commit");

        // The new commit's subject is exactly the message.
        let subject = Cmd::new("git")
            .args(["log", "-1", "--pretty=%s"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&subject.stdout).trim(),
            "add new file",
            "the new commit's subject must be the message"
        );

        // The staged change is now committed: clean porcelain.
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after committing the staged change"
        );
    }

    #[test]
    fn commit_repo_creates_initial_commit_on_unborn_head() {
        // Fresh repo, a file staged, no commits yet (unborn HEAD).
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(repo)
            .output()
            .unwrap();
        git_setup(repo);
        std::fs::write(repo.join("first.txt"), "first\n").unwrap();
        git(&["add", "."], repo);

        // No commits yet: rev-parse HEAD fails.
        let before = Cmd::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            !before.status.success(),
            "HEAD must be unborn before the initial commit"
        );

        let result = commit_repo(repo, "initial");

        assert!(
            result.success,
            "initial commit on unborn HEAD must succeed: {}",
            result.message
        );
        let after = Cmd::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            after.status.success(),
            "HEAD must resolve after the initial commit"
        );
    }

    #[test]
    fn commit_repo_fails_when_nothing_staged() {
        // Clean repo with an existing commit and nothing staged: git refuses.
        let (_tmp, local) = origin_and_local();

        let result = commit_repo(&local, "nothing to do");

        assert!(
            !result.success,
            "commit with nothing staged must fail, message: {}",
            result.message
        );
        assert!(
            !result.message.is_empty(),
            "a failed commit must carry a message"
        );
    }
}
