use anyhow::{Context, Result};
use git2::{Repository, StatusOptions};
use std::cmp::Reverse;
use std::path::Path;

#[derive(Debug, Default, serde::Serialize)]
pub struct RepoStatus {
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffTarget {
    Head,
    Base,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub status: FileStatus,
    pub staged: bool,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    HunkHeader,
    #[allow(dead_code)] // matched in render_diff_overlay, not yet constructed
    FileHeader,
    Binary,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    #[allow(dead_code)] // used in diff overlay title (Step D)
    pub path: String,
    #[allow(dead_code)] // used in diff overlay for renamed files (Step D)
    pub old_path: Option<String>,
    #[allow(dead_code)] // used in diff overlay rendering
    pub is_binary: bool,
    pub lines: Vec<DiffLine>,
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
            branches.sort_by_key(|b| Reverse(b.last_commit_time));
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
/// Returns "unknown" for timestamps <= 0 (failed peel, unset field).
pub fn relative_time(unix_ts: i64) -> String {
    if unix_ts <= 0 {
        return "unknown".to_string();
    }
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

/// Return per-file diff entries for a repo.
///
/// `Head` mode returns uncommitted changes: staged entries (tree→index) and
/// unstaged entries (index→workdir), each with the correct `staged` flag.
///
/// `Base` mode returns total divergence from the base branch. The base branch
/// is resolved by probing `refs/heads/main`, `refs/heads/master`,
/// `refs/remotes/origin/main`, and `refs/remotes/origin/master` in order.
/// Returns an error if none of those refs exist. All entries have `staged: false`
/// in Base mode (the staged/unstaged distinction is not meaningful for
/// committed divergence).
pub fn file_diff(repo_path: &Path, target: &DiffTarget) -> Result<Vec<FileEntry>> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("opening repo at {}", repo_path.display()))?;

    match target {
        DiffTarget::Head => file_diff_vs_head(&repo),
        DiffTarget::Base => file_diff_vs_base(&repo, repo_path),
    }
}

fn file_diff_vs_head(repo: &Repository) -> Result<Vec<FileEntry>> {
    let mut entries: Vec<FileEntry> = Vec::new();

    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());

    // 1. Staged: tree -> index
    let staged_diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;
    entries.extend(collect_entries(&staged_diff, true)?);

    // 2. Unstaged: index -> workdir (include untracked)
    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let unstaged_diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
    entries.extend(collect_entries(&unstaged_diff, false)?);

    Ok(entries)
}

fn file_diff_vs_base(repo: &Repository, repo_path: &Path) -> Result<Vec<FileEntry>> {
    let base_tree = resolve_base_tree(repo, repo_path)?;

    let mut opts = git2::DiffOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let diff = repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?;
    collect_entries(&diff, false)
}

/// Resolve the base branch tree by probing main/master/origin variants.
fn resolve_base_tree<'repo>(
    repo: &'repo Repository,
    repo_path: &Path,
) -> Result<git2::Tree<'repo>> {
    let base_oid = repo
        .refname_to_id("refs/heads/main")
        .or_else(|_| repo.refname_to_id("refs/heads/master"))
        .or_else(|_| repo.refname_to_id("refs/remotes/origin/main"))
        .or_else(|_| repo.refname_to_id("refs/remotes/origin/master"))
        .or_else(|_| {
            repo.find_reference("refs/remotes/origin/HEAD")
                .ok()
                .and_then(|r| r.resolve().ok())
                .and_then(|r| r.target())
                .ok_or_else(|| git2::Error::from_str("origin/HEAD not found"))
        })
        .with_context(|| {
            format!(
                "could not find base branch (tried main/master/origin/main/origin/master/origin/HEAD) in {}",
                repo_path.display()
            )
        })?;

    let base_commit = repo.find_commit(base_oid)?;
    Ok(base_commit.tree()?)
}

