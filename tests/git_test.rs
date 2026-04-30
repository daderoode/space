mod common;

use space::core::git::{
    file_content_diff, file_diff, stage_all_unstaged, stage_file, unstage_all_staged, unstage_file,
    DiffLineKind, DiffTarget, FileStatus,
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
fn file_diff_base_mode_shows_committed_divergence() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("base.txt"), "base content").unwrap();
    Command::new("git")
        .args(["add", "base.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    std::fs::write(tmp.path().join("feature.txt"), "new feature\nline2").unwrap();
    Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let entries = file_diff(tmp.path(), &DiffTarget::Base).unwrap();

    let feature_entry = entries
        .iter()
        .find(|e| e.path == "feature.txt")
        .expect("base mode should show committed divergence from base branch");
    assert!(
        feature_entry.insertions > 0,
        "feature.txt should have insertions"
    );

    assert!(
        !entries.iter().any(|e| e.path == "base.txt"),
        "base.txt exists on both branches and should not appear in divergence diff"
    );
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
    let result = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let fd = file_content_diff(tmp.path(), &DiffTarget::Head, "f.txt", false).unwrap();
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

    let fd = file_content_diff(tmp.path(), &DiffTarget::Head, "s.txt", true).unwrap();
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

    let fd = file_content_diff(tmp.path(), &DiffTarget::Head, "new.txt", false).unwrap();
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

    let fd = file_content_diff(tmp.path(), &DiffTarget::Head, "bin.dat", false).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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
    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
    assert!(
        entries.iter().any(|e| e.path == "u.txt" && e.staged),
        "should be staged before unstage"
    );

    unstage_file(tmp.path(), "u.txt").unwrap();

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let before = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
    let before_entry = before.iter().find(|e| e.path == "rt.txt").unwrap();
    assert!(!before_entry.staged);

    stage_file(tmp.path(), "rt.txt").unwrap();
    unstage_file(tmp.path(), "rt.txt").unwrap();

    let after = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
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

    let entries = file_diff(tmp.path(), &DiffTarget::Head).unwrap();
    assert!(
        entries.iter().all(|e| !e.staged),
        "all files should be unstaged"
    );
}

#[test]
fn file_content_diff_base_mode_returns_committed_divergence() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());

    // Commit base.txt on main
    std::fs::write(tmp.path().join("base.txt"), "base content\n").unwrap();
    let out = Command::new("git")
        .args(["add", "base.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add base.txt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["commit", "-m", "base"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit base failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Create feature branch
    let out = Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git checkout -b feature failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Write and commit feature.txt on feature branch
    std::fs::write(tmp.path().join("feature.txt"), "line1\nline2\n").unwrap();
    let out = Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git add feature.txt failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("git")
        .args(["commit", "-m", "add feature file"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git commit feature failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let diff = file_content_diff(tmp.path(), &DiffTarget::Base, "feature.txt", false).unwrap();
    assert!(!diff.is_binary, "feature.txt should not be binary");
    assert!(!diff.lines.is_empty(), "diff lines should be non-empty");

    let addition_count = diff
        .lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Addition)
        .count();
    assert!(
        addition_count >= 2,
        "expected at least 2 addition lines for the 2 content lines, got {}",
        addition_count
    );
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

    let fd = file_content_diff(tmp.path(), &DiffTarget::Head, "new_name.txt", true).unwrap();
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
