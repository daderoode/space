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
                git_worktree_add(
                    &["worktree", "add", "-b", branch_name, &wt, &base_branch],
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
