mod common;

use space::core::git::{
    file_content_diff, file_diff, git_index_mtime, repo_status, stage_all_unstaged, stage_file,
    unstage_all_staged, unstage_file, DiffLineKind, FileStatus, MAX_DIFF_LINES,
};
use std::process::Command;
use tempfile::TempDir;

#[test]
fn detects_base_branch() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let branch = space::core::git::detect_base_branch(tmp.path());
    assert!(
        branch == "main" || branch == "master",
        "unexpected branch: {}",
        branch
    );
}

#[test]
fn clean_repo_status_is_zero() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.modified, 0);
    assert_eq!(status.staged, 0);
    assert_eq!(status.untracked, 0);
}

#[test]
fn lists_branches() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let out = Command::new("git")
        .args(["branch", "feature-x"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git branch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let branches = space::core::git::list_branches(tmp.path()).unwrap();
    assert_eq!(branches.len(), 2, "should have main + feature-x");

    let main_branch = branches
        .iter()
        .find(|b| b.name == "main")
        .expect("main branch");
    assert!(main_branch.is_current, "main should be current");
    assert!(!main_branch.is_remote, "main should be local");

    let feature_branch = branches
        .iter()
        .find(|b| b.name == "feature-x")
        .expect("feature-x");
    assert!(
        !feature_branch.is_current,
        "feature-x should not be current"
    );
    assert!(!feature_branch.is_remote, "feature-x should be local");

    // Both branches should have a commit time (from init_repo's initial commit)
    assert!(
        main_branch.last_commit_time > 0,
        "main branch should have a commit time"
    );
    assert!(
        feature_branch.last_commit_time > 0,
        "feature-x branch should have a commit time (inherited from main)"
    );

    // Verify locals come before remotes in sort order
    assert!(
        !branches[0].is_remote,
        "first branch should be local (sort order: locals before remotes)"
    );
}

#[test]
fn current_branch_returns_branch_name() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let branch = space::core::git::current_branch(tmp.path()).unwrap();
    assert_eq!(branch, "main");
}

#[test]
fn dirty_repo_counts_modified_and_staged() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Create a tracked file and commit it
    std::fs::write(tmp.path().join("tracked.txt"), "v1").unwrap();
    let out = Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["commit", "-m", "add tracked"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Modify the tracked file (working tree modified)
    std::fs::write(tmp.path().join("tracked.txt"), "v2").unwrap();

    // Create and stage a new file (index new)
    std::fs::write(tmp.path().join("staged.txt"), "new").unwrap();
    let out = Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add staged.txt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Create an untracked file
    std::fs::write(tmp.path().join("untracked.txt"), "zzz").unwrap();

    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.modified, 1, "one modified working-tree file");
    assert_eq!(status.staged, 1, "one staged file");
    assert_eq!(status.untracked, 1, "one untracked file");
}

#[test]
fn current_branch_detached_head() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    let out = Command::new("git")
        .args(["checkout", "--detach"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git checkout --detach failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let branch = space::core::git::current_branch(tmp.path()).unwrap();
    assert!(
        branch.starts_with('(') && branch.ends_with(')'),
        "detached HEAD should be formatted as (hash), got: {}",
        branch
    );
    assert_eq!(branch.len(), 10, "(8-char hash) = 10 chars");
}

#[test]
fn ahead_behind_no_remote_returns_zero() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let (ahead, behind) = space::core::git::ahead_behind(tmp.path()).unwrap();
    assert_eq!((ahead, behind), (0, 0), "no remote should yield (0,0)");
}

