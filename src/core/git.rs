use anyhow::{Context, Result};
use git2::{Repository, StatusOptions};
use std::path::Path;

#[derive(Debug, Default)]
pub struct RepoStatus {
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
}

/// Return the default branch name (main/master/etc.) by checking HEAD.
pub fn detect_base_branch(repo_path: &Path) -> String {
    let repo = match Repository::open(repo_path) {
        Ok(r) => r,
        Err(_) => return "main".to_string(),
    };
    if let Ok(head) = repo.head() {
        if let Some(name) = head.shorthand() {
            return name.to_string();
        }
    }
    "main".to_string()
}

/// Count modified, staged, and untracked files using git2.
pub fn repo_status(repo_path: &Path) -> Result<RepoStatus> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("opening repo at {}", repo_path.display()))?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let statuses = repo.statuses(Some(&mut opts))?;
    let mut result = RepoStatus::default();

    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED,
        ) {
            result.staged += 1;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED | git2::Status::WT_DELETED | git2::Status::WT_RENAMED,
        ) {
            result.modified += 1;
        }
        if s.contains(git2::Status::WT_NEW) {
            result.untracked += 1;
        }
    }
    Ok(result)
}

/// List local + remote branches. Remote HEAD refs (`origin/HEAD`) are excluded.
#[allow(dead_code)]
pub fn list_branches(repo_path: &Path) -> Result<Vec<BranchInfo>> {
    let repo = Repository::open(repo_path)?;
    let mut branches = Vec::new();

    let head_name = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(String::from));

    for branch_result in repo.branches(None)? {
        let (branch, branch_type) = branch_result?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.ends_with("/HEAD") {
            continue;
        }
        let is_remote = branch_type == git2::BranchType::Remote;
        let is_current = head_name.as_deref() == Some(&name);
        branches.push(BranchInfo {
            name,
            is_remote,
            is_current,
        });
    }

    branches.sort_by(|a, b| a.is_remote.cmp(&b.is_remote).then(a.name.cmp(&b.name)));
    Ok(branches)
}

/// Return the current checked-out branch name (or short hash for detached HEAD).
pub fn current_branch(repo_path: &Path) -> Result<String> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head()?;
    if head.is_branch() {
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    } else {
        let oid = head.target().unwrap_or(git2::Oid::zero());
        Ok(format!("({})", &oid.to_string()[..8]))
    }
}

/// Return (ahead, behind) relative to the upstream tracking branch.
/// Returns (0, 0) if there is no upstream or the repo has no remote.
pub fn ahead_behind(repo_path: &Path) -> Result<(usize, usize)> {
    let repo = Repository::open(repo_path)?;
    let head = repo.head()?;
    let local_oid = match head.target() {
        Some(o) => o,
        None => return Ok((0, 0)),
    };

    let branch_name = match head.shorthand() {
        Some(n) => n.to_string(),
        None => return Ok((0, 0)),
    };

    let upstream_ref = format!("refs/remotes/origin/{}", branch_name);
    let upstream_oid = match repo.refname_to_id(&upstream_ref) {
        Ok(o) => o,
        Err(_) => return Ok((0, 0)),
    };

    let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid)?;
    Ok((ahead, behind))
}
