mod common;

use assert_cmd::Command;
use common::TestEnv;
use predicates::prelude::*;
use space::core::workspace::{create_worktree, BranchStrategy};
/// Build a `space` Command wired to the test environment.
fn space(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin("space").unwrap();
    cmd.env("SPACE_CONFIG_DIR", &env.config_dir);
    // Disable colour codes so assertions match plain text.
    cmd.env("NO_COLOR", "1");
    cmd
}

// ---------------------------------------------------------------------------
// 1. ls – no workspaces
// ---------------------------------------------------------------------------
#[test]
fn ls_no_workspaces_succeeds() {
    let env = TestEnv::new();
    space(&env)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("No workspaces"));
}

// ---------------------------------------------------------------------------
// 2. ls – shows workspace names
// ---------------------------------------------------------------------------
#[test]
fn ls_shows_workspace_names() {
    let env = TestEnv::new();

    // Create a workspace dir with a repo subdir containing a .git marker
    let ws_path = env.workspaces_dir.join("my-feature");
    let repo_in_ws = ws_path.join("some-repo");
    std::fs::create_dir_all(&repo_in_ws).unwrap();
    // A .git *file* (not directory) is enough — list.rs checks .join(".git").exists()
    std::fs::write(repo_in_ws.join(".git"), "gitdir: /dev/null").unwrap();

    space(&env)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("my-feature"));
}

// ---------------------------------------------------------------------------
// 3. ls -v – shows branch info
// ---------------------------------------------------------------------------
#[test]
fn ls_verbose_shows_branch_info() {
    let env = TestEnv::new();

    let repo_path = env.create_repo("alpha");
    let ws_name = "verbose-ws";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("feat-x".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["ls", "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains(ws_name).and(predicate::str::contains("feat-x")));
}

// ---------------------------------------------------------------------------
// 4. status – existing workspace
// ---------------------------------------------------------------------------
#[test]
fn status_existing_workspace() {
    let env = TestEnv::new();

    let repo_path = env.create_repo("beta");
    let ws_name = "status-ws";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("feat-y".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["status", ws_name])
        .assert()
        .success()
        .stdout(predicate::str::contains("beta").and(predicate::str::contains("feat-y")));
}

// ---------------------------------------------------------------------------
// 5. status – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn status_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["status", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 6. repos – lists from cache
// ---------------------------------------------------------------------------
#[test]
fn repos_lists_from_cache() {
    let env = TestEnv::new();
    // Write cache pointing to a path that does NOT exist on disk.
    // The only way the name appears in output is via the cache.
    let fake_path = std::path::PathBuf::from("/nonexistent/repos/phantom-repo");
    env.write_cache(&[fake_path]);

    space(&env)
        .arg("repos")
        .assert()
        .success()
        .stdout(predicate::str::contains("phantom-repo"));
}

// ---------------------------------------------------------------------------
// 7. repos --refresh – rescans
// ---------------------------------------------------------------------------
#[test]
fn repos_refresh_rescans() {
    let env = TestEnv::new();
    env.create_repo("gamma");

    // No cache file exists initially
    let cache_path = env.config_dir.join("repos.cache");
    assert!(
        !cache_path.exists(),
        "cache should not exist before refresh"
    );

    space(&env)
        .args(["repos", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gamma"));

    // Cache file should now exist and contain the repo
    assert!(
        cache_path.exists(),
        "cache should be written after --refresh"
    );
    let content = std::fs::read_to_string(&cache_path).unwrap();
    assert!(
        content.contains("gamma"),
        "cache should contain scanned repo"
    );
}

// ---------------------------------------------------------------------------
// 8. go – existing workspace emits cd marker
// ---------------------------------------------------------------------------
#[test]
fn go_existing_emits_cd() {
    let env = TestEnv::new();
    let ws_path = env.workspaces_dir.join("jump-ws");
    std::fs::create_dir_all(&ws_path).unwrap();

    space(&env)
        .args(["go", "jump-ws"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "__SPACE_CD__:{}",
            ws_path.display()
        )));
}

// ---------------------------------------------------------------------------
// 9. go – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn go_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["go", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 10. rm --force – removes workspace
// ---------------------------------------------------------------------------
#[test]
fn rm_force_removes_workspace() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("ephemeral");
    let ws_name = "doomed";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("temp-branch".to_string()),
    )
    .unwrap();

    let ws_path = env.workspaces_dir.join(ws_name);
    assert!(ws_path.exists());

    space(&env)
        .args(["rm", "--force", ws_name])
        .assert()
        .success();

    assert!(!ws_path.exists(), "workspace dir should be deleted");

    // Verify worktree is unlinked from the main repo
    let output = std::process::Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let wt_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        !wt_list.contains(ws_name),
        "worktree should be unlinked after rm --force"
    );
}