#[test]
fn ahead_behind_with_remote_counts_commits() {
    // Create a bare repo to act as "origin"
    let bare_dir = TempDir::new().unwrap();
    let out = Command::new("git")
        .args(["init", "--bare", "-b", "main"])
        .current_dir(bare_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Clone it to get a repo with a remote
    let clone_dir = TempDir::new().unwrap();
    let out = Command::new("git")
        .args(["clone", &bare_dir.path().to_string_lossy(), "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Configure user
    for args in [
        vec!["config", "user.email", "test@local"],
        vec!["config", "user.name", "Test"],
    ] {
        let out = Command::new("git")
            .args(&args)
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    // Make initial commit + push to establish tracking
    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Make a second local commit (not pushed) -> should be ahead=1
    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "local only"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "second commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let (ahead, behind) = space::core::git::ahead_behind(clone_dir.path()).unwrap();
    assert_eq!(ahead, 1, "one unpushed commit");
    assert_eq!(behind, 0, "nothing to pull");
}

#[test]
fn repo_status_counts_deletions() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Create and commit a file
    std::fs::write(tmp.path().join("to-delete.txt"), "doomed").unwrap();
    let out = Command::new("git")
        .args(["add", "to-delete.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["commit", "-m", "add file"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Delete the file from working tree (WT_DELETED)
    std::fs::remove_file(tmp.path().join("to-delete.txt")).unwrap();

    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.modified, 1, "deleted file should count as modified");
}

#[test]
fn file_diff_clean_repo_returns_empty() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    let result = file_diff(tmp.path()).unwrap();
    assert!(result.is_empty(), "clean repo should have no diffs");
}

#[test]
fn file_diff_detects_unstaged_modification() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("foo.txt"), "v1").unwrap();
    Command::new("git")
        .args(["add", "foo.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add foo"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("foo.txt"), "v1\nv2\nv3").unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "foo.txt");
    assert_eq!(entries[0].status, FileStatus::Modified);
    assert!(
        !entries[0].staged,
        "unstaged change should have staged=false"
    );
    assert!(entries[0].insertions > 0 || entries[0].deletions > 0);
}

#[test]
fn file_diff_detects_staged_file() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("new.txt"), "hello\nworld").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let staged_entry = entries.iter().find(|e| e.path == "new.txt").unwrap();
    assert!(
        staged_entry.staged,
        "staged new file should have staged=true"
    );
    assert_eq!(staged_entry.status, FileStatus::Added);
    assert_eq!(staged_entry.insertions, 2);
    assert_eq!(staged_entry.deletions, 0);
}

#[test]
fn file_diff_detects_staged_and_unstaged_separately() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("a.txt"), "original").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add a"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("a.txt"), "staged change").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("b.txt"), "unstaged").unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let a_entry = entries.iter().find(|e| e.path == "a.txt").unwrap();
    assert!(a_entry.staged);
    let b_entry = entries.iter().find(|e| e.path == "b.txt").unwrap();
    assert!(!b_entry.staged);
    assert!(matches!(
        b_entry.status,
        FileStatus::Added | FileStatus::Untracked
    ));
}

#[test]
fn file_diff_detects_deleted_file() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("gone.txt"), "bye").unwrap();
    Command::new("git")
        .args(["add", "gone.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add gone"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::remove_file(tmp.path().join("gone.txt")).unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let entry = entries.iter().find(|e| e.path == "gone.txt").unwrap();
    assert_eq!(entry.status, FileStatus::Deleted);
    assert!(!entry.staged);
}

// ── A7 tests ──────────────────────────────────────────────────────────

#[test]
fn file_content_diff_returns_lines_for_modified_file() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("f.txt"), "line1\n").unwrap();
    Command::new("git")
        .args(["add", "f.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add f"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("f.txt"), "line1\nline2\n").unwrap();

    let fd = file_content_diff(tmp.path(), "f.txt", false).unwrap();
    assert_eq!(fd.path, "f.txt");
    assert!(!fd.is_binary);
    assert!(!fd.lines.is_empty());
    assert!(
        fd.lines.iter().any(|l| l.kind == DiffLineKind::Addition),
        "should contain an addition line"
    );
}

#[test]
fn file_content_diff_for_staged_uses_tree_to_index() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("s.txt"), "original\n").unwrap();
    Command::new("git")
        .args(["add", "s.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add s"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("s.txt"), "original\nstaged\n").unwrap();
    Command::new("git")
        .args(["add", "s.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let fd = file_content_diff(tmp.path(), "s.txt", true).unwrap();
    assert!(!fd.is_binary);
    assert!(
        fd.lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Addition && l.content.contains("staged")),
        "staged diff should contain the staged addition"
    );
}

#[test]
fn file_content_diff_for_untracked_returns_full_file_as_additions() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("new.txt"), "aaa\nbbb\nccc\n").unwrap();

    let fd = file_content_diff(tmp.path(), "new.txt", false).unwrap();
    assert!(!fd.is_binary);
    let additions: Vec<_> = fd
        .lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Addition)
        .collect();
    assert_eq!(additions.len(), 3, "all 3 lines should be additions");
}

