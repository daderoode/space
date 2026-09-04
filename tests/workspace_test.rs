mod common;

use common::TestEnv;
use space::core::workspace::{create_worktree, list_workspaces, BranchStrategy};
use std::process::Command;
use tempfile::TempDir;

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
    common::init_repo(repo_dir.path());
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
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();

    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    assert!(wt_path.exists());
    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert!(
        branch.starts_with('(') && branch.ends_with(')'),
        "worktree should be in detached HEAD, got: {}",
        branch
    );
}

#[test]
fn create_worktree_reuses_existing_local_branch() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    // Create the branch first
    Command::new("git")
        .args(["branch", "my-feature"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    let ws_dir = TempDir::new().unwrap();
    // Should succeed by checking out existing branch, not error with "already exists"
    let result = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "my-feature",
        &BranchStrategy::NewBranch("my-feature".to_string()),
    );
    assert!(
        result.is_ok(),
        "should reuse existing local branch: {:?}",
        result
    );
    let wt_path = result.unwrap();
    assert!(wt_path.join(".git").exists());
    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert_eq!(
        branch, "my-feature",
        "worktree should be on the reused branch"
    );
}

#[test]
fn create_worktree_errors_when_branch_already_checked_out() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    // "main" is already checked out in repo_dir — try to create worktree on it
    let ws_dir = TempDir::new().unwrap();
    let result = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::ExistingBranch("main".to_string()),
    );
    assert!(
        result.is_err(),
        "should error when branch is already checked out"
    );
}

#[test]
fn workspace_detail_returns_repo_info() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("my-repo");

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("test-ws".to_string()),
    )
    .unwrap();

    let ws = space::core::workspace::workspace_detail(&env.workspaces_dir, "test-ws").unwrap();
    assert_eq!(ws.name, "test-ws");
    assert_eq!(ws.repos.len(), 1);
    assert_eq!(ws.repos[0].name, "my-repo");
    assert_eq!(ws.repos[0].branch, "test-ws");
    assert_eq!(ws.repos[0].status.modified, 0);
    assert_eq!(ws.repos[0].status.staged, 0);
    assert_eq!(ws.repos[0].status.untracked, 0);
}

#[test]
fn workspace_detail_skips_non_repo_entries() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("real-repo");

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("test-ws".to_string()),
    )
    .unwrap();

    // Add a directory without .git (should be skipped)
    std::fs::create_dir_all(env.workspaces_dir.join("test-ws").join("not-a-repo")).unwrap();
    // Add a regular file (should be skipped)
    std::fs::write(env.workspaces_dir.join("test-ws").join("README.md"), "hi").unwrap();

    let ws = space::core::workspace::workspace_detail(&env.workspaces_dir, "test-ws").unwrap();
    assert_eq!(ws.repos.len(), 1, "should only find the real repo");
    assert_eq!(ws.repos[0].name, "real-repo");
}

#[test]
fn workspace_detail_not_found_errors() {
    let env = common::TestEnv::new();
    let result = space::core::workspace::workspace_detail(&env.workspaces_dir, "ghost");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn remove_workspace_unlinks_worktrees() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("my-repo");

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("test-ws".to_string()),
    )
    .unwrap();

    // Verify worktree is registered
    let output = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "git worktree list failed");
    let before = String::from_utf8_lossy(&output.stdout);
    assert!(
        before.contains("test-ws"),
        "worktree should be registered before removal"
    );

    // Remove
    space::core::workspace::remove_workspace(&env.workspaces_dir, "test-ws", true).unwrap();

    // Directory gone
    assert!(!env.workspaces_dir.join("test-ws").exists());

    // Worktree unlinked from main repo
    let output = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git worktree list failed after removal"
    );
    let after = String::from_utf8_lossy(&output.stdout);
    assert!(
        !after.contains("test-ws"),
        "worktree should be unlinked after removal"
    );
}

#[test]
fn remove_workspace_not_found_errors() {
    let env = common::TestEnv::new();
    let result = space::core::workspace::remove_workspace(&env.workspaces_dir, "ghost", true);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn create_worktree_existing_branch_strategy() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());

    let out = Command::new("git")
        .args(["branch", "feature-x"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git branch failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ws_dir = TempDir::new().unwrap();
    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::ExistingBranch("feature-x".to_string()),
    )
    .unwrap();

    assert!(wt_path.exists());
    assert!(wt_path.join(".git").exists());

    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "feature-x");
}