fn collect_entries(diff: &git2::Diff, staged: bool) -> Result<Vec<FileEntry>> {
    // First pass: collect file paths and statuses from deltas
    let file_stats: Vec<(String, FileStatus)> = diff
        .deltas()
        .filter_map(|delta| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(|p| p.to_str())
                .map(String::from)?;
            let status = delta_to_file_status(delta.status())?;
            Some((path, status))
        })
        .collect();

    // Second pass: count +/- lines per file via foreach line callback.
    // TODO: O(n²) path lookup — replace with HashMap<&str, usize> if diffs
    // with 500+ files cause noticeable latency in the TUI.
    let mut line_counts: Vec<(usize, usize)> = vec![(0, 0); file_stats.len()];
    {
        let file_stats_ref = &file_stats;
        let counts_ref = std::cell::RefCell::new(&mut line_counts);
        diff.foreach(
            &mut |_, _| true,
            None,
            None,
            Some(&mut |delta, _, line| {
                let path = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())
                    .and_then(|p| p.to_str());
                if let Some(path) = path {
                    if let Some(pos) = file_stats_ref.iter().position(|(p, _)| p == path) {
                        match line.origin() {
                            '+' => counts_ref.borrow_mut()[pos].0 += 1,
                            '-' => counts_ref.borrow_mut()[pos].1 += 1,
                            _ => {}
                        }
                    }
                }
                true
            }),
        )?;
    }

    Ok(file_stats
        .into_iter()
        .zip(line_counts)
        .map(|((path, status), (insertions, deletions))| FileEntry {
            path,
            status,
            staged,
            insertions,
            deletions,
        })
        .collect())
}

fn delta_to_file_status(delta: git2::Delta) -> Option<FileStatus> {
    match delta {
        git2::Delta::Modified => Some(FileStatus::Modified),
        git2::Delta::Added => Some(FileStatus::Added),
        git2::Delta::Deleted => Some(FileStatus::Deleted),
        git2::Delta::Renamed => Some(FileStatus::Renamed),
        git2::Delta::Copied => Some(FileStatus::Copied),
        git2::Delta::Untracked => Some(FileStatus::Untracked),
        _ => None,
    }
}

/// Return the full line-level diff for a single file.
///
/// `staged` controls which comparison is used:
/// - `true`:  HEAD tree -> index  (staged changes)
/// - `false`: index -> workdir    (unstaged changes, including untracked)
///
/// When `target` is `Base`, the diff is computed against the base branch tree
/// and the `staged` flag is ignored.
pub fn file_content_diff(
    repo_path: &Path,
    target: &DiffTarget,
    file_path: &str,
    staged: bool,
) -> Result<FileDiff> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("opening repo at {}", repo_path.display()))?;

    let diff = match target {
        DiffTarget::Head if staged => {
            let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            repo.diff_tree_to_index(head_tree.as_ref(), None, None)?
        }
        DiffTarget::Head => {
            let mut opts = git2::DiffOptions::new();
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            repo.diff_index_to_workdir(None, Some(&mut opts))?
        }
        DiffTarget::Base => {
            let base_tree = resolve_base_tree(&repo, repo_path)?;
            let mut opts = git2::DiffOptions::new();
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .show_untracked_content(true);
            repo.diff_tree_to_workdir_with_index(Some(&base_tree), Some(&mut opts))?
        }
    };

    // Find the delta index matching our file_path
    let delta_idx = diff
        .deltas()
        .enumerate()
        .find(|(_, delta)| {
            let new_match = delta
                .new_file()
                .path()
                .and_then(|p| p.to_str())
                .map(|p| p == file_path)
                .unwrap_or(false);
            let old_match = delta
                .old_file()
                .path()
                .and_then(|p| p.to_str())
                .map(|p| p == file_path)
                .unwrap_or(false);
            new_match || old_match
        })
        .map(|(idx, _)| idx)
        .with_context(|| format!("file '{}' not found in diff", file_path))?;

    let delta = diff.get_delta(delta_idx).unwrap();
    let old_path = delta
        .old_file()
        .path()
        .and_then(|p| p.to_str())
        .map(String::from)
        .filter(|op| op != file_path);

    let result = match git2::Patch::from_diff(&diff, delta_idx)? {
        None => {
            // Binary file (no patch available)
            let size = delta.new_file().size();
            FileDiff {
                path: file_path.to_string(),
                old_path,
                is_binary: true,
                lines: vec![DiffLine {
                    kind: DiffLineKind::Binary,
                    content: format!("Binary file ({} bytes)", size),
                }],
            }
        }
        Some(patch) => {
            // Check if the patch itself considers this binary (the delta flags may
            // only be set after the patch is created for untracked files).
            let is_binary = patch.delta().flags().contains(git2::DiffFlags::BINARY);
            if is_binary {
                let size = patch.delta().new_file().size();
                FileDiff {
                    path: file_path.to_string(),
                    old_path,
                    is_binary: true,
                    lines: vec![DiffLine {
                        kind: DiffLineKind::Binary,
                        content: format!("Binary file ({} bytes)", size),
                    }],
                }
            } else {
                let mut lines = Vec::new();
                for hunk_idx in 0..patch.num_hunks() {
                    let (hunk, _) = patch.hunk(hunk_idx)?;
                    let header = std::str::from_utf8(hunk.header()).unwrap_or("<binary hunk>");
                    lines.push(DiffLine {
                        kind: DiffLineKind::HunkHeader,
                        content: header.to_string(),
                    });
                    for line_idx in 0..patch.num_lines_in_hunk(hunk_idx)? {
                        let line = patch.line_in_hunk(hunk_idx, line_idx)?;
                        let kind = match line.origin() {
                            '+' => DiffLineKind::Addition,
                            '-' => DiffLineKind::Deletion,
                            ' ' => DiffLineKind::Context,
                            '=' | '>' => DiffLineKind::HunkHeader,
                            _ => DiffLineKind::Context,
                        };
                        let content =
                            std::str::from_utf8(line.content()).unwrap_or("<binary line>");
                        lines.push(DiffLine {
                            kind,
                            content: content.to_string(),
                        });
                    }
                }
                FileDiff {
                    path: file_path.to_string(),
                    old_path,
                    is_binary: false,
                    lines,
                }
            }
        }
    };
    Ok(result)
}

