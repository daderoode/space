use std::process::Command;
use tempfile::TempDir;

fn init_bare_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(dir)
        .output()
        .unwrap();
}

#[test]
fn detects_base_branch() {
    let tmp = TempDir::new().unwrap();
    init_bare_repo(tmp.path());
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
    init_bare_repo(tmp.path());
    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.modified, 0);
    assert_eq!(status.staged, 0);
    assert_eq!(status.untracked, 0);
}

#[test]
fn dirty_repo_status_counts_correctly() {
    let tmp = TempDir::new().unwrap();
    init_bare_repo(tmp.path());
    std::fs::write(tmp.path().join("new_file.txt"), "hello").unwrap();
    let status = space::core::git::repo_status(tmp.path()).unwrap();
    assert_eq!(status.untracked, 1);
}

#[test]
fn lists_branches() {
    let tmp = TempDir::new().unwrap();
    init_bare_repo(tmp.path());
    Command::new("git")
        .args(["branch", "feature-x"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    let branches = space::core::git::list_branches(tmp.path()).unwrap();
    assert!(branches.len() >= 2);
    assert!(branches.iter().any(|b| b.name == "feature-x"));
}

#[test]
fn current_branch_returns_branch_name() {
    let tmp = TempDir::new().unwrap();
    init_bare_repo(tmp.path());
    let branch = space::core::git::current_branch(tmp.path()).unwrap();
    assert_eq!(branch, "main");
}