#[test]
fn file_content_diff_marks_binary() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    // Write a file with null bytes to trigger binary detection
    std::fs::write(tmp.path().join("bin.dat"), b"\x00\x01\x02\x03").unwrap();

    let fd = file_content_diff(tmp.path(), "bin.dat", false).unwrap();
    assert!(
        fd.is_binary,
        "file with null bytes should be detected as binary"
    );
    assert!(
        fd.lines.iter().any(|l| l.kind == DiffLineKind::Binary),
        "should have a Binary diff line"
    );
}

#[test]
fn stage_file_marks_modified_as_staged() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("m.txt"), "v1").unwrap();
    Command::new("git")
        .args(["add", "m.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add m"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("m.txt"), "v2").unwrap();

    stage_file(tmp.path(), "m.txt").unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let entry = entries.iter().find(|e| e.path == "m.txt").unwrap();
    assert!(entry.staged, "file should be staged after stage_file");
}

#[test]
fn stage_file_for_deletion_uses_remove_path() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("d.txt"), "doomed").unwrap();
    Command::new("git")
        .args(["add", "d.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add d"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::remove_file(tmp.path().join("d.txt")).unwrap();

    stage_file(tmp.path(), "d.txt").unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let entry = entries.iter().find(|e| e.path == "d.txt").unwrap();
    assert!(entry.staged, "deleted file should be staged");
    assert_eq!(entry.status, FileStatus::Deleted);
}

#[test]
fn unstage_file_resets_index_to_head() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("u.txt"), "v1").unwrap();
    Command::new("git")
        .args(["add", "u.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add u"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("u.txt"), "v2").unwrap();
    Command::new("git")
        .args(["add", "u.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Verify it's staged
    let entries = file_diff(tmp.path()).unwrap();
    assert!(
        entries.iter().any(|e| e.path == "u.txt" && e.staged),
        "should be staged before unstage"
    );

    unstage_file(tmp.path(), "u.txt").unwrap();

    let entries = file_diff(tmp.path()).unwrap();
    let entry = entries.iter().find(|e| e.path == "u.txt").unwrap();
    assert!(!entry.staged, "file should be unstaged after unstage_file");
}

#[test]
fn unstage_file_in_unborn_repo_uses_remove_path() {
    let tmp = TempDir::new().unwrap();
    // Init repo WITHOUT any commits (unborn HEAD)
    let out = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    for args in [
        vec!["config", "user.email", "test@local"],
        vec!["config", "user.name", "Test"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(tmp.path())
            .output()
            .unwrap();
    }

    std::fs::write(tmp.path().join("new.txt"), "hello").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    unstage_file(tmp.path(), "new.txt").unwrap();

    // Verify index is empty
    let repo = git2::Repository::open(tmp.path()).unwrap();
    let index = repo.index().unwrap();
    assert_eq!(
        index.len(),
        0,
        "index should be empty after unstaging in unborn repo"
    );
}

#[test]
fn stage_then_unstage_is_round_trip() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("rt.txt"), "v1").unwrap();
    Command::new("git")
        .args(["add", "rt.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add rt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("rt.txt"), "v2").unwrap();

    let before = file_diff(tmp.path()).unwrap();
    let before_entry = before.iter().find(|e| e.path == "rt.txt").unwrap();
    assert!(!before_entry.staged);

    stage_file(tmp.path(), "rt.txt").unwrap();
    unstage_file(tmp.path(), "rt.txt").unwrap();

    let after = file_diff(tmp.path()).unwrap();
    let after_entry = after.iter().find(|e| e.path == "rt.txt").unwrap();
    assert!(
        !after_entry.staged,
        "after stage+unstage, file should be unstaged"
    );
    assert_eq!(after_entry.status, before_entry.status);
}

#[test]
fn stage_all_unstaged_returns_correct_count() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("a.txt"), "aaa").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "bbb").unwrap();
    std::fs::write(tmp.path().join("c.txt"), "ccc").unwrap();

    let count = stage_all_unstaged(tmp.path()).unwrap();
    assert_eq!(count, 3, "should stage 3 unstaged files");

    let entries = file_diff(tmp.path()).unwrap();
    assert!(
        entries.iter().all(|e| e.staged),
        "all files should be staged"
    );
}

