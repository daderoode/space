use anyhow::{Context, Result};
use git2::{Repository, StatusOptions};
use std::path::Path;

#[derive(Debug, Default, serde::Serialize)]
pub struct RepoStatus {
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
}

#[derive(Debug)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
    pub last_commit_time: i64,
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
        let last_commit_time = branch
            .get()
            .peel_to_commit()
            .map(|c| c.time().seconds())
            .unwrap_or(0);
        branches.push(BranchInfo {
            name,
            is_remote,
            is_current,
            last_commit_time,
        });
    }

    branches.sort_by(|a, b| a.is_remote.cmp(&b.is_remote).then(a.name.cmp(&b.name)));
    Ok(branches)
}

/// Return the N most recently committed-to branches for a repo.
pub fn recent_branches(repo_path: &std::path::Path, limit: usize) -> Vec<BranchInfo> {
    list_branches(repo_path)
        .map(|mut branches| {
            branches.sort_by(|a, b| b.last_commit_time.cmp(&a.last_commit_time));
            branches.truncate(limit);
            branches
        })
        .unwrap_or_default()
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

/// Human-readable relative time string from a unix timestamp.
pub fn relative_time(unix_ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    format_delta(now - unix_ts)
}

/// Format a time delta (in seconds) as a human-readable string.
/// Negative deltas (future timestamps / clock skew) are treated as "just now".
fn format_delta(delta: i64) -> String {
    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let m = delta / 60;
        if m == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", m)
        }
    } else if delta < 86400 {
        let h = delta / 3600;
        if h == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", h)
        }
    } else if delta < 604800 {
        let d = delta / 86400;
        if d == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", d)
        }
    } else if delta < 2592000 {
        let w = delta / 604800;
        if w == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", w)
        }
    } else if delta < 31536000 {
        let m = delta / 2592000;
        if m == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", m)
        }
    } else {
        let y = delta / 31536000;
        if y == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_delta_just_now() {
        assert_eq!(format_delta(30), "just now");
    }

    #[test]
    fn format_delta_negative_is_just_now() {
        assert_eq!(format_delta(-100), "just now");
    }

    #[test]
    fn format_delta_minutes() {
        assert_eq!(format_delta(300), "5 minutes ago");
    }

    #[test]
    fn format_delta_singular_minute() {
        assert_eq!(format_delta(90), "1 minute ago");
    }

    #[test]
    fn format_delta_hours() {
        assert_eq!(format_delta(7200), "2 hours ago");
    }

    #[test]
    fn format_delta_days() {
        assert_eq!(format_delta(259200), "3 days ago");
    }

    #[test]
    fn format_delta_weeks() {
        assert_eq!(format_delta(1209600), "2 weeks ago");
    }

    #[test]
    fn format_delta_months() {
        assert_eq!(format_delta(7776000), "3 months ago");
    }

    #[test]
    fn format_delta_years() {
        assert_eq!(format_delta(63072000), "2 years ago");
    }

    #[test]
    fn relative_time_zero_timestamp() {
        // Epoch 0 should produce a "years ago" result
        let result = relative_time(0);
        assert!(result.contains("years ago"), "got: {}", result);
    }
}
