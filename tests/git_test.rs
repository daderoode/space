mod common;

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
fn dirty_repo_status_counts_correctly() {
    let tmp = TempDir::new().unwrap();
    common::init_repo(tmp.path());
    std::fs::write(tmp.path().join("new_file.txt"), "hello").unwrap();
    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.untracked, 1);
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
