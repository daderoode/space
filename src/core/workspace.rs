use crate::core::git::{self, RepoStatus};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
    pub repos: Vec<WorkspaceRepo>,
}

#[derive(Debug)]
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

/// Return a workspace with populated repo details (branch, status, ahead/behind).
pub fn workspace_detail(ws_dir: &Path, name: &str) -> Result<Workspace> {
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
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
        let branch = git::current_branch(&repo_path).unwrap_or_else(|_| "?".to_string());
        let status = git::repo_status(&repo_path).unwrap_or_default();
        let (ahead, behind) = git::ahead_behind(&repo_path).unwrap_or((0, 0));
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

    // Prefer stderr; fall back to stdout; fall back to exit code.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .map(|l| l.trim_start_matches("fatal: ").trim())
        .find(|l| !l.is_empty())
        .unwrap_or("git worktree add failed");

    anyhow::bail!("{}", msg)
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
                    &["worktree", "add", "--track", "-b", branch_name, &wt, &remote_ref],
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