// ---------------------------------------------------------------------------
// 11. rm --force – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn rm_force_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["rm", "--force", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 12. go – writes cd target to file when __SPACE_CD_FILE__ is set
// ---------------------------------------------------------------------------
#[test]
fn go_writes_cd_file_when_env_set() {
    let env = TestEnv::new();
    let ws_path = env.workspaces_dir.join("cd-ws");
    std::fs::create_dir_all(&ws_path).unwrap();

    let cd_file = env.dir.path().join("cd_target");

    space(&env)
        .args(["go", "cd-ws"])
        .env("__SPACE_CD_FILE__", &cd_file)
        .assert()
        .success();

    assert!(cd_file.exists(), "cd file should be written");
    let content = std::fs::read_to_string(&cd_file).unwrap();
    assert_eq!(
        content,
        ws_path.display().to_string(),
        "cd file should contain workspace path"
    );
}

// ---------------------------------------------------------------------------
// __complete workspaces -- lists workspace names with context
// ---------------------------------------------------------------------------
#[test]
fn complete_workspaces_lists_names() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("alpha");
    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "my-feature",
        &BranchStrategy::NewBranch("feat-x".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["__complete", "workspaces"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-feature"))
        .stdout(predicate::str::contains("feat-x"));
}

// ---------------------------------------------------------------------------
// __complete workspaces -- empty when none exist
// ---------------------------------------------------------------------------
#[test]
fn complete_workspaces_empty() {
    let env = TestEnv::new();
    space(&env)
        .args(["__complete", "workspaces"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// ---------------------------------------------------------------------------
// __complete repos -- lists from cache
// ---------------------------------------------------------------------------
#[test]
fn complete_repos_lists_cached() {
    let env = TestEnv::new();
    let repo_path = env.repos_dir.join("my-service");
    std::fs::create_dir_all(&repo_path).unwrap();
    env.write_cache(&[repo_path.clone()]);

    space(&env)
        .args(["__complete", "repos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-service"));
}

// ---------------------------------------------------------------------------
// __complete available-repos -- filters out existing repos
// ---------------------------------------------------------------------------
#[test]
fn complete_available_repos_filters_existing() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let beta = env.create_repo("beta");
    env.write_cache(&[alpha.clone(), beta.clone()]);

    create_worktree(
        &alpha,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("feat".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["__complete", "available-repos", "test-ws"])
        .assert()
        .success()
        .stdout(predicate::str::contains("beta"))
        .stdout(predicate::str::contains("alpha").not());
}

// ---------------------------------------------------------------------------
// init zsh -- outputs wrapper function and completions
// ---------------------------------------------------------------------------
#[test]
fn init_zsh_outputs_wrapper_and_completions() {
    let env = TestEnv::new();
    let output = space(&env).args(["init", "zsh"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Contains shell wrapper
    assert!(
        stdout.contains("__SPACE_CD_FILE__"),
        "init output should contain the shell wrapper"
    );
    // Contains completion function
    assert!(
        stdout.contains("compdef _space space"),
        "init output should contain the completion registration"
    );
}

// ---------------------------------------------------------------------------
// init -- unsupported shell returns error
// ---------------------------------------------------------------------------
#[test]
fn init_unsupported_shell_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["init", "fish"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported"));
}