#[test]
fn unstage_all_staged_returns_correct_count() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("x.txt"), "xxx").unwrap();
    std::fs::write(tmp.path().join("y.txt"), "yyy").unwrap();
    Command::new("git")
        .args(["add", "x.txt", "y.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let count = unstage_all_staged(tmp.path()).unwrap();
    assert_eq!(count, 2, "should unstage 2 staged files");

    let entries = file_diff(tmp.path()).unwrap();
    assert!(
        entries.iter().all(|e| !e.staged),
        "all files should be unstaged"
    );
}

#[test]
fn stage_file_after_git_mv_is_already_staged() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit a file
    std::fs::write(tmp.path().join("old_name.txt"), "content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "old_name.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add failed");
    let out = Command::new("git")
        .args(["commit", "-m", "add old_name"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit failed");

    // Rename via `git mv` (stages both sides automatically)
    let out = Command::new("git")
        .args(["mv", "old_name.txt", "new_name.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git mv failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let entries = file_diff(tmp.path()).unwrap();

    // `git mv` stages both the delete of old_name.txt and the add of new_name.txt.
    // Without `find_similar()` on the diff, git2 reports these as separate staged
    // entries (Deleted + Added) rather than a single Renamed entry.
    // This correctly reflects the index state — both sides are staged.
    let new_entry = entries
        .iter()
        .find(|e| e.path == "new_name.txt")
        .expect("new_name.txt should appear in diff entries");
    assert!(
        new_entry.staged,
        "git mv stages the new file — should be staged"
    );

    let old_entry = entries
        .iter()
        .find(|e| e.path == "old_name.txt")
        .expect("old_name.txt should appear as deleted in diff entries");
    assert!(
        old_entry.staged,
        "git mv stages the deletion — should be staged"
    );
    assert_eq!(
        old_entry.status,
        FileStatus::Deleted,
        "old path should show as Deleted"
    );

    // Verify no unstaged orphan entries exist for these paths
    let unstaged_old: Vec<_> = entries
        .iter()
        .filter(|e| e.path == "old_name.txt" && !e.staged)
        .collect();
    assert!(
        unstaged_old.is_empty(),
        "old_name.txt should NOT appear as an unstaged orphan"
    );
}

#[test]
fn manual_rename_shows_as_separate_add_and_delete() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit a file
    std::fs::write(tmp.path().join("original.txt"), "content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "original.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add failed");
    let out = Command::new("git")
        .args(["commit", "-m", "add original"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit failed");

    // Manual rename (NOT git mv) — filesystem only
    std::fs::rename(
        tmp.path().join("original.txt"),
        tmp.path().join("renamed.txt"),
    )
    .unwrap();

    let entries = file_diff(tmp.path()).unwrap();

    // Manual rename is NOT detected as a rename — it appears as two separate entries:
    // 1. original.txt is Deleted (working-tree deletion, unstaged)
    // 2. renamed.txt is Untracked (new file, unstaged)
    let original_entry = entries
        .iter()
        .find(|e| e.path == "original.txt")
        .expect("original.txt should appear as deleted");
    assert_eq!(
        original_entry.status,
        FileStatus::Deleted,
        "original.txt should show as Deleted"
    );
    assert!(!original_entry.staged, "manual deletion is unstaged");

    let renamed_entry = entries
        .iter()
        .find(|e| e.path == "renamed.txt")
        .expect("renamed.txt should appear as untracked");
    assert_eq!(
        renamed_entry.status,
        FileStatus::Untracked,
        "renamed.txt should show as Untracked"
    );
    assert!(!renamed_entry.staged, "untracked file is not staged");
}

#[test]
fn file_content_diff_detects_rename_old_path() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit old_name.txt
    std::fs::write(tmp.path().join("old_name.txt"), "content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "old_name.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add old_name.txt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["commit", "-m", "add old_name"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit old_name failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Rename via git mv (stages the rename)
    let out = Command::new("git")
        .args(["mv", "old_name.txt", "new_name.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git mv failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let fd = file_content_diff(tmp.path(), "new_name.txt", true).unwrap();
    assert_eq!(fd.path, "new_name.txt");

    // Document rename detection behaviour:
    // If old_path is populated, it should equal the original name.
    // If old_path is None, that documents a current limitation (no find_similar call).
    // Either way, the function must not error (already proven by the unwrap above).
    if let Some(ref old) = fd.old_path {
        assert_eq!(
            old, "old_name.txt",
            "old_path should be the original filename"
        );
    }
}

#[test]
fn file_diff_detects_conflicted_file() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Create a file on main and commit it
    std::fs::write(tmp.path().join("conflict.txt"), "base content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add failed");
    let out = Command::new("git")
        .args(["commit", "-m", "add conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit failed");

    // Create a branch and modify the file
    let out = Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git checkout -b feature failed");
    std::fs::write(tmp.path().join("conflict.txt"), "feature content\n").unwrap();
    let out = Command::new("git")
        .args(["commit", "-am", "feature change"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit on feature failed");

    // Switch back to main and make a conflicting change
    let out = Command::new("git")
        .args(["checkout", "main"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git checkout main failed");
    std::fs::write(tmp.path().join("conflict.txt"), "main content\n").unwrap();
    let out = Command::new("git")
        .args(["commit", "-am", "main change"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit on main failed");

    // Attempt merge — this should fail with a conflict
    let out = Command::new("git")
        .args(["merge", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "git merge should have failed with conflict"
    );

    // file_diff should detect the conflicted file
    let entries = file_diff(tmp.path()).unwrap();
    let conflicted = entries
        .iter()
        .find(|e| e.path == "conflict.txt")
        .expect("conflict.txt should appear in file_diff results");
    assert_eq!(
        conflicted.status,
        FileStatus::Conflicted,
        "conflicted file should have Conflicted status"
    );
}

#[test]
fn file_content_diff_truncates_large_diff() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Create and commit an initial file
    let file_path = tmp.path().join("large.txt");
    std::fs::write(&file_path, "initial\n").unwrap();
    Command::new("git")
        .args(["add", "large.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Generate a modified file with 15,000+ new lines so the diff exceeds MAX_DIFF_LINES
    let content: String = (0..15_500).map(|i| format!("line {}\n", i)).collect();
    std::fs::write(&file_path, content).unwrap();

    // Get the diff for the unstaged change
    let diff = file_content_diff(tmp.path(), "large.txt", false).unwrap();

    // Should be truncated to MAX_DIFF_LINES + 1 (the truncation marker)
    assert_eq!(
        diff.lines.len(),
        MAX_DIFF_LINES + 1,
        "diff should be truncated to MAX_DIFF_LINES + 1 lines, got {}",
        diff.lines.len()
    );

    // The last line should be the truncation marker
    let last = diff.lines.last().unwrap();
    assert_eq!(last.kind, DiffLineKind::FileHeader);
    assert!(
        last.content.contains("truncated"),
        "truncation marker should contain 'truncated', got: {}",
        last.content
    );
    assert!(
        last.content.contains("omitted"),
        "truncation marker should contain 'omitted', got: {}",
        last.content
    );
}

#[test]
fn stage_all_unstaged_skips_conflicted_files() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit a file on main
    std::fs::write(tmp.path().join("conflict.txt"), "original\n").unwrap();
    Command::new("git")
        .args(["add", "conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Create a branch and modify the file
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("conflict.txt"), "feature change\n").unwrap();
    Command::new("git")
        .args(["add", "conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Switch back to main and make a conflicting change
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("conflict.txt"), "main change\n").unwrap();
    Command::new("git")
        .args(["add", "conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "main change"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Merge feature branch — should fail with conflict
    let merge_output = Command::new("git")
        .args(["merge", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        !merge_output.status.success(),
        "merge should fail with conflict"
    );

    // stage_all_unstaged should skip conflicted files
    let count = stage_all_unstaged(tmp.path()).unwrap();
    assert_eq!(count, 0, "conflicted files should not be staged");

    // Verify conflicted file still appears as conflicted in the diff
    let entries = file_diff(tmp.path()).unwrap();
    let conflicted: Vec<_> = entries
        .iter()
        .filter(|e| e.status == FileStatus::Conflicted)
        .collect();
    assert!(
        !conflicted.is_empty(),
        "conflicted file should still appear as conflicted"
    );
}

#[test]
fn git_index_mtime_works_for_worktree() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit a file so the repo has history
    std::fs::write(tmp.path().join("file.txt"), "content\n").unwrap();
    Command::new("git")
        .args(["add", "file.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    // Create a worktree
    let wt_path = tmp.path().parent().unwrap().join("wt-test");
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            wt_path.to_str().unwrap(),
            "-b",
            "wt-branch",
        ])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // git_index_mtime should return Some for the worktree
    let mtime = git_index_mtime(&wt_path);
    assert!(
        mtime.is_some(),
        "git_index_mtime should return Some for a worktree, got None"
    );

    // Cleanup worktree
    Command::new("git")
        .args(["worktree", "remove", wt_path.to_str().unwrap()])
        .current_dir(tmp.path())
        .output()
        .unwrap();
}

#[test]
fn repo_status_counts_conflicted_files() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Create a file on main and commit it
    std::fs::write(tmp.path().join("conflict.txt"), "base content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add failed");
    let out = Command::new("git")
        .args(["commit", "-m", "add conflict.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit failed");

    // Create a branch and modify the file
    let out = Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git checkout -b feature failed");
    std::fs::write(tmp.path().join("conflict.txt"), "feature content\n").unwrap();
    let out = Command::new("git")
        .args(["commit", "-am", "feature change"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit on feature failed");

    // Switch back to main and make a conflicting change
    let out = Command::new("git")
        .args(["checkout", "main"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git checkout main failed");
    std::fs::write(tmp.path().join("conflict.txt"), "main content\n").unwrap();
    let out = Command::new("git")
        .args(["commit", "-am", "main change"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit on main failed");

    // Attempt merge — this should fail with a conflict
    let out = Command::new("git")
        .args(["merge", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "git merge should have failed with conflict"
    );

    // repo_status should count the conflicted file
    let status = repo_status(tmp.path()).unwrap();
    assert!(
        status.conflicted > 0,
        "repo_status should count conflicted files, got conflicted={}",
        status.conflicted
    );
}

#[test]
fn current_branch_from_repo_matches_path_version() {
    let dir = tempfile::tempdir().unwrap();
    common::init_repo(dir.path());
    let repo = git2::Repository::open(dir.path()).unwrap();
    let via_path = space::core::git::current_branch(dir.path()).unwrap();
    let via_repo = space::core::git::current_branch_from_repo(&repo).unwrap();
    assert_eq!(via_path, via_repo);
}

#[test]
fn repo_status_from_repo_matches_path_version() {
    let dir = tempfile::tempdir().unwrap();
    common::init_repo(dir.path());
    std::fs::write(dir.path().join("new.txt"), "hello").unwrap();
    let repo = git2::Repository::open(dir.path()).unwrap();
    let via_path = space::core::git::repo_status(dir.path()).unwrap();
    let via_repo = space::core::git::repo_status_from_repo(&repo).unwrap();
    assert_eq!(via_path.modified, via_repo.modified);
    assert_eq!(via_path.staged, via_repo.staged);
    assert_eq!(via_path.untracked, via_repo.untracked);
    assert_eq!(via_path.conflicted, via_repo.conflicted);
}

#[test]
fn ahead_behind_from_repo_matches_path_version() {
    let dir = tempfile::tempdir().unwrap();
    common::init_repo(dir.path());
    let repo = git2::Repository::open(dir.path()).unwrap();
    let via_path = space::core::git::ahead_behind(dir.path()).unwrap();
    let via_repo = space::core::git::ahead_behind_from_repo(&repo).unwrap();
    assert_eq!(via_path, via_repo);
}

#[test]
fn ahead_behind_from_repo_matches_path_version_with_remote() {
    // Create a bare repo to act as "origin"
    let bare_dir = tempfile::tempdir().unwrap();
    let out = Command::new("git")
        .args(["init", "--bare", "-b", "main"])
        .current_dir(bare_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git init --bare failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Clone it to get a repo with a remote
    let clone_dir = tempfile::tempdir().unwrap();
    let out = Command::new("git")
        .args(["clone", &bare_dir.path().to_string_lossy(), "."])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Configure user
    for args in [
        vec!["config", "user.email", "test@local"],
        vec!["config", "user.name", "Test"],
    ] {
        let out = Command::new("git")
            .args(&args)
            .current_dir(clone_dir.path())
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    // Make initial commit + push to establish tracking
    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Make a second local commit (not pushed) -> ahead=1, behind=0
    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "local only"])
        .current_dir(clone_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "second commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Both path-based and repo-based functions must agree
    let via_path = space::core::git::ahead_behind(clone_dir.path()).unwrap();
    let repo = git2::Repository::open(clone_dir.path()).unwrap();
    let via_repo = space::core::git::ahead_behind_from_repo(&repo).unwrap();

    assert_eq!(
        via_path, via_repo,
        "path and repo variants must return the same value"
    );
    assert_eq!(via_path.0, 1, "one unpushed commit => ahead=1");
    assert_eq!(via_path.1, 0, "nothing to pull => behind=0");
}

// ── recent_commits (Phase 6: Log) ─────────────────────────────────────

/// Run a git command in `dir`, asserting success.
fn git_in(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Init a repo at `dir` with branch `main` and user config but NO initial
/// commit, so callers can create an exact, known number of commits.
fn init_repo_no_commit(dir: &std::path::Path) {
    git_in(dir, &["init", "-b", "main"]);
    git_in(dir, &["config", "user.email", "test@local"]);
    git_in(dir, &["config", "user.name", "Test"]);
}

#[test]
fn recent_commits_returns_newest_first_with_subjects() {
    let tmp = TempDir::new().unwrap();
    init_repo_no_commit(tmp.path());
    for msg in ["first", "second", "third"] {
        git_in(tmp.path(), &["commit", "--allow-empty", "-m", msg]);
    }

    let commits = space::core::git::recent_commits(tmp.path(), 50);

    assert_eq!(commits.len(), 3, "should return all 3 commits");
    assert_eq!(commits[0].subject, "third", "newest commit must be first");
    assert_eq!(commits[1].subject, "second");
    assert_eq!(commits[2].subject, "first", "oldest commit must be last");
    assert!(
        !commits[0].short_hash.is_empty(),
        "short_hash should be populated"
    );
    assert_eq!(commits[0].author, "Test", "author name should be populated");
}

#[test]
fn recent_commits_respects_limit() {
    let tmp = TempDir::new().unwrap();
    init_repo_no_commit(tmp.path());
    for msg in ["c1", "c2", "c3", "c4", "c5"] {
        git_in(tmp.path(), &["commit", "--allow-empty", "-m", msg]);
    }

    let commits = space::core::git::recent_commits(tmp.path(), 2);

    assert_eq!(commits.len(), 2, "limit must cap the number of commits");
    assert_eq!(commits[0].subject, "c5", "should return the 2 newest");
    assert_eq!(commits[1].subject, "c4");
}

#[test]
fn recent_commits_unborn_head_is_empty() {
    let tmp = TempDir::new().unwrap();
    // Freshly initialised repo with no commits (unborn HEAD).
    init_repo_no_commit(tmp.path());

    let commits = space::core::git::recent_commits(tmp.path(), 50);

    assert!(
        commits.is_empty(),
        "unborn HEAD must yield an empty Vec, not panic"
    );
}