/// Stage a single file (add to index). Handles both new/modified files and deletions.
pub fn stage_file(repo_path: &Path, file_path: &str) -> Result<()> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("opening repo at {}", repo_path.display()))?;
    let mut index = repo.index()?;

    if repo_path.join(file_path).exists() {
        index.add_path(Path::new(file_path))?;
    } else {
        index.remove_path(Path::new(file_path))?;
    }
    index.write()?;
    Ok(())
}

/// Unstage a single file (reset index entry to HEAD). Handles unborn HEAD (no commits).
pub fn unstage_file(repo_path: &Path, file_path: &str) -> Result<()> {
    let repo = Repository::open(repo_path)
        .with_context(|| format!("opening repo at {}", repo_path.display()))?;

    match repo.head() {
        Ok(head_ref) => {
            let head_commit = head_ref.peel_to_commit()?;
            repo.reset_default(Some(head_commit.as_object()), [Path::new(file_path)])?;
        }
        Err(_) => {
            // Unborn HEAD: no commits yet, just remove from index
            let mut index = repo.index()?;
            index.remove_path(Path::new(file_path))?;
            index.write()?;
        }
    }
    Ok(())
}

/// Stage all currently unstaged files. Returns the count of files staged.
pub fn stage_all_unstaged(repo_path: &Path) -> Result<usize> {
    let entries = file_diff(repo_path, &DiffTarget::Head)?;
    let unstaged: Vec<_> = entries.iter().filter(|e| !e.staged).collect();
    let count = unstaged.len();
    for entry in &unstaged {
        stage_file(repo_path, &entry.path)?;
    }
    Ok(count)
}

/// Unstage all currently staged files. Returns the count of files unstaged.
pub fn unstage_all_staged(repo_path: &Path) -> Result<usize> {
    let entries = file_diff(repo_path, &DiffTarget::Head)?;
    let staged: Vec<_> = entries.iter().filter(|e| e.staged).collect();
    let count = staged.len();
    for entry in &staged {
        unstage_file(repo_path, &entry.path)?;
    }
    Ok(count)
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
        // Epoch 0 (failed peel) should return "unknown", not "55 years ago"
        assert_eq!(relative_time(0), "unknown");
    }

    #[test]
    fn relative_time_negative_timestamp() {
        // Negative timestamps (future commits / clock skew) also return "unknown"
        assert_eq!(relative_time(-1), "unknown");
    }
}