#[test]
fn remove_workspace_without_force() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("my-repo");

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("test-ws".to_string()),
    )
    .unwrap();

    // Remove without force (clean worktree, so should succeed)
    space::core::workspace::remove_workspace(&env.workspaces_dir, "test-ws", false).unwrap();
    assert!(
        !env.workspaces_dir.join("test-ws").exists(),
        "workspace should be removed"
    );
}

#[test]
fn list_workspaces_nonexistent_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does-not-exist");
    let workspaces = list_workspaces(&nonexistent).unwrap();
    assert!(
        workspaces.is_empty(),
        "nonexistent dir should return empty list"
    );
}

#[test]
fn workspace_repo_skeletons_returns_placeholder_branch() {
    let env = TestEnv::new();
    let ws_dir = env.workspaces_dir.join("my-ws");
    std::fs::create_dir_all(&ws_dir).unwrap();
    // Create a subdirectory with a .git entry (sufficient for skeleton detection)
    let repo_dir = ws_dir.join("alpha");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join(".git"), "gitdir: /fake").unwrap();

    let skeletons = space::core::workspace::workspace_repo_skeletons(&env.workspaces_dir, "my-ws");

    assert_eq!(skeletons.len(), 1);
    assert_eq!(skeletons[0].name, "alpha");
    assert_eq!(skeletons[0].branch, "...");
    assert_eq!(skeletons[0].status.modified, 0);
    assert_eq!(skeletons[0].ahead, 0);
}

#[test]
fn workspace_repo_skeletons_skips_non_git_dirs() {
    let env = TestEnv::new();
    let ws_dir = env.workspaces_dir.join("my-ws");
    std::fs::create_dir_all(ws_dir.join("not-a-repo")).unwrap();
    std::fs::write(env.workspaces_dir.join("my-ws").join("some-file.txt"), "x").unwrap();

    // The workspace dir exists but has no .git subdirs
    let skeletons = space::core::workspace::workspace_repo_skeletons(&env.workspaces_dir, "my-ws");
    assert!(skeletons.is_empty());
}

#[test]
fn switch_worktree_branch_new_branch_from_detached_head() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();

    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    space::core::workspace::switch_worktree_branch(&wt_path, "my-feature", true).unwrap();

    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "my-feature");
}

#[test]
fn switch_worktree_branch_existing_local_branch() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());

    Command::new("git")
        .args(["branch", "existing-branch"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();

    let ws_dir = TempDir::new().unwrap();
    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    space::core::workspace::switch_worktree_branch(&wt_path, "existing-branch", false).unwrap();

    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert_eq!(branch, "existing-branch");
}

#[test]
fn switch_worktree_branch_nonexistent_branch_errors() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();
    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    let result = space::core::workspace::switch_worktree_branch(&wt_path, "ghost-branch", false);
    assert!(
        result.is_err(),
        "switching to nonexistent branch should fail"
    );
}

#[test]
fn recent_branches_excludes_remote_refs() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());

    let head_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    assert!(head_out.status.success(), "git rev-parse HEAD failed");
    let sha = String::from_utf8_lossy(&head_out.stdout).trim().to_string();

    let status = Command::new("git")
        .args(["update-ref", "refs/remotes/origin/remote-only", &sha])
        .current_dir(repo_dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git update-ref failed");

    let branches = space::core::git::recent_branches(repo_dir.path(), 10);
    assert!(
        branches.iter().all(|b| !b.is_remote),
        "recent_branches must not include remote-tracking refs"
    );
}

#[test]
fn switch_worktree_branch_origin_prefix_creates_local_tracking_branch() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());

    let head_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir.path())
        .output()
        .unwrap();
    assert!(head_out.status.success(), "git rev-parse HEAD failed");
    let sha = String::from_utf8_lossy(&head_out.stdout).trim().to_string();

    // Create a remote-tracking ref without a real remote
    let status = Command::new("git")
        .args(["update-ref", "refs/remotes/origin/feature-x", &sha])
        .current_dir(repo_dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git update-ref failed");

    let ws_dir = TempDir::new().unwrap();
    let wt_path = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    // Pass the branch as "origin/feature-x" (as the full picker emits it)
    space::core::workspace::switch_worktree_branch(&wt_path, "origin/feature-x", false).unwrap();

    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert_eq!(
        branch, "feature-x",
        "should create local 'feature-x' from origin/feature-x"
    );
}

