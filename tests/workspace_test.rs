use space::core::workspace::{create_worktree, list_workspaces, BranchStrategy};
use std::process::Command;
use tempfile::TempDir;

fn init_repo(dir: &std::path::Path) {
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
        .args(["config", "user.name", "T"])
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
fn list_workspaces_returns_directories() {
    let ws_dir = TempDir::new().unwrap();
    std::fs::create_dir(ws_dir.path().join("alpha")).unwrap();
    std::fs::create_dir(ws_dir.path().join("beta")).unwrap();

    let workspaces = list_workspaces(ws_dir.path()).unwrap();
    let names: Vec<&str> = workspaces.iter().map(|w| w.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}

#[test]
fn create_worktree_new_branch_strategy() {
    let repo_dir = TempDir::new().unwrap();
    init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();

    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::NewBranch("test-ws".to_string()),
    )
    .unwrap();

    assert!(wt_path.exists(), "worktree directory should exist");
    assert!(wt_path.join(".git").exists(), "worktree should have .git");
}

#[test]
fn create_worktree_detached_head_strategy() {
    let repo_dir = TempDir::new().unwrap();
    init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();

    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    assert!(wt_path.exists());
}