// ---------------------------------------------------------------------------
// Cancellable creation (the Creating stage's background worker)
// ---------------------------------------------------------------------------

/// Checkpoint 2: a flag that is already set stops the attempt before
/// `git worktree add` runs, so nothing lands on disk.
///
/// A preset flag exercises the same code path as a flag set while the fetch
/// was running: checkpoint 2 is a single branch on one `load`, and the flag is
/// monotonic (set once, never cleared), so *when* it was set cannot change
/// which side of that branch runs. Presetting it only removes the thread
/// scheduling a test would otherwise have to win.
#[test]
fn create_worktree_cancellable_stops_before_the_add() {
    use space::core::workspace::{create_worktree_cancellable, PreCreateFetch};
    use std::sync::atomic::AtomicBool;

    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();
    let repo_name = repo_dir.path().file_name().unwrap().to_owned();

    let attempt = create_worktree_cancellable(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
        PreCreateFetch::Skip,
        &AtomicBool::new(true),
    );

    let err = attempt
        .created
        .expect_err("a cancelled attempt must not report a created worktree");
    assert!(
        err.to_string().contains("cancelled"),
        "the error must name the cancellation, got {:?}",
        err.to_string()
    );
    assert!(
        !ws_dir.path().join("test-ws").join(&repo_name).exists(),
        "cancelling before the add must leave no worktree on disk"
    );
}

/// Checkpoint 1: the pre-create fetch does not run either, and `fetch: None`
/// is how the caller observes that.
#[test]
fn create_worktree_cancellable_skips_the_pre_create_fetch() {
    use space::core::workspace::{create_worktree_cancellable, PreCreateFetch};
    use std::sync::atomic::AtomicBool;

    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();
    let repo_name = repo_dir.path().file_name().unwrap().to_owned();

    let attempt = create_worktree_cancellable(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
        PreCreateFetch::Run(std::time::Duration::from_secs(5)),
        &AtomicBool::new(true),
    );

    assert!(
        attempt.fetch.is_none(),
        "a cancelled attempt must not have run its fetch, got {:?}",
        attempt.fetch
    );
    assert!(
        attempt.created.is_err(),
        "a cancelled attempt must not report a created worktree"
    );
    assert!(
        !ws_dir.path().join("test-ws").join(&repo_name).exists(),
        "cancelling before the fetch must leave no worktree on disk"
    );
}

/// The uncancelled path is the one the flow actually takes, and it is what
/// makes the two cancellation tests above non-vacuous: the same call with an
/// unset flag creates the worktree.
#[test]
fn create_worktree_cancellable_creates_when_the_flag_is_unset() {
    use space::core::workspace::{create_worktree_cancellable, PreCreateFetch};
    use std::sync::atomic::AtomicBool;

    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let ws_dir = TempDir::new().unwrap();

    let attempt = create_worktree_cancellable(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
        PreCreateFetch::Skip,
        &AtomicBool::new(false),
    );

    assert!(
        attempt.fetch.is_none(),
        "PreCreateFetch::Skip runs no fetch whatever the flag says"
    );
    let wt_path = attempt.created.expect("uncancelled creation must succeed");
    assert!(wt_path.join(".git").exists(), "worktree should have .git");
    let branch = space::core::git::current_branch(&wt_path).unwrap();
    assert!(
        branch.starts_with('(') && branch.ends_with(')'),
        "the strategy must still be applied, expected detached HEAD, got {}",
        branch
    );
}

/// `refuses_because_checked_out` documents a limit rather than asserting a
/// desirable one: it matches git's pre-2.42 wording only. On git 2.50.1
/// (Apple Git-155) `git worktree add` refuses with "is already used by
/// worktree at", which this does not match, so the strategy-picker bounce it
/// gates is dormant there and the generic failure path runs instead.
#[test]
fn refuses_because_checked_out_matches_only_the_pre_2_42_wording() {
    use space::core::workspace::refuses_because_checked_out;

    assert!(
        refuses_because_checked_out("fatal: 'main' is already checked out at '/x'"),
        "the pre-2.42 wording is the one this predicate was written for"
    );
    assert!(
        !refuses_because_checked_out("fatal: 'main' is already used by worktree at '/x'"),
        "git 2.50.1's wording is NOT matched: the bounce is dormant on modern git"
    );
    assert!(
        !refuses_because_checked_out("fatal: not a git repository"),
        "an unrelated refusal must not bounce to the strategy picker"
    );
}

/// The two checkpoints are two reads, not one read cached at entry, and this is
/// the only test that can tell those apart. It sets the flag DURING the fetch,
/// after checkpoint 1 has already passed with the flag clear.
///
/// That matters because the fetch is the window this design exists for: up to
/// `UNATTENDED_FETCH_TIMEOUT` in which the user can press Esc. A single read at
/// entry would still stop a run cancelled before it started, and would sail
/// straight through a cancel arriving during the fetch, creating the very
/// worktree the user pressed Esc to avoid.
///
/// The ordering is enforced by files, not by sleeps. `remote.origin.uploadpack`
/// points at a script that touches STARTED and then blocks until RELEASE
/// appears, so the fetch cannot finish until this test lets it. The test waits
/// for STARTED, at which point the fetch is provably running and checkpoint 1
/// has provably passed with the flag clear, sets the flag, and only then
/// touches RELEASE. Checkpoint 2 therefore always reads a flag that was false
/// at entry and true by the time the fetch returned, with no assumption about
/// ordering anywhere.
///
/// Visibility is a separate question from ordering, and this test does rely on
/// one property. The flipper's store is `Relaxed` and the chain from it to the
/// checkpoint's load runs through a file write, a child process and a `wait`,
/// with no Rust synchronisation between the two threads. What guarantees the
/// load sees the store is atomic coherence, that is, eventual visibility, plus
/// the fetch taking non-zero time to return. That is safe on every platform
/// this app supports and is the same property production relies on for this
/// flag. It is written down because a reader who believes the memory model is
/// doing the ordering work will make a confident wrong edit, and this test is
/// the only thing standing between a collapsed checkpoint and a silent
/// 60-second hang.
///
/// Collapsing both reads into one at entry, which `create_worktree_cancellable`
/// explicitly forbids in its own doc comment, passes every other test in this
/// repository and fails this one.
#[test]
fn create_worktree_cancellable_reads_the_flag_again_after_the_fetch() {
    use space::core::workspace::{create_worktree_cancellable, PreCreateFetch};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let tmp = TempDir::new().unwrap();
    let origin = tmp.path().join("origin.git");
    assert!(Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&origin)
        .status()
        .unwrap()
        .success());

    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo_dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let url = format!("file://{}", origin.display());
    git(&["remote", "add", "origin", &url]);
    git(&["push", "-q", "origin", "HEAD:refs/heads/main"]);

    // The gate. `sh` is fine here: the app documents macOS and Linux only.
    let started = tmp.path().join("STARTED");
    let release = tmp.path().join("RELEASE");
    let gate = tmp.path().join("gate.sh");
    std::fs::write(
        &gate,
        format!(
            "#!/bin/sh\ntouch \"{}\"\nwhile [ ! -f \"{}\" ]; do sleep 0.01; done\nexec git upload-pack \"$@\"\n",
            started.display(),
            release.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let gate_path = gate.display().to_string();
    git(&["config", "remote.origin.uploadpack", &gate_path]);

    let ws_dir = TempDir::new().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));

    let flipper = {
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            // Comfortably shorter than the fetch's own limit below. If the
            // gate never starts, this fires first and says so, instead of
            // expiring together with the timeout it exists to test around and
            // leaving the failure ambiguous.
            let deadline = Instant::now() + Duration::from_secs(20);
            while !started.exists() {
                assert!(Instant::now() < deadline, "the gated fetch never started");
                std::thread::sleep(Duration::from_millis(2));
            }
            // Strictly ordered: the flag is set before the fetch is released,
            // so checkpoint 1 cannot have seen it and checkpoint 2 must.
            cancel.store(true, Ordering::Relaxed);
            std::fs::write(&release, b"go").unwrap();
        })
    };

    let attempt = create_worktree_cancellable(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::DetachedHead,
        PreCreateFetch::Run(Duration::from_secs(60)),
        &cancel,
    );
    flipper.join().unwrap();

    assert!(
        attempt.fetch.is_some(),
        "checkpoint 1 saw a clear flag, so the fetch must have run: that is \
         what makes this a test of the SECOND read"
    );
    assert!(
        attempt.created.is_err(),
        "a cancel observed after the fetch must stop the attempt"
    );
    let wt = ws_dir
        .path()
        .join("test-ws")
        .join(repo_dir.path().file_name().unwrap());
    assert!(
        !wt.exists(),
        "no worktree may be created for a repo cancelled during its fetch"
    );
}
