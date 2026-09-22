#![allow(clippy::disallowed_methods)] // fixtures start git directly (ADR 0002)

mod common;

use common::TestEnv;
use space::core::workspace::{create_worktree, list_workspaces, BranchStrategy};
use std::path::{Path, PathBuf};
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

/// `is_err()` alone was not a test of what its name claimed. `create_worktree`
/// returns an error for every reason git can refuse an add, so the assertion
/// held whether the refusal was recognised as "the branch is checked out
/// elsewhere" or fell through to the generic failure path, and those are the
/// two outcomes the Creating stage acts on differently: only the first bounces
/// the user back to the branch-strategy picker. This pins the refusal against
/// the predicate that gates that bounce, on whatever wording the local git uses.
#[test]
fn create_worktree_refuses_a_checked_out_branch_as_pick_another_strategy() {
    let repo_dir = TempDir::new().unwrap();
    common::init_repo(repo_dir.path());
    // "main" is already checked out in repo_dir, so the add must be refused.
    let ws_dir = TempDir::new().unwrap();
    let err = create_worktree(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::ExistingBranch("main".to_string()),
    )
    .expect_err("a branch checked out in the source repo cannot be added again");
    let text = format!("{}", err);

    assert!(
        space::core::workspace::refuses_because_checked_out(&text),
        "the refusal must be recognised as 'pick another strategy', got {:?}",
        text
    );
    assert!(
        text.contains("main"),
        "the refusal must name the checked-out branch, got {:?}",
        text
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

/// Git renamed this refusal mid-flight and both spellings are still in the
/// wild, so the predicate has to know both: "is already used by worktree at"
/// (git 2.42 and later, including the 2.50.1 this is developed on) and "is
/// already checked out at" (git 2.38 to 2.41). A wording it misses is a user
/// shown a generic failure instead of the branch-strategy picker the stage
/// bounces to, which is what a wording drift cost once already.
#[test]
fn refuses_because_checked_out_matches_both_git_wordings() {
    use space::core::workspace::refuses_because_checked_out;

    assert!(
        refuses_because_checked_out("fatal: 'main' is already checked out at '/x'"),
        "git 2.38 to 2.41 wording must bounce to the strategy picker"
    );
    assert!(
        refuses_because_checked_out("fatal: 'main' is already used by worktree at '/x'"),
        "git 2.42 and later wording must bounce to the strategy picker"
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
/// points at a script that marks that it is holding and then blocks until it
/// is released, so the fetch cannot finish until this test lets it. The test
/// waits for that mark, at which point the fetch is provably running and checkpoint 1
/// has provably passed with the flag clear, sets the flag, and only then
/// releases it. Checkpoint 2 therefore always reads a flag that was false
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
    use std::time::Duration;

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

    // The gate is the shared hold (`common::hold`) in front of the real
    // upload-pack: it marks that it is holding, waits for the release, records
    // that it saw it, and only then serves the fetch. It gives up without that
    // record once the test can no longer release it or after its cap. git
    // runs in its own session, so a killed test binary leaves nothing else to
    // stop it, and the fetch limit below dies with the test process; the pid
    // guard is what ends it then (ticket 31). A panic in the flipper below
    // does not drop this TempDir, which this thread owns while it is inside
    // the fetch, so on that path only the fetch limit ends the hold; the
    // marker guard covers the return and drop paths.
    let hold = Arc::new(common::hold::Hold::new(tmp.path(), "gate"));
    let gate = tmp.path().join("gate.sh");
    hold.write_script(&gate, "exec git upload-pack \"$@\"");
    let gate_path = gate.display().to_string();
    git(&["config", "remote.origin.uploadpack", &gate_path]);

    let ws_dir = TempDir::new().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));

    let flipper = {
        let cancel = Arc::clone(&cancel);
        let hold = Arc::clone(&hold);
        std::thread::spawn(move || {
            // Comfortably shorter than the fetch's own limit below. If the
            // gate never starts, this fires first and says so, instead of
            // expiring together with the timeout it exists to test around and
            // leaving the failure ambiguous.
            hold.wait_holding(
                Duration::from_secs(20),
                "the gated fetch never started",
                || None,
            );
            // Strictly ordered: the flag is set before the fetch is released,
            // so checkpoint 1 cannot have seen it and checkpoint 2 must.
            cancel.store(true, Ordering::Relaxed);
            hold.release();
        })
    };

    // A strategy that reads `origin/*`, so the fetch this test gates on is
    // one the attempt actually starts; a detached HEAD skips it.
    let attempt = create_worktree_cancellable(
        repo_dir.path(),
        ws_dir.path(),
        "test-ws",
        &BranchStrategy::NewBranch("topic".to_string()),
        PreCreateFetch::Run(Duration::from_secs(60)),
        &cancel,
    );
    flipper.join().unwrap();

    // Everything below rests on the fetch having been held until the flag was
    // set. Checked from the upload-pack's side rather than by timing: it wrote
    // its receipt only after seeing the release. A fetch that hit its limit above
    // was killed with its upload-pack before it could write that, so a
    // missing record names the timeout instead of reading as a cancellation
    // regression.
    assert!(
        hold.saw_release(),
        "the gated upload-pack must hold until the release (no record means it \
         gave up, or the fetch hit its 60 s limit and was killed): {:?}",
        attempt.fetch
    );
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

/// Ticket 13. A name that is not one plain path component would resolve
/// outside `ws_dir` when joined; the core refuses it before any directory is
/// created, so the escape path never appears.
#[test]
fn create_worktree_refuses_a_name_that_leaves_ws_dir() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("alpha");

    let err = create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "../escape",
        &BranchStrategy::NewBranch("topic".to_string()),
    )
    .expect_err("a name containing '/' must be refused");
    let text = format!("{}", err);

    assert!(
        text.contains("invalid space name \"../escape\""),
        "the error must name the offending name, got {:?}",
        text
    );
    assert!(
        text.contains("Space name cannot contain '/' or '\\'"),
        "the error must name the rule that was broken, got {:?}",
        text
    );
    assert!(
        !env.dir.path().join("escape").exists(),
        "nothing may be created outside ws_dir"
    );
    assert!(
        std::fs::read_dir(&env.workspaces_dir)
            .unwrap()
            .next()
            .is_none(),
        "ws_dir must gain no entry either"
    );
}

/// Ticket 13. A branch name beginning with '-' would land in git's `-b` slot,
/// which takes the next argv verbatim and re-parses it as options in the
/// child `git branch`. The core refuses it before `create_dir_all`, so a
/// rejected call leaves no space directory behind.
#[test]
fn create_worktree_refuses_a_dash_branch_before_touching_disk() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("alpha");

    for strategy in [
        BranchStrategy::NewBranch("-x".to_string()),
        BranchStrategy::ExistingBranch("-x".to_string()),
        // git accepts origin/-x as a branch name; the stripped -x is what
        // reaches -b, so the guard must look at that.
        BranchStrategy::ExistingBranch("origin/-x".to_string()),
    ] {
        let err = create_worktree(&repo_path, &env.workspaces_dir, "dashed", &strategy)
            .expect_err("a branch beginning with '-' must be refused");
        let text = format!("{}", err);
        assert_eq!(
            text, "'-x' is not a valid branch name",
            "the refusal uses git's own sentence for {:?}",
            strategy
        );
        assert!(
            !env.workspaces_dir.join("dashed").exists(),
            "the space directory must not exist after a refusal for {:?}",
            strategy
        );
    }
}

/// Ticket 13, finding beyond the ticket: `remove_workspace` joined the name
/// unchecked, so `..` named the parent of `ws_dir`, and `.` or an empty name
/// named `ws_dir` itself, each then handed to `remove_dir_all`. Positive
/// evidence: a real space and `ws_dir` both survive every refused call.
#[test]
fn remove_workspace_refuses_dot_dot_and_dot() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("alpha");
    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "keep",
        &BranchStrategy::NewBranch("keep".to_string()),
    )
    .unwrap();
    let kept = env.workspaces_dir.join("keep").join("alpha");
    assert!(kept.join(".git").exists(), "fixture: the space exists");

    for (name, rule) in [
        ("..", "Space name cannot be '.' or '..'"),
        (".", "Space name cannot be '.' or '..'"),
        ("", "Space name cannot be empty"),
    ] {
        let err = space::core::workspace::remove_workspace(&env.workspaces_dir, name, true)
            .expect_err("a name that is not one plain component must be refused");
        let text = format!("{}", err);
        assert!(
            text.contains(rule),
            "{:?} must be refused by its rule, got {:?}",
            name,
            text
        );
        assert!(
            kept.join(".git").exists(),
            "the real space must survive a refused remove of {:?}",
            name
        );
        assert!(
            env.workspaces_dir.exists() && env.repos_dir.exists(),
            "ws_dir and its parent's other children must survive a refused remove of {:?}",
            name
        );
    }
}

/// Ticket 13, finding beyond the ticket: `workspace_detail` listed whatever
/// directory the joined name resolved to and ran git status inside its repo
/// subdirectories, so over MCP `../repos` disclosed the repo roots.
#[test]
fn workspace_detail_refuses_a_slash_name() {
    let env = common::TestEnv::new();
    env.create_repo("alpha");

    let err = space::core::workspace::workspace_detail(&env.workspaces_dir, "../repos")
        .expect_err("a name containing '/' must be refused");
    let text = format!("{}", err);
    assert!(
        text.contains("Space name cannot contain '/' or '\\'"),
        "got {:?}",
        text
    );
}

/// Ticket 13, found in review. `ExistingBranch("origin/-M")` passes git's
/// own check as a whole name, and `add_worktree` strips the prefix so `-M`
/// reaches `-b`, where git's child `git branch -M origin/-M` force-renames
/// the checked-out branch of the SOURCE repo (reproduced on git 2.50.1 with
/// a local branch named `origin/-M` present, as the `new` strategy can
/// create). Positive evidence: the source repo's HEAD still names `main`.
#[test]
fn an_origin_prefixed_dash_branch_cannot_rename_the_checked_out_branch() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("victim");
    let out = Command::new("git")
        .args(["branch", "origin/-M", "main"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture: a local branch named origin/-M"
    );

    let err = create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "stage2",
        &BranchStrategy::ExistingBranch("origin/-M".to_string()),
    )
    .expect_err("the derived -b name begins with '-' and must be refused");
    assert_eq!(format!("{}", err), "'-M' is not a valid branch name");

    let head = Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&head.stdout).trim(),
        "refs/heads/main",
        "the source repo's checked-out branch keeps its name"
    );
    assert!(
        !env.workspaces_dir.join("stage2").exists(),
        "nothing is created for a refused branch"
    );
}

/// Ticket 13, coverage found in review. When both the name and the branch
/// are invalid, the name guard answers, because a rejected name has no
/// space to put a branch in. Either order refuses before any write; this
/// pins which sentence the caller sees.
#[test]
fn an_invalid_name_is_reported_before_an_invalid_branch() {
    let env = common::TestEnv::new();
    let repo_path = env.create_repo("alpha");
    let err = create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "../escape",
        &BranchStrategy::NewBranch("-x".to_string()),
    )
    .expect_err("both are invalid");
    assert!(
        format!("{}", err).starts_with("invalid space name"),
        "the name guard runs first, got {:?}",
        format!("{}", err)
    );
}

// ---------------------------------------------------------------------------
// Ticket 27: a failed `git worktree remove` must not be followed by deleting
// the space directory.
// ---------------------------------------------------------------------------

/// Run git in `dir` and return its stdout, asserting it worked.
fn git_ok(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The worktrees `repo` still has registered, as `git worktree list` sees them.
fn registered_worktrees(repo: &Path) -> String {
    git_ok(repo, &["worktree", "list", "--porcelain"])
}

/// Add one worktree of `repo` to space `ws_name`, on a branch of that name.
fn worktree_in_space(env: &TestEnv, repo: &Path, ws_name: &str) -> PathBuf {
    create_worktree(
        repo,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch(ws_name.to_string()),
    )
    .unwrap()
}

/// A locked worktree makes `git worktree remove --force` refuse (exit 128 on
/// git 2.50.1). Deleting the directory anyway leaves the source repo with a
/// `locked` admin entry that `git worktree prune` deliberately keeps and a
/// branch that `git branch -D` then refuses to delete. The space is kept
/// instead; the repos that could be removed still are, which is what makes
/// "the run continued past the failure" observable.
#[test]
fn remove_workspace_keeps_a_locked_worktree_and_removes_the_others() {
    let env = common::TestEnv::new();
    // Entries are visited in name order, so the locked one is seen first.
    let locked_repo = env.create_repo("a-locked");
    let free_repo = env.create_repo("b-free");
    let locked_wt = worktree_in_space(&env, &locked_repo, "test-ws");
    let free_wt = worktree_in_space(&env, &free_repo, "test-ws");
    git_ok(
        &locked_repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "on usb",
            locked_wt.to_str().unwrap(),
        ],
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "test-ws", true)
        .expect_err("a refused worktree removal must be reported, not swallowed");
    let text = err.to_string();

    assert!(
        text.contains("a-locked") && text.contains("locked working tree"),
        "the report names the repo and git's own reason, got {:?}",
        text
    );
    assert!(
        text.contains("git worktree unlock"),
        "the report names the way out, got {:?}",
        text
    );
    assert!(
        !text.contains("remove -f -f"),
        "and not git's own advice for a flag space does not offer, got {:?}",
        text
    );
    assert!(
        text.lines().next().unwrap().contains("a-locked"),
        "the first line stands alone as a summary (it is all the TUI shows), got {:?}",
        text
    );
    assert!(
        locked_wt.join(".git").exists() && env.workspaces_dir.join("test-ws").exists(),
        "the space and the worktree git refused to remove must both survive"
    );
    let still = registered_worktrees(&locked_repo);
    assert!(
        still.contains("test-ws"),
        "the source repo must still own the worktree it refused to give up, got {}",
        still
    );

    assert!(
        !free_wt.exists(),
        "the run must continue past the failure and remove the other repo"
    );
    let freed = registered_worktrees(&free_repo);
    assert!(
        !freed.contains("test-ws"),
        "the repo that was removed must be unregistered too, got {}",
        freed
    );
    assert!(
        text.contains("b-free"),
        "the report says what it did remove, got {:?}",
        text
    );
}

/// The admin directory a worktree's `.git` file names, resolved against
/// the worktree when git wrote it relative (`worktree.useRelativePaths`,
/// which a developer's global config may set).
fn admin_dir_of(wt: &Path) -> PathBuf {
    let content = std::fs::read_to_string(wt.join(".git")).unwrap();
    let target = content
        .strip_prefix("gitdir: ")
        .unwrap()
        .trim_end_matches(['\n', '\r']);
    if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        wt.join(target)
    }
}

/// Ticket 20. The one lock `space` does override: the lock git itself took
/// for a `git worktree add` whose checkout was killed, read from the
/// checkout's `index.lock` still there and no `index`. Such a tree holds
/// nothing of the user's and `--force` has already consented to losing
/// whatever it holds, so a forced removal runs `git worktree remove --force
/// --force` for that worktree alone: the space a user lost power creating
/// is removed and can be created again with no git command. The content of
/// the lock decides nothing: a finished tree under git's own `initializing`
/// word (a killed add whose checkout child finished, or a user who typed
/// the word), even with a stale `index.lock` beside its surviving index, is
/// kept with ticket 27's unlock hint, never escalated, and so is a user's
/// own lock.
#[test]
fn remove_workspace_removes_a_half_built_worktree_and_keeps_every_finished_lock() {
    let env = common::TestEnv::new();
    let marker_repo = env.create_repo("a-marker");
    let index_repo = env.create_repo("b-no-index");
    let user_repo = env.create_repo("c-user-lock");
    let marker_wt = worktree_in_space(&env, &marker_repo, "test-ws");
    let index_wt = worktree_in_space(&env, &index_repo, "test-ws");
    let user_wt = worktree_in_space(&env, &user_repo, "test-ws");

    let marker_admin = admin_dir_of(&marker_wt);
    std::fs::write(marker_admin.join("locked"), "initializing\n").unwrap();
    assert!(
        marker_admin.join("index").is_file(),
        "fixture: a finished tree"
    );
    // And a stale index.lock beside the surviving index (a killed index
    // writer): still a finished tree, still never escalated.
    std::fs::write(marker_admin.join("index.lock"), "").unwrap();
    std::fs::write(marker_wt.join("WIP"), "mine\n").unwrap();
    let index_admin = admin_dir_of(&index_wt);
    std::fs::write(index_admin.join("locked"), "initializing\n").unwrap();
    std::fs::remove_file(index_admin.join("index")).unwrap();
    std::fs::write(index_admin.join("index.lock"), "").unwrap();
    git_ok(
        &user_repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "on usb",
            user_wt.to_str().unwrap(),
        ],
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "test-ws", true)
        .expect_err("two finished locks are still refused");
    let text = err.to_string();
    let kept_part = text.split("removed:").next().unwrap();
    for name in ["a-marker", "c-user-lock"] {
        assert!(kept_part.contains(name), "{} is kept, got {:?}", name, text);
    }
    assert_eq!(
        text.matches("git worktree unlock").count(),
        2,
        "both finished locks get ticket 27's hint, got {:?}",
        text
    );
    assert!(
        !kept_part.contains("b-no-index"),
        "the half-built worktree is not reported as kept, got {:?}",
        text
    );
    assert!(
        text.contains("removed: \"b-no-index\""),
        "it is reported as removed, got {:?}",
        text
    );
    assert!(
        !index_wt.exists() && !registered_worktrees(&index_repo).contains("test-ws"),
        "the half-built worktree is gone from the space and unregistered"
    );
    assert_eq!(
        std::fs::read_to_string(marker_wt.join("WIP")).unwrap(),
        "mine\n",
        "the finished tree under git's own word survives with its work"
    );
    assert!(
        registered_worktrees(&marker_repo).contains("test-ws")
            && user_wt.join(".git").exists()
            && registered_worktrees(&user_repo).contains("test-ws"),
        "both finished locks survive, still registered"
    );
}

/// `git worktree add` writes a relative `gitdir:` when the user sets
/// `worktree.useRelativePaths` (git 2.48 and later). The old code resolved
/// that against the process's own working directory, found no repo, and ran
/// no git at all while still deleting the directory, so the source repo kept
/// a prunable entry. The `.git` file is written by hand here, byte for byte
/// what git writes for that setting, so the test does not depend on the git
/// version the suite runs against.
#[test]
fn remove_workspace_unregisters_a_worktree_whose_gitdir_is_relative() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "rel-ws");

    let admin = repo.join(".git").join("worktrees").join("alpha");
    assert!(
        admin.is_dir(),
        "fixture: the admin directory is where git puts it"
    );
    // <ws_dir>/rel-ws/alpha -> <repos_dir>/alpha/.git/worktrees/alpha
    std::fs::write(
        wt.join(".git"),
        "gitdir: ../../../repos/alpha/.git/worktrees/alpha\n",
    )
    .unwrap();
    let branch = space::core::git::current_branch(&wt).unwrap();
    assert_eq!(branch, "rel-ws", "fixture: git still reads the worktree");

    space::core::workspace::remove_workspace(&env.workspaces_dir, "rel-ws", true).unwrap();

    assert!(!env.workspaces_dir.join("rel-ws").exists());
    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("rel-ws"),
        "git must have run and unregistered the worktree, got {}",
        left
    );
}

/// The opposite guard: when the source repo is gone, git has nothing to
/// unregister, so the space stays removable rather than becoming stuck.
#[test]
fn remove_workspace_deletes_a_space_whose_source_repo_is_gone() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "orphan-ws");
    std::fs::remove_dir_all(&repo).unwrap();
    assert!(
        wt.join(".git").is_file(),
        "fixture: the gitfile outlives its repo"
    );

    space::core::workspace::remove_workspace(&env.workspaces_dir, "orphan-ws", true).unwrap();

    assert!(
        !env.workspaces_dir.join("orphan-ws").exists(),
        "a space whose source repo is gone must still be removable"
    );
}

/// Without `--force` git refuses a worktree with uncommitted work. Deleting
/// the directory anyway destroyed exactly the work git was protecting. No
/// production caller passes `force: false`; the library API does.
#[test]
fn remove_workspace_keeps_a_dirty_worktree_when_not_forced() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "dirty-ws");
    std::fs::write(wt.join("notes.txt"), "wip").unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "dirty-ws", false)
        .expect_err("git refuses a dirty worktree without --force");
    assert!(
        err.to_string().contains("alpha"),
        "the report names the repo, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("notes.txt")).unwrap(),
        "wip",
        "the uncommitted work git protected must still be on disk"
    );
}

/// The headline case: a directory in a space that holds its own repository
/// (a `git clone` dropped there by hand) is not a worktree of anything, so
/// git never ran for it and `remove_dir_all` took the whole repository,
/// unpushed commits and all. It is kept and reported now.
#[test]
fn remove_workspace_keeps_a_clone_that_is_not_a_worktree() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &repo, "mixed-ws");

    let clone = env.workspaces_dir.join("mixed-ws").join("z-clone");
    git_ok(
        &env.workspaces_dir,
        &[
            "clone",
            "--quiet",
            repo.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    for (key, value) in [
        ("user.email", "space@local"),
        ("user.name", "Test"),
        ("commit.gpgsign", "false"),
    ] {
        git_ok(&clone, &["config", key, value]);
    }
    git_ok(&clone, &["commit", "--allow-empty", "-m", "unpushed"]);
    assert!(
        clone.join(".git").is_dir(),
        "fixture: a clone, not a worktree"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "mixed-ws", true)
        .expect_err("a repository space did not create must not be deleted");
    assert!(
        err.to_string().contains("z-clone") && err.to_string().contains("move it aside"),
        "the report names the directory it kept and what to do, got {:?}",
        err.to_string()
    );

    assert_eq!(
        git_ok(&clone, &["log", "-1", "--format=%s"]).trim(),
        "unpushed",
        "the clone's own commit must still be there afterwards"
    );
    assert!(
        !worktree.exists(),
        "the real worktree beside it is still removed"
    );
}

/// A repo whose directory name begins with `-` is removed like any other.
/// What reaches git is the canonicalised path, so the dash sits inside it and
/// the `--` separator in the call is belt and braces, not what this test
/// proves: dropping the separator keeps this test green. It proves the rest
/// of the path, from classification to the source repo losing the
/// registration, on a name that any argv handling is most likely to mangle.
#[test]
fn remove_workspace_removes_a_repo_whose_name_begins_with_a_dash() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("-dash");
    let wt = worktree_in_space(&env, &repo, "dash-ws");
    assert!(wt.join(".git").is_file(), "fixture: the worktree is there");

    space::core::workspace::remove_workspace(&env.workspaces_dir, "dash-ws", true).unwrap();

    assert!(!env.workspaces_dir.join("dash-ws").exists());
    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("dash-ws"),
        "git must have unregistered it, got {}",
        left
    );
}

/// Review of the first commit: a bare repo has no `.git` entry at all, so the
/// scan never saw it and `remove_dir_all` took it with the space, history and
/// all. The clone case and this one are the same promise.
#[test]
fn remove_workspace_keeps_a_bare_repo_in_the_space() {
    let env = common::TestEnv::new();
    let source = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &source, "bare-ws");
    let bare = env.workspaces_dir.join("bare-ws").join("z-bare.git");
    git_ok(
        &env.workspaces_dir,
        &[
            "clone",
            "--quiet",
            "--bare",
            source.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    assert!(
        !bare.join(".git").exists() && bare.join("HEAD").is_file(),
        "fixture: a bare repo keeps its files at the top level"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "bare-ws", true)
        .expect_err("a bare repository must not be deleted with the space");
    assert!(
        err.to_string().contains("z-bare.git"),
        "the report names it, got {:?}",
        err.to_string()
    );
    assert_eq!(
        git_ok(&bare, &["rev-list", "--count", "HEAD"]).trim(),
        "1",
        "the bare repository still answers for its own history"
    );
    assert!(!worktree.exists(), "the real worktree beside it still goes");
}

/// Review of the first commit: a submodule checkout's `.git` file points at
/// `<host>/.git/modules/<name>`, which exists, so reading the gitfile alone
/// called it a worktree and left git to refuse it with `is not a working
/// tree`. the layout reader (`commondir`) is what `placement_of` uses for this exact
/// trap, and it gives the honest reason instead.
#[test]
fn remove_workspace_keeps_a_submodule_checkout() {
    let env = common::TestEnv::new();
    let inner = env.create_repo("inner");
    let host = env.create_repo("host");
    git_ok(
        &host,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "--quiet",
            "add",
            inner.to_str().unwrap(),
            "sub",
        ],
    );
    git_ok(&host, &["commit", "--quiet", "-m", "add submodule"]);

    // The checkout is moved into the space, which is the only way one lands
    // beside the worktrees: nothing in the app puts it there.
    let space_dir = env.workspaces_dir.join("sub-ws");
    std::fs::create_dir_all(&space_dir).unwrap();
    let sub = space_dir.join("z-sub");
    std::fs::rename(host.join("sub"), &sub).unwrap();
    // git wrote that gitfile relative to where the checkout was; moving it
    // would leave the target dangling, which is the orphan case, not this
    // one. Point it at the module directory it came from, so what is being
    // tested is a gitfile whose target exists and is not a worktree.
    let modules = host.join(".git").join("modules").join("sub");
    assert!(
        modules.is_dir(),
        "fixture: the submodule's git dir is there"
    );
    std::fs::write(sub.join(".git"), format!("gitdir: {}\n", modules.display())).unwrap();
    // And point the module back at where the checkout now is, which is what
    // makes this a submodule that still works rather than a broken one.
    git_ok(
        &env.workspaces_dir,
        &[
            "config",
            "--file",
            modules.join("config").to_str().unwrap(),
            "core.worktree",
            sub.to_str().unwrap(),
        ],
    );
    assert_eq!(
        git_ok(&sub, &["rev-parse", "--is-inside-work-tree"]).trim(),
        "true",
        "fixture: git can work in the submodule checkout where it now is"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "sub-ws", true)
        .expect_err("a submodule checkout is a repository, not a worktree");
    let text = err.to_string();
    assert!(
        text.contains("z-sub") && text.contains("repository of its own"),
        "the reason says what it is, rather than passing on git's `is not a working tree`, got {:?}",
        text
    );
    assert!(sub.join(".git").exists(), "and it is still there");
}

/// Review of the first commit: `.git` unreadable or malformed used to mean
/// "orphan", and an orphan is deleted. Not knowing what a directory is, is
/// not a reason to destroy it.
#[test]
fn remove_workspace_keeps_a_directory_whose_gitfile_cannot_be_read() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &repo, "odd-ws");
    let odd = env.workspaces_dir.join("odd-ws").join("z-odd");
    std::fs::create_dir_all(&odd).unwrap();
    std::fs::write(odd.join(".git"), "this is not a gitfile\n").unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "odd-ws", true)
        .expect_err("a directory that cannot be read must not be deleted");
    assert!(
        err.to_string().contains("z-odd") && err.to_string().contains("delete it by hand"),
        "the report names it and what to do, got {:?}",
        err.to_string()
    );
    assert!(odd.join(".git").exists(), "and it is still there");
    assert!(!worktree.exists(), "the real worktree beside it still goes");
}

/// Review of the first commit: an orphaned worktree is deleted without git
/// ever running, so `force: false` destroyed uncommitted work in the one
/// case where git is not there to object.
#[test]
fn remove_workspace_keeps_an_orphan_when_not_forced() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "orphan-ws");
    std::fs::write(wt.join("notes.txt"), "wip").unwrap();
    std::fs::remove_dir_all(&repo).unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "orphan-ws", false)
        .expect_err("without force nothing is destroyed unchecked");
    assert!(
        err.to_string().contains("alpha")
            && err.to_string().contains("remove the space with force"),
        "the report names it and what to do, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("notes.txt")).unwrap(),
        "wip",
        "the uncommitted work is still on disk"
    );

    // With force it goes, and is accounted for rather than passed over.
    let removed = space::core::workspace::remove_workspace(&env.workspaces_dir, "orphan-ws", true);
    removed.unwrap();
    assert!(!env.workspaces_dir.join("orphan-ws").exists());
}

/// Review of the first commit: the summary line repeated the only body line
/// word for word when one repo was kept, git's second stderr line landed at
/// column zero as though it were a report entry of its own, and the counts
/// left orphans out of a total that included them.
#[test]
fn remove_workspace_report_reads_as_a_report() {
    let env = common::TestEnv::new();
    let locked_repo = env.create_repo("a-locked");
    let free_repo = env.create_repo("b-free");
    let orphan_repo = env.create_repo("c-orphan");
    let locked_wt = worktree_in_space(&env, &locked_repo, "ws");
    worktree_in_space(&env, &free_repo, "ws");
    worktree_in_space(&env, &orphan_repo, "ws");
    git_ok(
        &locked_repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "on usb",
            locked_wt.to_str().unwrap(),
        ],
    );
    std::fs::remove_dir_all(&orphan_repo).unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "ws", true)
        .expect_err("the locked worktree is refused");
    let text = err.to_string();
    let lines: Vec<&str> = text.lines().collect();

    assert!(
        lines[0].contains("a-locked") && lines[0].contains("ws"),
        "the summary leads with what to act on, since a status row is clipped \
         at 80 columns, got {:?}",
        lines[0]
    );
    assert!(
        lines[1..]
            .iter()
            .any(|l| l.contains("1 of 3 repos in the space were kept")),
        "and the count covers every repo the space held: {:?}",
        text
    );
    assert!(
        lines[1..].iter().any(|l| l.contains("c-orphan")),
        "including the one with no source repo left, which is not silently dropped: {:?}",
        text
    );
    assert!(
        lines[1..].iter().all(|l| l.starts_with("  ")),
        "every line under the summary is indented, so git's second line cannot read as an entry of its own: {:?}",
        text
    );
    assert!(
        lines[1..].iter().all(|l| l.trim() != lines[0].trim()),
        "no line repeats the summary word for word: {:?}",
        text
    );
    assert!(
        text.contains("b-free"),
        "and what was removed is named: {:?}",
        text
    );
}

/// Review of the first commit: nothing pinned `--force` itself. The whole
/// suite stayed green with the flag dropped, and after this ticket that
/// regression is no longer invisible: it would make every space holding a
/// modified file unremovable. This is the complement of the unforced test.
#[test]
fn remove_workspace_removes_a_dirty_worktree_when_forced() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "forced-ws");
    std::fs::write(wt.join("notes.txt"), "wip").unwrap();
    std::fs::write(wt.join("tracked.txt"), "changed").unwrap();

    space::core::workspace::remove_workspace(&env.workspaces_dir, "forced-ws", true).unwrap();

    assert!(
        !env.workspaces_dir.join("forced-ws").exists(),
        "force is what lets a space with uncommitted work go"
    );
    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("forced-ws"),
        "and the source repo is told about it, got {}",
        left
    );
}

/// A repository git2 cannot open is still a repository. libgit2 refuses a
/// format extension it does not know (`git init --ref-format=reftable`
/// writes `extensions.refstorage`, git 2.45 and later), and asking it
/// "is this a repository" made the answer "no", which is the answer that
/// deletes. Every repository has an `objects` directory and a `config` file,
/// whatever its format, and that is what decides now.
#[test]
fn remove_workspace_keeps_a_repo_no_library_can_open() {
    let env = common::TestEnv::new();
    let source = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &source, "odd-format-ws");
    let odd = env
        .workspaces_dir
        .join("odd-format-ws")
        .join("z-odd-format");

    let bare = env
        .workspaces_dir
        .join("odd-format-ws")
        .join("z-odd-bare.git");

    // Real repositories in a format this build's libgit2 does not support,
    // in both shapes: one with a `.git` directory and one bare. Reftable
    // when the local git can make one (git 2.45 and later), an unknown
    // extension named in the config when it cannot; git2 refuses either.
    for (path, args) in [
        (&odd, vec!["init", "--quiet", "--ref-format=reftable"]),
        (
            &bare,
            vec!["init", "--quiet", "--bare", "--ref-format=reftable"],
        ),
    ] {
        let reftable = Command::new("git").args(&args).arg(path).output().unwrap();
        if !reftable.status.success() {
            let plain: Vec<&str> = args
                .iter()
                .filter(|a| *a != &"--ref-format=reftable")
                .copied()
                .collect();
            Command::new("git").args(&plain).arg(path).output().unwrap();
            git_ok(path, &["config", "core.repositoryformatversion", "1"]);
            git_ok(path, &["config", "extensions.spaceUnknown", "true"]);
        }
        assert!(
            git2::Repository::open(path).is_err(),
            "fixture: git2 must refuse {}, or the test proves nothing",
            path.display()
        );
    }
    std::fs::write(odd.join("unpushed-marker.txt"), "work").unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "odd-format-ws", true)
        .expect_err("a repository must not be deleted because a library cannot read it");
    let text = err.to_string();
    assert!(
        text.contains("z-odd-format") && text.contains("z-odd-bare.git"),
        "the report names both, got {:?}",
        text
    );
    assert!(
        odd.join(".git").is_dir() && odd.join("unpushed-marker.txt").exists(),
        "the repository with a .git directory and its contents are still on disk"
    );
    assert!(
        bare.join("objects").is_dir(),
        "and so is the bare one, objects and all"
    );
    assert!(!worktree.exists(), "the real worktree beside it still goes");
}

/// The source repo of a worktree made under `worktree.useRelativePaths`
/// carries `extensions.relativeWorktrees` in its config (git 2.48 and
/// later), which libgit2 also refuses. Asking it to tell a worktree from a
/// submodule checkout therefore made every space of such a repo permanently
/// unremovable. The hand-written fixture in the test above this one wrote
/// the relative gitfile without that config, so it never saw this.
#[test]
fn remove_workspace_removes_a_worktree_whose_source_uses_relative_paths() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    git_ok(&repo, &["config", "worktree.useRelativePaths", "true"]);
    let wt = env.workspaces_dir.join("relcfg-ws").join("alpha");
    git_ok(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "relcfg-ws",
            wt.to_str().unwrap(),
        ],
    );
    assert!(
        wt.join(".git").is_file(),
        "fixture: the worktree is where git put it"
    );

    space::core::workspace::remove_workspace(&env.workspaces_dir, "relcfg-ws", true).unwrap();

    assert!(!env.workspaces_dir.join("relcfg-ws").exists());
    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("relcfg-ws"),
        "git must have unregistered it, got {}",
        left
    );
}

/// The report quotes directory names with `{:?}`, which is a terminal-safety
/// claim: a repo directory name is not the space name and nothing validates
/// it, so it can hold a newline or an escape sequence. Neither may break the
/// summary into two lines or reach a terminal as control characters.
#[test]
fn remove_workspace_report_neutralises_a_hostile_directory_name() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    worktree_in_space(&env, &repo, "hostile-ws");
    // A directory that is a repository of its own, so it is reported, named
    // with a newline and an escape sequence.
    let hostile = env
        .workspaces_dir
        .join("hostile-ws")
        .join("z-\u{1b}[2Jwiped\nsecond line");
    std::fs::create_dir_all(&hostile).unwrap();
    git_ok(&hostile, &["init", "--quiet"]);
    // And one just as hostile that is removed, so the `removed:` list is
    // held to the same rule as the kept names.
    let removed_repo = env.create_repo("b-\u{1b}[2Jgone\nalso second");
    worktree_in_space(&env, &removed_repo, "hostile-ws");

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "hostile-ws", true)
        .expect_err("the repository of its own is kept");
    let text = err.to_string();
    let summary = text.lines().next().unwrap();

    assert!(
        !summary.contains('\u{1b}'),
        "no escape byte reaches a terminal through the summary, got {:?}",
        summary
    );
    assert!(
        summary.contains("wiped") && summary.contains("second line"),
        "the name is still legible, escaped, got {:?}",
        summary
    );
    assert!(
        !text.contains("\n\u{1b}") && text.lines().skip(1).all(|l| l.starts_with("  ")),
        "and it cannot open a line of its own in the body, got {:?}",
        text
    );
    let removed_line = text
        .lines()
        .find(|l| l.trim_start().starts_with("removed:"))
        .expect("the hostile-named worktree was removed and is listed");
    assert!(
        !removed_line.contains('\u{1b}') && removed_line.contains("gone"),
        "the removed list is escaped the same way, got {:?}",
        removed_line
    );
}

/// The summary names what was kept before it quotes a reason, so that
/// nothing in git's sentence (which ends with the user's own lock reason)
/// can read as part of the count or the list.
#[test]
fn remove_workspace_summary_puts_the_count_before_the_reason() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-locked");
    let wt = worktree_in_space(&env, &repo, "order-ws");
    git_ok(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "on usb",
            wt.to_str().unwrap(),
        ],
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "order-ws", true)
        .expect_err("the locked worktree is refused");
    let summary = err.to_string().lines().next().unwrap().to_string();

    let name_at = summary
        .find("a-locked")
        .expect("the summary names the repo");
    let reason_at = summary
        .find("cannot remove a locked working tree")
        .expect("and carries git's reason");
    assert!(
        name_at < reason_at,
        "the repo and count come first, git's sentence last, got {:?}",
        summary
    );
    assert!(
        summary.contains("first reason:"),
        "with git's sentence introduced, so it cannot read as part of the list, got {:?}",
        summary
    );
}

/// The scan sorts the directories, so the report and the order git is
/// reached in do not inherit `read_dir`'s, which is not defined. Bare repos
/// are the cheapest thing that is always kept, and they need no git run.
#[test]
fn remove_workspace_reports_in_name_order_whatever_read_dir_says() {
    let env = common::TestEnv::new();
    let space_dir = env.workspaces_dir.join("order-ws");
    std::fs::create_dir_all(&space_dir).unwrap();
    // Created back to front, so creation order is not name order.
    for name in ["z-last", "m-mid", "a-first"] {
        git_ok(
            &space_dir,
            &[
                "init",
                "--quiet",
                "--bare",
                space_dir.join(name).to_str().unwrap(),
            ],
        );
    }

    let on_disk: Vec<String> = std::fs::read_dir(&space_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let mut sorted = on_disk.clone();
    sorted.sort();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "order-ws", true)
        .expect_err("three repositories of their own are kept");
    let text = err.to_string();
    // The summary lists at most two names, so the order is read from the
    // body, which carries every one of them.
    let body = text.lines().skip(1).collect::<Vec<_>>().join("\n");
    let at = |name: &str| body.find(name).expect("every repo is named in the body");

    assert!(
        at("a-first") < at("m-mid") && at("m-mid") < at("z-last"),
        "the report is in name order, got {:?}",
        text
    );
    // If this filesystem hands them back sorted already, the assertion above
    // holds either way and this test proves nothing; say so rather than
    // quietly passing.
    assert_ne!(
        on_disk, sorted,
        "fixture: read_dir returned name order by itself, so this test cannot \
         see the sort (it is not a failure of the code)"
    );
}

/// A worktree copied beside its original in the same space shares its `.git`
/// file, so both name one admin directory. Removing the original destroys
/// that admin directory, after which the copy has nothing registered and git
/// can no longer read it (before ticket 36 a retry then deleted it as an
/// orphan, every such copy at once, with a clean success). So neither is
/// handed to git: the pair is kept and reported together, and the space is
/// left as it was found.
#[test]
fn remove_workspace_keeps_a_worktree_and_its_copy_across_retries() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let original = worktree_in_space(&env, &repo, "copy-ws");
    let copy = env.workspaces_dir.join("copy-ws").join("z-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();
    std::fs::write(copy.join("experiment.txt"), "a day of work").unwrap();

    for run in 1..=2 {
        let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "copy-ws", true)
            .expect_err("a worktree and its copy are kept together");
        let text = err.to_string();
        assert!(
            text.contains("a-repo") && text.contains("z-copy"),
            "run {}: the report names both halves of the pair, got {:?}",
            run,
            text
        );
        assert!(
            original.join(".git").exists(),
            "run {}: the original is kept, so the copy's admin directory survives",
            run
        );
        assert_eq!(
            std::fs::read_to_string(copy.join("experiment.txt")).unwrap(),
            "a day of work",
            "run {}: and the work in the copy is still there",
            run
        );
    }
    let still = registered_worktrees(&repo);
    assert!(
        still.contains("copy-ws"),
        "the original is still registered, so nothing was unregistered under it: {}",
        still
    );

    // Each half is told the truth about itself: the original that it has a
    // copy, the copy that it is one. Swapping them would tell a user to
    // delete the original by hand.
    let text = space::core::workspace::remove_workspace(&env.workspaces_dir, "copy-ws", true)
        .expect_err("kept")
        .to_string();
    let entry = |name: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(&format!("{:?}:", name)))
            .unwrap_or_else(|| panic!("an entry for {}, in {:?}", name, text))
            .to_string()
    };
    assert!(
        entry("a-repo").contains("a copy of it") && !entry("a-repo").contains("it is a copy"),
        "the original is told it has a copy, got {:?}",
        entry("a-repo")
    );
    assert!(
        entry("z-copy").contains("it is a copy of \"a-repo\""),
        "the copy is told it is one, and of which, got {:?}",
        entry("z-copy")
    );
}

/// The count of a kept pair leads the summary. It was placed after the
/// names at first, and with ordinary names ("frontend-service",
/// "frontend-service-copy") that put it at column 79 of an 80-column status
/// row, behind the `Delete failed: ` prefix. Leading, no name can push it off.
#[test]
fn remove_workspace_summary_leads_with_a_kept_pair_count() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let original = worktree_in_space(&env, &repo, "pair-ws");
    let copy = env.workspaces_dir.join("pair-ws").join("z-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();

    let summary = space::core::workspace::remove_workspace(&env.workspaces_dir, "pair-ws", true)
        .expect_err("the pair is kept")
        .to_string()
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(
        summary.starts_with("2 share one worktree; "),
        "the count comes first, got {:?}",
        summary
    );
    let names_at = summary.find("a-repo").expect("then the names");
    let reason_at = summary.find("first reason:").expect("then the reason");
    assert!(
        names_at < reason_at,
        "names before the reason, got {:?}",
        summary
    );
}

/// The same arm, reached by a permission error rather than by absence: the
/// source repo is intact and simply cannot be read right now. (An unmounted
/// volume does not look like this: its missing mount point is a NotFound,
/// which is an orphan. That is recorded as a residual of ticket 27, since
/// nothing on disk tells an unplugged drive from a deleted repo.)
#[test]
fn remove_workspace_keeps_a_worktree_whose_admin_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;

    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "locked-out-ws");
    std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();

    let worktrees = repo.join(".git").join("worktrees");
    let restore = std::fs::metadata(&worktrees).unwrap().permissions();
    std::fs::set_permissions(&worktrees, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::symlink_metadata(worktrees.join("alpha")).is_ok() {
        // Running as a user the mode does not apply to (root), so there is
        // nothing to test here. Say so rather than passing quietly.
        std::fs::set_permissions(&worktrees, restore).unwrap();
        eprintln!("skipped: this user can read a directory with mode 000");
        return;
    }

    let outcome =
        space::core::workspace::remove_workspace(&env.workspaces_dir, "locked-out-ws", true);
    std::fs::set_permissions(&worktrees, restore).unwrap();

    let err = outcome.expect_err("a directory that cannot be read is not deleted");
    assert!(
        err.to_string().contains("alpha"),
        "the report names it, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("uncommitted.txt")).unwrap(),
        "wip",
        "and the work in it is still there"
    );
    let still = registered_worktrees(&repo);
    assert!(
        still.contains("locked-out-ws"),
        "the registration it could not unregister is intact, got {}",
        still
    );
}

/// git's own rule for a `.git` file is that the gitdir is the rest of that
/// one line and a file carrying anything else is not a gitfile. Accepting
/// more than git does is not harmless: the extra text became part of the
/// path, the path did not exist, and a path that does not exist used to mean
/// "orphan", which deletes. Since ticket 36 such a path is kept anyway while
/// its source repo is there, so the near-miss check is pinned by its reason:
/// without it the report would say git has no record of a directory whose
/// admin directory is live.
#[test]
fn remove_workspace_keeps_a_gitfile_with_more_than_a_gitdir_line() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &repo, "extra-line-ws");
    let odd = env.workspaces_dir.join("extra-line-ws").join("z-odd");
    std::fs::create_dir_all(&odd).unwrap();
    let admin = repo.join(".git").join("worktrees").join("a-repo");
    std::fs::write(
        odd.join(".git"),
        format!("gitdir: {}\n# a note someone left\n", admin.display()),
    )
    .unwrap();
    std::fs::write(odd.join("keep.txt"), "work").unwrap();
    // git does not accept it either, which is the point: space must not be
    // laxer than git about a file that decides a deletion.
    let git_says = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(&odd)
        .output()
        .unwrap();
    assert!(
        !git_says.status.success(),
        "fixture: git rejects this gitfile, so space must not accept it"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "extra-line-ws", true)
        .expect_err("a gitfile space cannot read is not a licence to delete");
    assert!(
        err.to_string().contains("z-odd"),
        "the report names it, got {:?}",
        err.to_string()
    );
    assert!(
        err.to_string().contains("in a form git does not read"),
        "as a near miss of a live admin directory, got {:?}",
        err.to_string()
    );
    assert!(odd.join("keep.txt").exists(), "and the work in it is there");
    assert!(!worktree.exists(), "the real worktree beside it still goes");
}

/// A symlink in a space is left alone: `read_dir` reports the link, not what
/// it points at, so it is never classified and never handed to git. The
/// alternative, following it, would let a link inside a space aim
/// `git worktree remove` at a worktree outside it.
#[test]
fn remove_workspace_does_not_follow_a_symlinked_entry() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    // A worktree of the same repo, in a space of its own, linked into this one.
    let elsewhere = worktree_in_space(&env, &repo, "other-ws");
    let inside = worktree_in_space(&env, &repo, "link-ws");
    std::os::unix::fs::symlink(
        &elsewhere,
        env.workspaces_dir.join("link-ws").join("z-link"),
    )
    .unwrap();

    space::core::workspace::remove_workspace(&env.workspaces_dir, "link-ws", true).unwrap();

    assert!(!env.workspaces_dir.join("link-ws").exists());
    assert!(!inside.exists(), "the real worktree in the space went");
    assert!(
        elsewhere.join(".git").exists(),
        "what the link pointed at is untouched"
    );
    let still = registered_worktrees(&repo);
    assert!(
        still.contains("other-ws"),
        "and still registered, so git was never aimed through the link: {}",
        still
    );
}

/// A repository that has lost part of itself is still a repository. A bare
/// repo whose `objects` went is the case where the rest matters most.
#[test]
fn remove_workspace_keeps_a_damaged_bare_repo() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let worktree = worktree_in_space(&env, &repo, "damaged-ws");
    let bare = env.workspaces_dir.join("damaged-ws").join("z-damaged.git");
    git_ok(
        &env.workspaces_dir,
        &["init", "--quiet", "--bare", bare.to_str().unwrap()],
    );
    std::fs::remove_dir_all(bare.join("objects")).unwrap();
    assert!(
        bare.join("config").is_file() && bare.join("HEAD").is_file(),
        "fixture: what is left still says repository"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "damaged-ws", true)
        .expect_err("a damaged repository is still not ours to delete");
    assert!(
        err.to_string().contains("z-damaged.git"),
        "the report names it, got {:?}",
        err.to_string()
    );
    assert!(bare.join("config").is_file(), "and it is still there");
    assert!(!worktree.exists(), "the real worktree beside it still goes");
}

/// A space directory moved or renamed by hand is the refusal a user is most
/// likely to meet, and it was the only one arriving as git's raw sentence
/// with no way out. It names `git worktree repair` now, and only it: the
/// unlock hint belongs to a lock.
#[test]
fn remove_workspace_names_repair_for_a_space_that_was_moved() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    worktree_in_space(&env, &repo, "moved-ws");
    std::fs::rename(
        env.workspaces_dir.join("moved-ws"),
        env.workspaces_dir.join("renamed-ws"),
    )
    .unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "renamed-ws", true)
        .expect_err("git refuses a worktree at a path it does not know");
    let text = err.to_string();
    assert!(
        text.contains("git worktree repair"),
        "the report names the one command that fixes it, got {:?}",
        text
    );
    assert!(
        !text.contains("git worktree unlock"),
        "and not the one that does not apply, got {:?}",
        text
    );
    assert!(
        env.workspaces_dir.join("renamed-ws").exists(),
        "the space is kept"
    );
}

/// A gitfile git rejects because blanks follow a path that does not end in
/// one. git's reading names nothing, but the path trimmed names a live admin
/// directory, so it is a near miss: reported and kept, not read as an absent
/// repo. Reading only git's way, without the near-miss check, deleted it as
/// an orphan before ticket 36 and would now report it as a directory git has
/// no record of, which is false, so the reason is pinned; trimming instead of
/// reading git's way would lose the worktree whose real path ends in a blank
/// (the test after next).
#[test]
fn remove_workspace_keeps_a_worktree_whose_gitfile_has_trailing_blanks() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "blanks-ws");
    let admin = repo.join(".git").join("worktrees").join("alpha");
    std::fs::write(wt.join(".git"), format!("gitdir: {}   \n", admin.display())).unwrap();
    std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();
    let git_says = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(&wt)
        .output()
        .unwrap();
    assert!(
        !git_says.status.success(),
        "fixture: git itself rejects a gitfile with trailing blanks"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "blanks-ws", true)
        .expect_err("git refuses it, so it is kept, not treated as an orphan");
    assert!(
        err.to_string().contains("alpha"),
        "the report names it, got {:?}",
        err.to_string()
    );
    assert!(
        err.to_string().contains("in a form git does not read"),
        "as a near miss of a live admin directory, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("uncommitted.txt")).unwrap(),
        "wip",
        "and the work in it is still on disk"
    );
}

/// Coordinator review of 750aa5c: the admin directory itself unreadable,
/// rather than its parent. Stat-ing the admin directory needs only its
/// parent, so that succeeded, and then `commondir.is_file()` read the
/// permission error as "no commondir", which is how a submodule module
/// directory looks. The live worktree was reported as "a git repository of
/// its own", with advice to delete it by hand.
#[test]
fn remove_workspace_keeps_a_worktree_whose_admin_dir_itself_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;

    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "shut-ws");
    std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();

    let admin = repo.join(".git").join("worktrees").join("alpha");
    let restore = std::fs::metadata(&admin).unwrap().permissions();
    std::fs::set_permissions(&admin, std::fs::Permissions::from_mode(0o000)).unwrap();
    let parent_still_lets_us_see_it = std::fs::symlink_metadata(&admin).is_ok();
    let but_not_inside = std::fs::symlink_metadata(admin.join("commondir")).is_err();
    if !(parent_still_lets_us_see_it && but_not_inside) {
        std::fs::set_permissions(&admin, restore).unwrap();
        eprintln!("skipped: this user is not stopped by mode 000");
        return;
    }

    let outcome = space::core::workspace::remove_workspace(&env.workspaces_dir, "shut-ws", true);
    std::fs::set_permissions(&admin, restore).unwrap();

    let text = outcome
        .expect_err("a worktree that cannot be read is kept")
        .to_string();
    assert!(
        !text.contains("repository of its own"),
        "a live worktree must not be reported as a repository to delete by hand, got {:?}",
        text
    );
    assert!(
        text.contains("cannot be read"),
        "it is reported as unreadable, which is what it is, got {:?}",
        text
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("uncommitted.txt")).unwrap(),
        "wip",
        "and the work in it is still there"
    );
}

/// Skeptical pass 3, SR-15: a real admin path may end in whitespace, because
/// a repo directory name may. git made this worktree, of a repo whose name
/// ends in a non-breaking space, and git reads its `.git` file. Stripping all
/// trailing whitespace instead of only line endings read a path that does not
/// exist, which is an orphan, which was deleted without git ever running,
/// lock and all.
#[test]
fn remove_workspace_keeps_a_locked_worktree_whose_real_path_ends_in_a_blank() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("nb\u{a0}");
    let wt = worktree_in_space(&env, &repo, "nbsp-ws");
    std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();
    git_ok(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "keep me",
            wt.to_str().unwrap(),
        ],
    );
    assert_eq!(
        git_ok(&wt, &["rev-parse", "--is-inside-work-tree"]).trim(),
        "true",
        "fixture: git itself reads this worktree's .git file"
    );

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "nbsp-ws", true)
        .expect_err("a locked worktree is refused by git, not deleted as an orphan");
    assert!(
        err.to_string().contains("git worktree unlock"),
        "it was handed to git, which refused the lock, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(wt.join("uncommitted.txt")).unwrap(),
        "wip",
        "and the work in it is still there"
    );
}

/// The same path, unlocked: git removes it and the source repo forgets it,
/// which is what reading the path git's way buys.
#[test]
fn remove_workspace_unregisters_a_worktree_whose_real_path_ends_in_a_blank() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("nb\u{a0}");
    worktree_in_space(&env, &repo, "nbsp2-ws");

    space::core::workspace::remove_workspace(&env.workspaces_dir, "nbsp2-ws", true).unwrap();

    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("nbsp2-ws"),
        "git ran and unregistered it, got {}",
        left
    );
}

/// Near misses of git's rule are reported, never read as an absent repo.
/// Two spaces after the colon and a CRLF line followed by a comment are each
/// a file git rejects that plainly names a live admin directory.
#[test]
fn remove_workspace_keeps_near_miss_gitfiles_of_a_live_worktree() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let wt = worktree_in_space(&env, &repo, "near-ws");
    let admin = repo.join(".git").join("worktrees").join("alpha");
    std::fs::write(wt.join("uncommitted.txt"), "wip").unwrap();

    for content in [
        format!("gitdir:  {}\n", admin.display()),
        format!("gitdir: {}\r\n# a note\n", admin.display()),
    ] {
        std::fs::write(wt.join(".git"), &content).unwrap();
        let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "near-ws", true)
            .expect_err("a gitfile git rejects but that names a live admin is kept");
        assert!(
            err.to_string().contains("alpha"),
            "the report names it for {:?}, got {:?}",
            content,
            err.to_string()
        );
        assert!(
            wt.join("uncommitted.txt").exists(),
            "and the work is still there after {:?}",
            content
        );
    }
}

/// Skeptical pass 3, SR-16: the repair hint was given whenever the source
/// repo's record named some other existing directory, which is what a copy
/// looks like, not a move. Following it hands the original's registration to
/// the copy, and the next removal then deletes the admin directory the
/// original still uses. A space duplicated with `cp -R` is ordinary.
#[test]
fn remove_workspace_does_not_tell_a_copy_to_repair() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let original = worktree_in_space(&env, &repo, "feat");
    let copy_space = env.workspaces_dir.join("feat-copy");
    std::fs::create_dir_all(copy_space.join("alpha")).unwrap();
    std::fs::copy(original.join(".git"), copy_space.join("alpha").join(".git")).unwrap();
    std::fs::write(copy_space.join("alpha").join("mine.txt"), "copy work").unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "feat-copy", true)
        .expect_err("git will not remove a copy it does not know");
    let text = err.to_string();
    assert!(
        !text.contains("git worktree repair"),
        "a copy must not be told to take over the original's registration, got {:?}",
        text
    );
    assert!(
        text.contains("it is a copy of a worktree git still knows")
            && text.contains("keeps this copy")
            && !text.contains("deletes this copy"),
        "it is told what it is and what removing the space again would do, which is \
         keep it (ticket 36), got {:?}",
        text
    );
    assert_eq!(
        git_ok(&original, &["rev-parse", "--is-inside-work-tree"]).trim(),
        "true",
        "and the original, outside this space, still works"
    );
}

/// The summary lists two kept names, then a count, so a space with many kept
/// directories cannot push the reason off an 80-column status row.
#[test]
fn remove_workspace_summary_caps_the_names_it_lists() {
    let env = common::TestEnv::new();
    let space_dir = env.workspaces_dir.join("many-ws");
    std::fs::create_dir_all(&space_dir).unwrap();
    for name in ["a-one", "b-two", "c-three"] {
        git_ok(
            &space_dir,
            &[
                "init",
                "--quiet",
                "--bare",
                space_dir.join(name).to_str().unwrap(),
            ],
        );
    }

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "many-ws", true)
        .expect_err("three repositories of their own are kept");
    let summary = err.to_string().lines().next().unwrap().to_string();
    assert!(
        summary.contains("and 1 more") && !summary.contains("c-three"),
        "two names and a count, got {:?}",
        summary
    );
}

/// A gitdir naming a regular file is reported as such, rather than as a
/// repository of its own: it is neither, and the advice differs.
#[test]
fn remove_workspace_says_when_a_gitdir_names_a_file() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    worktree_in_space(&env, &repo, "file-ws");
    let odd = env.workspaces_dir.join("file-ws").join("z-odd");
    std::fs::create_dir_all(&odd).unwrap();
    std::fs::write(
        odd.join(".git"),
        format!("gitdir: {}\n", repo.join(".git").join("HEAD").display()),
    )
    .unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "file-ws", true)
        .expect_err("kept");
    assert!(
        err.to_string().contains("not a directory"),
        "the reason says what is wrong with it, got {:?}",
        err.to_string()
    );
    assert!(odd.join(".git").exists(), "and it is still there");
}

/// Coordinator review of 175c8ce: a relative gitdir breaks when EITHER side
/// moves, so for one a `NotFound` cannot tell "the source repo is gone" from
/// "the space was moved". An absolute gitdir survives a move of the worktree,
/// which is why only its `NotFound` is read as an orphan. This worktree was
/// made under `worktree.useRelativePaths` and its space moved one level
/// deeper; it used to be deleted on the first removal, with a clean success.
/// It is kept, and the advice it gets is followed here to prove it works.
#[test]
fn remove_workspace_keeps_a_relative_worktree_whose_space_moved_deeper() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    git_ok(&repo, &["config", "worktree.useRelativePaths", "true"]);
    let before = env.workspaces_dir.join("moved-ws").join("alpha");
    git_ok(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "moved-ws",
            before.to_str().unwrap(),
        ],
    );
    let gitfile = std::fs::read_to_string(before.join(".git")).unwrap();
    if !gitfile.starts_with("gitdir: ..") {
        // This git predates worktree.useRelativePaths (git 2.48), so there is
        // no relative gitdir to move; say so rather than passing quietly.
        eprintln!(
            "skipped: this git writes absolute gitdirs, got {:?}",
            gitfile
        );
        return;
    }
    std::fs::write(before.join("uncommitted.txt"), "wip").unwrap();

    // One level deeper: a new workspaces dir that holds the old one's space.
    let deeper = env.workspaces_dir.join("nested");
    std::fs::create_dir_all(&deeper).unwrap();
    std::fs::rename(env.workspaces_dir.join("moved-ws"), deeper.join("moved-ws")).unwrap();
    let after = deeper.join("moved-ws").join("alpha");

    let text = space::core::workspace::remove_workspace(&deeper, "moved-ws", true)
        .expect_err("a relative gitdir that does not resolve is not proof of an orphan")
        .to_string();
    assert_eq!(
        std::fs::read_to_string(after.join("uncommitted.txt")).unwrap(),
        "wip",
        "the work in it is still there"
    );
    assert!(
        text.contains("git worktree repair") && text.contains("from the source repo"),
        "and the report says how to fix it, got {:?}",
        text
    );

    // The advice works: repair from the source repo, then remove again.
    git_ok(&repo, &["worktree", "repair", after.to_str().unwrap()]);
    space::core::workspace::remove_workspace(&deeper, "moved-ws", true).unwrap();
    let left = registered_worktrees(&repo);
    assert!(
        !left.contains("moved-ws"),
        "after the repair the removal unregisters it, got {}",
        left
    );
}

/// A pair in a space renamed by hand: the admin directory's record names
/// where the original used to be, so neither half is the original here.
/// Claiming one was told the real original to delete itself by hand. With
/// no half at the recorded place, both are told the same thing, which is
/// what git has recorded and the repair that fixes it.
#[test]
fn remove_workspace_tells_a_moved_pair_to_repair_not_to_delete_the_original() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let original = worktree_in_space(&env, &repo, "pair-ws");
    let copy = env.workspaces_dir.join("pair-ws").join("z-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();
    std::fs::rename(
        env.workspaces_dir.join("pair-ws"),
        env.workspaces_dir.join("renamed-ws"),
    )
    .unwrap();

    let text = space::core::workspace::remove_workspace(&env.workspaces_dir, "renamed-ws", true)
        .expect_err("the pair is kept")
        .to_string();
    for name in ["alpha", "z-copy"] {
        let entry: String = text
            .lines()
            .skip_while(|l| !l.trim_start().starts_with(&format!("{:?}:", name)))
            .take(2)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !entry.contains("it is a copy of") && !entry.contains("a copy of it"),
            "{}: neither half is claimed to be the original, got {:?}",
            name,
            entry
        );
        assert!(
            entry.contains("git worktree repair"),
            "{}: both are told the repair that fixes a move, got {:?}",
            name,
            entry
        );
        // The user picks which one is real; if they pick the copy, "the
        // others" includes the original and its uncommitted work.
        assert!(
            entry.contains("keep what you need from the others"),
            "{}: and told to keep work before deleting anything, got {:?}",
            name,
            entry
        );
    }
}

/// Grouping compares canonical admin paths. A relative gitdir reaches the
/// same admin directory through different text from each half of a pair,
/// so without the canonical form the pair was not grouped, the original was
/// removed, and the copy was left with nothing registered.
#[test]
fn remove_workspace_keeps_a_relative_pair_together() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    git_ok(&repo, &["config", "worktree.useRelativePaths", "true"]);
    let original = env.workspaces_dir.join("relpair-ws").join("a-repo");
    git_ok(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "relpair-ws",
            original.to_str().unwrap(),
        ],
    );
    if !std::fs::read_to_string(original.join(".git"))
        .unwrap()
        .starts_with("gitdir: ..")
    {
        eprintln!("skipped: this git writes absolute gitdirs");
        return;
    }
    let copy = env.workspaces_dir.join("relpair-ws").join("z-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();
    std::fs::write(copy.join("work.txt"), "mine").unwrap();

    for run in 1..=2 {
        space::core::workspace::remove_workspace(&env.workspaces_dir, "relpair-ws", true)
            .expect_err("the relative pair is kept together");
        assert!(original.join(".git").exists(), "run {}: original kept", run);
        assert!(copy.join("work.txt").exists(), "run {}: copy kept", run);
    }
}

/// A relative gitdir that does not resolve names the directory in its advice,
/// and that line must not carry a hostile name onto the summary, which is the
/// line the TUI shows.
#[test]
fn remove_workspace_unresolved_advice_keeps_the_summary_clean() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    git_ok(&repo, &["config", "worktree.useRelativePaths", "true"]);
    let hostile = "z-\u{1b}[2Jwiped";
    let before = env.workspaces_dir.join("hmove-ws").join(hostile);
    git_ok(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "hmove-ws",
            before.to_str().unwrap(),
        ],
    );
    if !std::fs::read_to_string(before.join(".git"))
        .unwrap()
        .starts_with("gitdir: ..")
    {
        eprintln!("skipped: this git writes absolute gitdirs");
        return;
    }
    let deeper = env.workspaces_dir.join("nested");
    std::fs::create_dir_all(&deeper).unwrap();
    std::fs::rename(env.workspaces_dir.join("hmove-ws"), deeper.join("hmove-ws")).unwrap();

    let text = space::core::workspace::remove_workspace(&deeper, "hmove-ws", true)
        .expect_err("kept")
        .to_string();
    let summary = text.lines().next().unwrap();
    assert!(
        !summary.contains('\u{1b}'),
        "no escape byte in the summary, got {:?}",
        summary
    );
    assert!(
        text.contains("delete this directory by hand"),
        "the manual step says which directory, got {:?}",
        text
    );
    // The CLI prints the whole report to stderr, body included.
    assert!(
        !text.contains('\u{1b}'),
        "no escape byte anywhere in the report, got {:?}",
        text
    );
}

/// Which member of a pair is the original is read from git's record, not from
/// where it sorts or where it sits among the space's entries. Here the copy
/// sorts first and a plain directory sits between the two, so a member's
/// position in the pair differs from its position in the space. Mixing those
/// up, or taking the first member as the original, tells the real original
/// it is a copy and to delete itself by hand.
#[test]
fn remove_workspace_finds_the_original_of_a_pair_wherever_it_sorts() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("beta");
    let original = worktree_in_space(&env, &repo, "sort-ws");
    let space = env.workspaces_dir.join("sort-ws");
    let copy = space.join("0-beta-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();
    std::fs::create_dir_all(space.join("Alpha-plain")).unwrap();

    let text = space::core::workspace::remove_workspace(&env.workspaces_dir, "sort-ws", true)
        .expect_err("the pair is kept")
        .to_string();
    let entry = |name: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(&format!("{:?}:", name)))
            .unwrap_or_else(|| panic!("an entry for {}, in {:?}", name, text))
            .to_string()
    };
    assert!(
        entry("beta").contains("a copy of it") && !entry("beta").contains("it is a copy"),
        "the original is told it has a copy, got {:?}",
        entry("beta")
    );
    assert!(
        entry("0-beta-copy").contains("it is a copy of \"beta\""),
        "the copy is told it is one, of the real original, got {:?}",
        entry("0-beta-copy")
    );
}

/// Two copies of a worktree that lives outside the space: neither is the
/// original, and the one piece of advice they must not get is the repair,
/// which would take the outside worktree's registration away from it.
#[test]
fn remove_workspace_never_tells_copies_of_an_outside_worktree_to_repair() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("alpha");
    let outside = worktree_in_space(&env, &repo, "home-ws");
    let copies = env.workspaces_dir.join("copies-ws");
    for name in ["c1", "c2"] {
        std::fs::create_dir_all(copies.join(name)).unwrap();
        std::fs::copy(outside.join(".git"), copies.join(name).join(".git")).unwrap();
    }

    let text = space::core::workspace::remove_workspace(&env.workspaces_dir, "copies-ws", true)
        .expect_err("the copies are kept")
        .to_string();
    assert!(
        !text.contains("git worktree repair"),
        "copies of an outside worktree are never told to repair, got {:?}",
        text
    );
    assert!(
        text.contains("home-ws"),
        "they are told where the worktree git knows lives, got {:?}",
        text
    );
    assert_eq!(
        git_ok(&outside, &["rev-parse", "--is-inside-work-tree"]).trim(),
        "true",
        "and the outside worktree still works"
    );
}

// ---------------------------------------------------------------------------
// Ticket 25: a remote-tracking branch of any remote becomes a local tracking
// branch, the way `origin/<name>` always has.
// ---------------------------------------------------------------------------

/// A repo with two remotes, `origin` and `upstream` (bare repos beside it),
/// plus a third remote named `a/b` (a slash in a remote name is legal) that
/// points at the same bare repo as `upstream`. `feat` exists on both `origin`
/// and `upstream` at DIFFERENT tips, so an assertion on the checked-out
/// commit can tell the two apart; `only-up` exists on `upstream` only. The
/// repo itself keeps one local branch, `main`, and every remote-tracking ref
/// is fetched. `upstream/HEAD` is set so the picker's `*/HEAD` exclusion is
/// exercised.
struct TwoRemotes {
    _tmp: TempDir,
    repo: PathBuf,
    origin: PathBuf,
    origin_feat: String,
    upstream_feat: String,
}

fn two_remote_repo(env: &TestEnv) -> TwoRemotes {
    let tmp = TempDir::new().unwrap();
    let repo = env.create_repo("two");
    let origin = tmp.path().join("origin.git");
    let upstream = tmp.path().join("upstream.git");
    for bare in [&origin, &upstream] {
        std::fs::create_dir_all(bare).unwrap();
        git_ok(bare, &["init", "-q", "--bare", "-b", "main"]);
    }
    git_ok(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git_ok(
        &repo,
        &["remote", "add", "upstream", upstream.to_str().unwrap()],
    );
    git_ok(&repo, &["remote", "add", "a/b", upstream.to_str().unwrap()]);

    git_ok(&repo, &["branch", "feat", "main"]);
    git_ok(&repo, &["push", "-q", "origin", "main", "feat"]);
    let origin_feat = git_ok(&repo, &["rev-parse", "refs/heads/feat"])
        .trim()
        .to_string();

    // A decoy tip for upstream's `feat`, minted without touching the
    // checked-out branch (ticket 24's lesson: two empty commits in one
    // second are the same commit, so mint through `commit-tree`).
    let decoy = git_ok(
        &repo,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-m",
            "upstream-only",
        ],
    )
    .trim()
    .to_string();
    assert_ne!(decoy, origin_feat, "fixture: upstream's feat must differ");
    git_ok(&repo, &["update-ref", "refs/heads/feat", &decoy]);
    git_ok(&repo, &["branch", "only-up", "main"]);
    git_ok(
        &repo,
        &["push", "-q", "upstream", "main", "feat", "only-up"],
    );
    git_ok(&repo, &["branch", "-q", "-D", "feat", "only-up"]);

    git_ok(&repo, &["fetch", "-q", "--all"]);
    git_ok(&repo, &["remote", "set-head", "upstream", "main"]);
    assert_eq!(
        git_ok(&repo, &["rev-parse", "refs/remotes/upstream/feat"]).trim(),
        decoy,
        "fixture: upstream/feat is the decoy"
    );
    TwoRemotes {
        _tmp: tmp,
        repo,
        origin,
        origin_feat,
        upstream_feat: decoy,
    }
}

fn head_symref(wt: &Path) -> String {
    git_ok(wt, &["symbolic-ref", "HEAD"]).trim().to_string()
}

fn upstream_of(repo: &Path, branch: &str) -> String {
    git_ok(
        repo,
        &[
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{}", branch),
        ],
    )
    .trim()
    .to_string()
}

/// T1. `upstream/feat` from the picker is checked out as a new local `feat`
/// tracking `refs/remotes/upstream/feat`, at upstream's tip and not
/// origin's. Before the fix git took the name as a commit-ish and detached.
#[test]
fn an_upstream_existing_branch_becomes_a_local_tracking_branch() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t1",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
    )
    .expect("upstream/feat must be checked out as a tracking branch");

    assert_eq!(
        head_symref(&wt),
        "refs/heads/feat",
        "on a branch, not detached"
    );
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.upstream_feat, "at upstream's tip");
    assert_ne!(head, f.origin_feat, "and not origin's");
}

/// T2. When a local `feat` already exists, whichever remote it tracks, git
/// refuses with its own sentence and nothing is created or moved; the same
/// rule `origin/<name>` has always had, with no fallback to the local
/// branch.
#[test]
fn an_upstream_branch_whose_local_name_exists_is_refused() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(&f.repo, &["branch", "-q", "--track", "feat", "origin/feat"]);

    for (ws, tracks) in [
        ("t2-origin", "origin/feat"),
        ("t2-upstream", "upstream/feat"),
    ] {
        git_ok(&f.repo, &["branch", "-q", "-u", tracks, "feat"]);
        let err = create_worktree(
            &f.repo,
            &env.workspaces_dir,
            ws,
            &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
        )
        .expect_err("a local feat already exists")
        .to_string();
        assert!(
            err.contains("a branch named 'feat' already exists"),
            "git's own refusal, got {:?}",
            err
        );
        // The space directory itself is made before the add, as it is for
        // every strategy; the worktree is what must be absent.
        assert!(
            !env.workspaces_dir.join(ws).join("two").exists(),
            "no worktree is created for {}",
            ws
        );
        assert!(
            !registered_worktrees(&f.repo).contains(ws),
            "and none is registered for {}",
            ws
        );
        assert_eq!(
            git_ok(&f.repo, &["rev-parse", "refs/heads/feat"]).trim(),
            f.origin_feat,
            "the local feat is not moved"
        );
        assert_eq!(
            upstream_of(&f.repo, "feat"),
            format!("refs/remotes/{}", tracks),
            "and keeps its upstream"
        );
    }
}

/// T3. A remote named `a/b` is matched as a whole, so `a/b/feat` becomes the
/// local `feat` tracking `refs/remotes/a/b/feat`, not a local `b/feat`.
#[test]
fn a_remote_named_with_a_slash_splits_at_the_remote() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t3",
        &BranchStrategy::ExistingBranch("a/b/feat".to_string()),
    )
    .expect("a/b/feat must be checked out as a tracking branch");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/a/b/feat");
}

/// T4. A prefix that names no configured remote is not a remote: the name
/// goes to git as it is, and a local branch called `nobody/feat` is checked
/// out as that branch.
#[test]
fn a_prefix_that_names_no_remote_stays_a_plain_name() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(&f.repo, &["branch", "-q", "nobody/feat", "main"]);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t4",
        &BranchStrategy::ExistingBranch("nobody/feat".to_string()),
    )
    .expect("a local branch with a slash in its name is checked out");

    assert_eq!(head_symref(&wt), "refs/heads/nobody/feat");
}

/// T5. Ticket 13's guard runs on the derived name for every remote: the
/// `-M` that `upstream/-M` would hand to `-b` is refused before git runs,
/// with git's sentence, and the source repo's checked-out branch keeps its
/// name. (A local branch of that whole name would win instead, and is
/// checked out safely: T18.)
#[test]
fn an_upstream_dash_branch_is_refused_before_git() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);

    let err = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t5",
        &BranchStrategy::ExistingBranch("upstream/-M".to_string()),
    )
    .expect_err("the derived -b name begins with '-'");
    assert_eq!(err.to_string(), "'-M' is not a valid branch name");
    assert_eq!(head_symref(&f.repo), "refs/heads/main");
    assert!(!env.workspaces_dir.join("t5").exists());
}

/// T6. `upstream/feat` reads `refs/remotes/upstream/feat`, which a fetch of
/// `origin` never writes under the default refspec, so the pre-create fetch
/// is skipped (ticket 16's rule, generalised). Positive evidence: origin
/// gains a branch after the clone, and after the create the repo still has
/// no `refs/remotes/origin/newb`, which the fetch would have created.
#[test]
fn an_upstream_existing_branch_runs_no_fetch() {
    use space::core::workspace::{create_worktree_with_fetch, PreCreateFetch};
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.origin,
        &["update-ref", "refs/heads/newb", &f.origin_feat],
    );

    let attempt = create_worktree_with_fetch(
        &f.repo,
        &env.workspaces_dir,
        "t6",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
        PreCreateFetch::Run(std::time::Duration::from_secs(20)),
    );
    let wt = attempt.created.expect("the worktree must be created");
    assert_eq!(attempt.fetch, None, "the fetch is skipped");
    let seen = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/newb",
        ])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(
        !seen.success(),
        "no fetch reached origin: origin/newb is unknown"
    );
    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
}

/// T7. When origin's fetch refspec writes under `refs/remotes/upstream/`,
/// the fetch can change what `upstream/feat` names, so it runs. Positive
/// evidence: the worktree lands on what origin wrote there (origin's
/// `feat`), not on the decoy the ref held before. Upstream's own refspec is
/// dropped first: with two remotes mapping the same ref, git refuses
/// `--track` outright (`not tracking: ambiguous information for ref`,
/// git 2.50.1), whether or not a fetch ran.
#[test]
fn an_origin_refspec_writing_another_remotes_refs_still_fetches() {
    use space::core::workspace::{create_worktree_with_fetch, FetchOutcome, PreCreateFetch};
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(&f.repo, &["config", "--unset-all", "remote.upstream.fetch"]);
    git_ok(
        &f.repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/upstream/*",
        ],
    );

    let attempt = create_worktree_with_fetch(
        &f.repo,
        &env.workspaces_dir,
        "t7",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
        PreCreateFetch::Run(std::time::Duration::from_secs(20)),
    );
    let wt = attempt.created.expect("the worktree must be created");
    assert_eq!(attempt.fetch, Some(FetchOutcome::Ok), "the fetch ran");
    assert_eq!(
        git_ok(&wt, &["rev-parse", "HEAD"]).trim(),
        f.origin_feat,
        "the add read the ref origin's fetch had just written"
    );
}

/// T8. The picker's source lists every remote's branches, not origin's
/// alone, each marked remote, and no `*/HEAD`.
#[test]
fn list_branches_offers_every_remotes_branches() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);

    let branches = space::core::git::list_branches(&f.repo).unwrap();
    for name in [
        "origin/feat",
        "upstream/feat",
        "upstream/only-up",
        "a/b/feat",
    ] {
        let row = branches
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("{} is listed, got {:?}", name, branches));
        assert!(row.is_remote, "{} is marked remote", name);
    }
    assert!(
        branches.iter().all(|b| !b.name.ends_with("/HEAD")),
        "no remote HEAD row, got {:?}",
        branches
    );
}

/// T11 (skeptical review of PR #59). The tracking arm creates a local
/// branch as well as reading a remote-tracking ref, so ticket 16's other
/// invariant still holds for it: a repo whose origin refspec writes into
/// `refs/heads/*` fetches whatever the strategy. Positive evidence: the
/// fetch writes the local `mirror-main` the refspec names, and the add
/// still lands on upstream's `feat`.
#[test]
fn an_origin_refspec_writing_local_branches_fetches_for_an_upstream_name() {
    use space::core::workspace::{create_worktree_with_fetch, FetchOutcome, PreCreateFetch};
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/main:refs/heads/mirror-main",
        ],
    );

    let attempt = create_worktree_with_fetch(
        &f.repo,
        &env.workspaces_dir,
        "t11",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
        PreCreateFetch::Run(std::time::Duration::from_secs(20)),
    );
    let wt = attempt.created.expect("the worktree must be created");
    assert_eq!(attempt.fetch, Some(FetchOutcome::Ok), "the fetch ran");
    let mirrored = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/mirror-main"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(
        mirrored.success(),
        "the fetch wrote the local branch its refspec names"
    );
    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
}

// ---------------------------------------------------------------------------
// Ticket 25, push confirmation: where a bare push of a tracking branch goes.
// ---------------------------------------------------------------------------

/// T12. A confirmed push from a worktree made from `upstream/feat` lands on
/// upstream, and nowhere else: the bare `git push` the git-ops worker runs
/// after the confirmation follows the branch's own remote.
#[test]
fn a_confirmed_push_from_an_upstream_tracking_branch_lands_on_upstream() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let upstream = f._tmp.path().join("upstream.git");
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t12",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
    )
    .unwrap();
    git_ok(&wt, &["commit", "-q", "--allow-empty", "-m", "local-work"]);
    let pushed = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();

    let result = space::core::workspace::push_repo(&wt, false);

    assert!(result.success, "the push must succeed: {}", result.message);
    assert_eq!(
        git_ok(&upstream, &["rev-parse", "refs/heads/feat"]).trim(),
        pushed,
        "upstream's feat is the pushed commit"
    );
    assert_eq!(
        git_ok(&f.origin, &["rev-parse", "refs/heads/feat"]).trim(),
        f.origin_feat,
        "origin's feat is untouched"
    );
}

/// T13. `git::push_target` names the remote git would push to and what the
/// branch tracks, resolved as git resolves a bare push: `branch.<n>.pushRemote`
/// first, then `remote.pushDefault`, then `branch.<n>.remote`.
#[test]
fn push_target_resolves_the_destination_as_git_does() {
    use space::core::git::push_target;
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t13",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
    )
    .unwrap();

    let t = push_target(&wt).expect("a tracking branch has a push target");
    assert_eq!(
        (t.remote.as_str(), t.tracks.as_str()),
        ("upstream", "upstream/feat")
    );

    git_ok(&wt, &["config", "remote.pushDefault", "a/b"]);
    let t = push_target(&wt).unwrap();
    assert_eq!(
        t.remote, "a/b",
        "remote.pushDefault overrides the branch's remote"
    );
    assert_eq!(
        t.tracks, "upstream/feat",
        "and does not change what it tracks"
    );

    git_ok(&wt, &["config", "branch.feat.pushRemote", "origin"]);
    let t = push_target(&wt).unwrap();
    assert_eq!(t.remote, "origin", "branch.<n>.pushRemote overrides both");

    git_ok(&f.repo, &["checkout", "-q", "--detach"]);
    assert!(
        push_target(&f.repo).is_none(),
        "a detached HEAD has no target"
    );
    git_ok(&f.repo, &["checkout", "-q", "main"]);
    assert!(
        push_target(&f.repo).is_none(),
        "a branch with no upstream has no target: that case has its own prompt"
    );
}

/// T14 (security pass). A push-destination key that is present but cannot
/// be read is not skipped in favour of a lower-priority key, which git
/// would not do either: the destination is unknown, and the caller asks.
#[test]
fn push_target_is_unknown_when_a_higher_priority_key_cannot_be_read() {
    use space::core::git::push_target;
    use std::io::Write;
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t14",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
    )
    .unwrap();
    assert_eq!(push_target(&wt).unwrap().remote, "upstream");

    let config = f.repo.join(".git").join("config");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config)
        .unwrap();
    file.write_all(b"[branch \"feat\"]\n\tpushRemote = \xff\xfe\n")
        .unwrap();

    assert!(
        push_target(&wt).is_none(),
        "an unreadable pushRemote is unknown, not upstream"
    );
}
/// The shared hold's give-up guards, each run directly under `sh` so the
/// suite itself can fail a guard that was dropped (ticket 31). Every fixture
/// that uses the hold relies on these and cannot prove them: in a healthy run
/// the release always arrives, so a helper with no give-up path passes every
/// fixture. Ticket 22 recorded that gap as a residual; this closes it.
mod hold_guards {
    use super::common::hold::Hold;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    /// Start the hold's script under `sh`, in the test binary's own process
    /// group, and wait until it is holding.
    fn start(hold: &Hold, tmp: &TempDir, tail: &str) -> Child {
        let script = tmp.path().join("hold.sh");
        hold.write_script(&script, tail);
        let child = Command::new("/bin/sh")
            .arg(&script)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        hold.wait_holding(Duration::from_secs(10), "the guard probe", || None);
        child
    }

    /// The helper's exit status, if it exits within `limit`; `None` if it is
    /// still running, in which case it is killed so it cannot outlive the test.
    fn exit_within(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A tail that records that it ran, so a give-up test can assert it did
    /// not: the tail is gated on the receipt, and a helper that gave up must
    /// never do the fixture's work.
    fn recording_tail(tmp: &TempDir) -> (std::path::PathBuf, String) {
        let ran = tmp.path().join("tail-ran");
        let tail = format!(": > '{}'", ran.display());
        (ran, tail)
    }

    #[test]
    fn gives_up_without_a_record_when_its_marker_is_gone() {
        let tmp = TempDir::new().unwrap();
        let hold = Hold::new(tmp.path(), "marker");
        let (ran, tail) = recording_tail(&tmp);
        let mut child = start(&hold, &tmp, &tail);
        std::fs::remove_file(&hold.holding).unwrap();
        let status = exit_within(&mut child, Duration::from_secs(5))
            .expect("the helper must exit once its marker is gone");
        assert!(!status.success(), "a helper that gave up must not exit 0");
        assert!(!ran.exists(), "a helper that gave up must not run its tail");
        assert!(
            !hold.saw_release(),
            "no release was written, so none may be recorded"
        );
    }

    #[test]
    fn gives_up_without_a_record_when_the_test_pid_is_gone() {
        let tmp = TempDir::new().unwrap();
        // A pid no process can have: above the platform's maximum, so
        // `kill -0` fails at once and stays failed.
        let hold = Hold::new(tmp.path(), "pid").watching_pid(i32::MAX as u32);
        let (ran, tail) = recording_tail(&tmp);
        let script = tmp.path().join("hold.sh");
        hold.write_script(&script, &tail);
        let mut child = Command::new("/bin/sh")
            .arg(&script)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let status = exit_within(&mut child, Duration::from_secs(5))
            .expect("the helper must exit once the pid it watches is gone");
        assert!(!status.success(), "a helper that gave up must not exit 0");
        assert!(!ran.exists(), "a helper that gave up must not run its tail");
        assert!(
            !hold.saw_release(),
            "no release was written, so none may be recorded"
        );
    }

    #[test]
    fn gives_up_without_a_record_at_its_cap() {
        let tmp = TempDir::new().unwrap();
        let hold = Hold::new(tmp.path(), "cap").with_cap(3);
        let (ran, tail) = recording_tail(&tmp);
        let mut child = start(&hold, &tmp, &tail);
        let status = exit_within(&mut child, Duration::from_secs(5))
            .expect("the helper must exit once its cap is reached");
        assert!(!status.success(), "a helper that gave up must not exit 0");
        assert!(!ran.exists(), "a helper that gave up must not run its tail");
        assert!(
            !hold.saw_release(),
            "no release was written, so none may be recorded"
        );
    }

    #[test]
    fn a_released_helper_records_it_and_runs_its_tail() {
        let tmp = TempDir::new().unwrap();
        let hold = Hold::new(tmp.path(), "released");
        let done = tmp.path().join("tail-ran");
        let mut child = start(&hold, &tmp, &format!(": > '{}'", done.display()));
        assert!(
            exit_within(&mut child, Duration::from_millis(300)).is_none(),
            "a helper nobody released must still be holding"
        );
        // The first helper was killed with its marker still in place; clear it
        // so the second start waits for its own helper rather than seeing the
        // stale marker.
        std::fs::remove_file(&hold.holding).unwrap();
        let mut child = start(&hold, &tmp, &format!(": > '{}'", done.display()));
        hold.release();
        let status = exit_within(&mut child, Duration::from_secs(5))
            .expect("the helper must exit once released");
        assert!(
            status.success(),
            "a released helper runs its tail and exits 0"
        );
        assert!(
            hold.saw_release(),
            "the helper must record that it saw the release"
        );
        assert!(done.exists(), "the tail must have run");
    }
}

// ---------------------------------------------------------------------------
// Ticket 25, residual 3 (coordinator decision): for a remote other than
// origin, a local branch named by the whole string wins, git's own
// precedence and master's behaviour for those names.
// ---------------------------------------------------------------------------

/// The two-remote fixture plus a remote named `alice` (the same bare repo as
/// `upstream`) with a branch `fix` on it, so `alice/fix` is a remote-tracking
/// ref and a plausible local branch name at once.
fn with_alice(f: &TwoRemotes) {
    let upstream = f._tmp.path().join("upstream.git");
    git_ok(
        &f.repo,
        &["remote", "add", "alice", upstream.to_str().unwrap()],
    );
    git_ok(&f.repo, &["push", "-q", "upstream", "main:fix"]);
    git_ok(&f.repo, &["fetch", "-q", "alice"]);
}

/// T15. A local branch `alice/fix` beside a remote `alice` that has `fix`:
/// picking the local row checks out that branch, as master did; no local
/// `fix` is created.
#[test]
fn a_local_branch_named_like_another_remotes_branch_is_checked_out() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    with_alice(&f);
    git_ok(&f.repo, &["branch", "-q", "alice/fix", "main"]);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t15",
        &BranchStrategy::ExistingBranch("alice/fix".to_string()),
    )
    .expect("the local branch is checked out");

    assert_eq!(head_symref(&wt), "refs/heads/alice/fix");
    let fix = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/fix"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(!fix.success(), "no local fix is created for the local row");
}

/// T16. Without the local branch, `alice/fix` is the remote-tracking form:
/// a local `fix` tracking `refs/remotes/alice/fix`.
#[test]
fn without_the_local_branch_another_remotes_branch_still_tracks() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    with_alice(&f);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t16",
        &BranchStrategy::ExistingBranch("alice/fix".to_string()),
    )
    .expect("alice/fix is checked out as a tracking branch");

    assert_eq!(head_symref(&wt), "refs/heads/fix");
    assert_eq!(upstream_of(&f.repo, "fix"), "refs/remotes/alice/fix");
}

/// T17. `origin/<x>` keeps master's rule: always the tracking form, even
/// beside a local branch literally named `origin/<x>` (master: the
/// `origin/` prefix test ran before anything else; probed on git 2.50.1,
/// `--track -b feat -- wt refs/remotes/origin/feat` ignores a local
/// `origin/feat`).
#[test]
fn an_origin_collision_keeps_masters_rule() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(&f.repo, &["branch", "-q", "origin/feat", "main"]);

    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t17",
        &BranchStrategy::ExistingBranch("origin/feat".to_string()),
    )
    .expect("origin/feat is the tracking form");

    assert_eq!(
        head_symref(&wt),
        "refs/heads/feat",
        "not the local origin/feat"
    );
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/origin/feat");
    assert_eq!(
        git_ok(&wt, &["rev-parse", "HEAD"]).trim(),
        f.origin_feat,
        "at origin's tip"
    );
}

/// T18. The dash guard follows the same rule: a local branch named
/// `alice/-M` is checked out as that branch (the name sits after `--`, so
/// `-M` never reaches `-b`) and the source repo's checked-out branch keeps
/// its name; without the local branch the derived `-M` is still refused.
#[test]
fn a_local_dash_branch_named_like_another_remotes_is_checked_out_safely() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    with_alice(&f);

    let err = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t18-refused",
        &BranchStrategy::ExistingBranch("alice/-M".to_string()),
    )
    .expect_err("with no such local branch the derived -b name is refused");
    assert_eq!(err.to_string(), "'-M' is not a valid branch name");

    git_ok(&f.repo, &["branch", "-q", "alice/-M", "main"]);
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "t18",
        &BranchStrategy::ExistingBranch("alice/-M".to_string()),
    )
    .expect("the local branch is checked out");
    assert_eq!(head_symref(&wt), "refs/heads/alice/-M");
    assert_eq!(
        head_symref(&f.repo),
        "refs/heads/main",
        "the source branch keeps its name"
    );
}

/// T19 (independent review). The skip rule derives through the same
/// repo-aware split as the add: a local `alice/fix` makes `alice/fix` the
/// plain-name arm, which reads no origin ref, so the fetch is skipped even
/// when origin's refspec writes under `refs/remotes/alice/` (which the
/// tracking arm would have to fetch for). Positive evidence: origin gains a
/// branch after the clone and the repo never learns of it under the
/// refspec that would have written it.
#[test]
fn a_local_branch_named_like_another_remotes_branch_skips_the_fetch() {
    use space::core::workspace::{create_worktree_with_fetch, PreCreateFetch};
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    with_alice(&f);
    git_ok(&f.repo, &["branch", "-q", "alice/fix", "main"]);
    git_ok(
        &f.repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/alice/*",
        ],
    );
    git_ok(
        &f.origin,
        &["update-ref", "refs/heads/newb", &f.origin_feat],
    );

    let attempt = create_worktree_with_fetch(
        &f.repo,
        &env.workspaces_dir,
        "t19",
        &BranchStrategy::ExistingBranch("alice/fix".to_string()),
        PreCreateFetch::Run(std::time::Duration::from_secs(20)),
    );
    let wt = attempt.created.expect("the local branch is checked out");
    assert_eq!(attempt.fetch, None, "the fetch is skipped");
    let seen = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/remotes/alice/newb"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(
        !seen.success(),
        "no fetch reached origin: alice/newb is unknown"
    );
    assert_eq!(head_symref(&wt), "refs/heads/alice/fix");
}

/// T20 (skeptical review). The pre-create fetch can delete the local
/// branch the guard relied on: an origin refspec writing under
/// `refs/heads/alice/` with `fetch.prune` removes a local `alice/-M` that
/// origin does not have. The name is derived once, before the fetch, so the
/// add still runs the form the guard accepted (the whole name after `--`)
/// and `-M` never reaches `-b`: the source repo's checked-out branch keeps
/// its name whatever the add then does. Positive evidence that the prune
/// path was exercised: the local branch is gone afterwards.
#[test]
fn a_local_branch_pruned_by_the_fetch_cannot_turn_into_a_dash_option() {
    use space::core::workspace::{create_worktree_with_fetch, FetchOutcome, PreCreateFetch};
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    with_alice(&f);
    git_ok(&f.repo, &["push", "-q", "upstream", "main:-M"]);
    git_ok(&f.repo, &["fetch", "-q", "alice"]);
    git_ok(&f.repo, &["branch", "-q", "alice/-M", "main"]);
    git_ok(
        &f.repo,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/heads/alice/*",
        ],
    );
    git_ok(&f.repo, &["config", "fetch.prune", "true"]);

    let attempt = create_worktree_with_fetch(
        &f.repo,
        &env.workspaces_dir,
        "t20",
        &BranchStrategy::ExistingBranch("alice/-M".to_string()),
        PreCreateFetch::Run(std::time::Duration::from_secs(20)),
    );

    assert_eq!(attempt.fetch, Some(FetchOutcome::Ok), "the fetch ran");
    let pruned = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/alice/-M"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(
        !pruned.success(),
        "the fetch pruned the local branch, the window this pins"
    );
    assert_eq!(
        head_symref(&f.repo),
        "refs/heads/main",
        "the source repo's checked-out branch keeps its name: {:?}",
        attempt
            .created
            .as_ref()
            .map(|_| ())
            .map_err(|e| e.to_string())
    );
    let renamed = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/-M"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(!renamed.success(), "no branch named -M was created");
}

// ---------------------------------------------------------------------------
// Ticket 36: a copy of a worktree is never deleted as an orphan while its
// source repo is there. An absolute gitdir that names nothing is an orphan
// only in git's own shape (`<common>/worktrees/<id>`) with `<common>` gone.
// ---------------------------------------------------------------------------

/// A source repo whose worktrees get absolute gitdirs, whatever the running
/// user's global config says: `worktree.useRelativePaths` makes them relative,
/// and a relative gitdir that names nothing is kept by another rule
/// (`Unresolved`), which these tests would then pass or fail on instead.
fn absolute_repo(env: &TestEnv, name: &str) -> PathBuf {
    let repo = env.create_repo(name);
    git_ok(&repo, &["config", "worktree.useRelativePaths", "false"]);
    repo
}

/// `cp -R`, as a user duplicating a space runs it.
fn cp_r(from: &Path, to: &Path) {
    let status = Command::new("cp")
        .arg("-R")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(status.success(), "fixture: cp -R {:?} {:?}", from, to);
}

fn remove_forced(env: &TestEnv, name: &str) -> anyhow::Result<()> {
    space::core::workspace::remove_workspace(&env.workspaces_dir, name, true)
}

/// The number of worktrees `repo` has registered, its own checkout included.
fn registered_count(repo: &Path) -> usize {
    registered_worktrees(repo)
        .lines()
        .filter(|l| l.starts_with("worktree "))
        .count()
}

/// A space duplicated with `cp -R`, the original's space removed first. git
/// deletes the admin directory, and `<common>/worktrees` with it since this
/// was the repo's only worktree, so the copy's `.git` names nothing. It used
/// to be deleted as an orphan on its space's first removal, with its work
/// and no report. Its source repo is still there, so it is kept, every time.
#[test]
fn a_cp_r_copy_is_kept_once_its_original_space_is_removed() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    worktree_in_space(&env, &repo, "feat");
    cp_r(
        &env.workspaces_dir.join("feat"),
        &env.workspaces_dir.join("feat-copy"),
    );
    let copy = env.workspaces_dir.join("feat-copy").join("alpha");
    std::fs::write(copy.join("mine.txt"), "a day of work").unwrap();

    remove_forced(&env, "feat").unwrap();
    assert!(
        !repo.join(".git").join("worktrees").exists(),
        "fixture: git removed `worktrees` with its last worktree, so that directory \
         cannot be the evidence the source repo is there"
    );

    for run in 1..=2 {
        let text = remove_forced(&env, "feat-copy")
            .expect_err("a copy whose source repo is there is kept")
            .to_string();
        assert!(
            text.contains("\"alpha\""),
            "run {}: the report names it, got {:?}",
            run,
            text
        );
        assert_eq!(
            std::fs::read_to_string(copy.join("mine.txt")).unwrap(),
            "a day of work",
            "run {}: and its work is still there",
            run
        );
    }
}

/// The same duplicate, the copy's space removed first. The first report used
/// to promise that removing the space again deletes the copy once the
/// original is gone, and that is what happened. What the report promises now
/// is what the later removals do: the copy is kept.
#[test]
fn a_cp_r_copy_removed_first_is_told_the_truth_about_later_removals() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    worktree_in_space(&env, &repo, "feat");
    cp_r(
        &env.workspaces_dir.join("feat"),
        &env.workspaces_dir.join("feat-copy"),
    );
    let copy = env.workspaces_dir.join("feat-copy").join("alpha");
    std::fs::write(copy.join("mine.txt"), "a day of work").unwrap();

    let first = remove_forced(&env, "feat-copy")
        .expect_err("git will not remove a copy it does not know")
        .to_string();
    assert!(
        first.contains("keeps this copy") && !first.contains("deletes this copy"),
        "the first report says later removals keep the copy, got {:?}",
        first
    );

    remove_forced(&env, "feat").unwrap();
    remove_forced(&env, "feat-copy").expect_err("and they do");
    assert_eq!(
        std::fs::read_to_string(copy.join("mine.txt")).unwrap(),
        "a day of work",
        "with its work"
    );
}

/// A copy whose `.git` was edited by hand to an admin id git never
/// registered, beside its original in the same space. Nothing ties it to the
/// original, so it is not grouped; it used to be deleted as an orphan on the
/// first removal, with no report. Its source repo is there, so it is kept,
/// and the original, a real worktree, is removed as asked.
#[test]
fn a_copy_edited_to_an_unregistered_admin_id_is_kept() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    let original = worktree_in_space(&env, &repo, "ws");
    let copy = env.workspaces_dir.join("ws").join("alpha-copy");
    cp_r(&original, &copy);
    let common = admin_dir_of(&original)
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    std::fs::write(
        copy.join(".git"),
        format!(
            "gitdir: {}\n",
            common.join("worktrees").join("alpha-copy").display()
        ),
    )
    .unwrap();
    std::fs::write(copy.join("mine.txt"), "a day of work").unwrap();

    let text = remove_forced(&env, "ws")
        .expect_err("the copy is kept")
        .to_string();
    assert!(
        text.contains("\"alpha-copy\"") && text.contains("removed: \"alpha\""),
        "the copy is kept and the original removed, got {:?}",
        text
    );
    assert!(!original.exists(), "the original is gone");
    assert_eq!(
        std::fs::read_to_string(copy.join("mine.txt")).unwrap(),
        "a day of work",
        "and the copy's work is still there"
    );
}

/// PR #44 residual 13's shape: a copy whose `.git` names its original's live
/// admin directory with blanks after the path, which git does not read. The
/// first removal keeps it as unreadable and removes the original; the retry
/// then found a path naming nothing and deleted it. It is kept on the retry.
#[test]
fn a_near_miss_copy_is_kept_on_the_retry_after_its_original_goes() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    let original = worktree_in_space(&env, &repo, "ws");
    let copy = env.workspaces_dir.join("ws").join("alpha-copy");
    cp_r(&original, &copy);
    std::fs::write(
        copy.join(".git"),
        format!("gitdir: {}  \n", admin_dir_of(&original).display()),
    )
    .unwrap();
    std::fs::write(copy.join("mine.txt"), "a day of work").unwrap();

    let first = remove_forced(&env, "ws")
        .expect_err("the near miss is unreadable, so kept")
        .to_string();
    assert!(
        first.contains("removed: \"alpha\""),
        "fixture: the original went on the first run, got {:?}",
        first
    );

    let retry = remove_forced(&env, "ws")
        .expect_err("the retry keeps it too")
        .to_string();
    assert!(
        retry.contains("\"alpha-copy\""),
        "the retry names it, got {:?}",
        retry
    );
    assert_eq!(
        std::fs::read_to_string(copy.join("mine.txt")).unwrap(),
        "a day of work",
        "and its work is still there"
    );
}

/// git writes an absolute worktree gitdir as `<common>/worktrees/<id>`, with
/// no `..` and no line break. A `.git`
/// naming anything else was written by hand (or is a `--separate-git-dir`
/// checkout, which space never makes), so where its source repo would be
/// cannot be read from it, and a `NotFound` on it is not proof the source
/// repo is gone. Each shape here is kept even when nothing on the path
/// exists, and each sits alone in its own space: an orphan is deleted only
/// with its space, which a kept neighbour would hold back.
///
/// Three rules keep them, and each is pinned by a shape only it catches.
/// The allow-list (`common_dir_in_gits_shape`): a last directory that is not
/// `worktrees`; a `..` through a directory that is not there; an old path on
/// the first line and the new one on the second, split by `\n` or by a bare
/// `\r`, which the near-miss check does not split on. The near-miss check's
/// blank-separated readings: the old path, a note, then the live repo's
/// admin path on one line (skeptical review of PR #61, pass 3). The nesting
/// rule (a missing `<common>` under a repository's `worktrees` directory):
/// the live admin path with a note glued on with no blank. The shapes the
/// reviews reproduced (a `..` through the `worktrees` directory git has
/// already removed, a second line or a note after the live admin path) are
/// held by more than one rule. The reason leads with what is wrong, and the
/// path, which may be long, stays off the summary line.
#[test]
fn a_gitdir_not_in_gits_own_shape_is_kept_when_it_names_nothing() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    let live_admin = repo.join(".git").join("worktrees").join("alpha");
    let old_admin = env
        .dir
        .path()
        .join("old-home")
        .join("alpha")
        .join(".git")
        .join("worktrees")
        .join("alpha");
    let joined = |sep: &str, first: &Path, second: &Path| {
        PathBuf::from(format!("{}{}{}", first.display(), sep, second.display()))
    };
    // The phrase each shape's summary leads with names the rule that kept
    // it: the allow-list ("does not write"), an alternative reading of the
    // line naming something live ("does not read"), or the nesting rule.
    let write = "in a form git does not write";
    let read = "in a form git does not read";
    let nested = "under another repository's worktrees directory";
    let shapes = [
        (
            "not-worktrees",
            env.dir.path().join("gone").join("deeper").join("alpha"),
            write,
        ),
        (
            "dot-dot-missing",
            env.dir
                .path()
                .join("missing")
                .join("..")
                .join("repos")
                .join("alpha")
                .join(".git")
                .join("worktrees")
                .join("alpha"),
            write,
        ),
        (
            "two-lines-moved",
            joined("\n", &old_admin, &live_admin),
            write,
        ),
        (
            "carriage-return-moved",
            joined("\r", &old_admin, &live_admin),
            write,
        ),
        (
            "same-line-moved",
            joined(" # now ", &old_admin, &live_admin),
            read,
        ),
        ("tab-moved", joined("\t", &old_admin, &live_admin), read),
        (
            "same-line-no-blank",
            joined("#was", &live_admin, &old_admin),
            nested,
        ),
        (
            "same-line",
            joined(" # was ", &live_admin, &old_admin),
            read,
        ),
        (
            "dot-dot",
            repo.join(".git")
                .join("worktrees")
                .join("..")
                .join("worktrees")
                .join("alpha"),
            write,
        ),
        (
            "two-lines",
            joined("\n# was ", &live_admin, &old_admin),
            read,
        ),
        (
            "carriage-return",
            joined("\r", &live_admin, &old_admin),
            write,
        ),
    ];
    assert_eq!(
        env.dir.path().join("repos").join("alpha"),
        repo,
        "fixture: `missing/../repos/alpha` would name the live repo"
    );
    assert!(
        !repo.join(".git").join("worktrees").exists(),
        "fixture: the repo has no worktrees, so `worktrees/..` names nothing"
    );
    for (name, gitdir, phrase) in &shapes {
        let dir = env.workspaces_dir.join(name).join("alpha");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        std::fs::write(dir.join("mine.txt"), "a day of work").unwrap();

        let text = match remove_forced(&env, name) {
            Ok(()) => panic!("{}: its space was removed, so it went as an orphan", name),
            Err(e) => e.to_string(),
        };
        assert!(
            text.contains("1 of 1 repos in the space were kept"),
            "{}: it is kept, not removed as an orphan, got {:?}",
            name,
            text
        );
        let summary = text.lines().next().unwrap_or_default();
        assert!(
            summary.contains(phrase) && !summary.contains(&env.dir.path().display().to_string()),
            "{}: the summary says {:?} and leaves the path off, got {:?}",
            name,
            phrase,
            summary
        );
        assert!(
            dir.join("mine.txt").exists(),
            "{}: and its work is still there",
            name
        );
    }
}

/// Removing a space is not blocked by a copy of one of its worktrees in
/// another space. Grouping across spaces was the rejected design: it would
/// make a space unremovable because of an unrelated one, and the copy is
/// protected where it is deleted instead.
#[test]
fn a_copy_in_another_space_does_not_keep_the_original() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    let original = worktree_in_space(&env, &repo, "feat");
    cp_r(
        &env.workspaces_dir.join("feat"),
        &env.workspaces_dir.join("feat-copy"),
    );
    let copy = env.workspaces_dir.join("feat-copy").join("alpha");
    std::fs::write(copy.join("mine.txt"), "a day of work").unwrap();

    remove_forced(&env, "feat").unwrap();

    assert!(!original.exists(), "the original's space is removed");
    assert_eq!(
        registered_count(&repo),
        1,
        "and git unregistered its worktree"
    );
    assert!(
        copy.join("mine.txt").exists(),
        "while the copy in the other space is left alone"
    );
}

/// The reason a copy whose source repo is there gets says that, and gives
/// neither of its neighbours' advice: not the repair a relative gitdir gets
/// (a copy handed the original's registration breaks the original), and not
/// "remove the space with force" (force keeps it too). Unforced says the same.
#[test]
fn a_copy_with_no_record_is_told_git_has_none() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    worktree_in_space(&env, &repo, "feat");
    cp_r(
        &env.workspaces_dir.join("feat"),
        &env.workspaces_dir.join("feat-copy"),
    );
    remove_forced(&env, "feat").unwrap();

    for force in [true, false] {
        let text =
            space::core::workspace::remove_workspace(&env.workspaces_dir, "feat-copy", force)
                .expect_err("kept")
                .to_string();
        let summary = text.lines().next().unwrap_or_default();
        assert!(
            summary.contains("git has no record of it"),
            "force {}: the summary's reason says git has no record, got {:?}",
            force,
            summary
        );
        assert!(
            text.contains("source repo is still there"),
            "force {}: and that its source repo is there, got {:?}",
            force,
            text
        );
        assert!(
            !text.contains("git worktree repair")
                && !text.contains("with force")
                && !text.contains("cannot be told; look at it"),
            "force {}: and nothing its neighbours are told, got {:?}",
            force,
            text
        );
    }
}

/// "Its source repo is still there" is said only of a repository. A gitdir in
/// git's shape whose `<common>` is a directory that is not a repository is
/// kept too, since nothing proves a repo is gone, but it is not told that a
/// repo is there (code review of PR #61).
#[test]
fn a_gitdir_under_a_directory_that_is_no_repository_is_not_called_one() {
    let env = TestEnv::new();
    let plain = env.dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let dir = env.workspaces_dir.join("ws").join("alpha");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(".git"),
        format!(
            "gitdir: {}\n",
            plain.join("worktrees").join("alpha").display()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("mine.txt"), "a day of work").unwrap();

    let text = remove_forced(&env, "ws")
        .expect_err("kept: nothing proves a repo is gone")
        .to_string();
    assert!(
        !text.contains("source repo is still there") && text.contains("not a git repository"),
        "it is not told a repo is there, got {:?}",
        text
    );
    assert!(dir.join("mine.txt").exists(), "and its work is still there");
}

/// An orphan is deleted only with its space, and the space is deleted only
/// when nothing was kept, so an orphan beside a kept directory is still on
/// disk when the report is read. The report used to say it was "removed",
/// which was never true when shown (independent review of PR #61).
#[test]
fn an_orphan_beside_a_kept_directory_is_not_reported_removed() {
    let env = TestEnv::new();
    let gone = absolute_repo(&env, "gone");
    let orphan = worktree_in_space(&env, &gone, "ws");
    std::fs::remove_dir_all(&gone).unwrap();
    let clone = env.workspaces_dir.join("ws").join("a-clone");
    git_ok(
        &env.workspaces_dir.join("ws"),
        &["init", "-q", clone.to_str().unwrap()],
    );

    let text = remove_forced(&env, "ws")
        .expect_err("the clone is kept")
        .to_string();
    let line = text
        .lines()
        .find(|l| l.contains("\"gone\""))
        .unwrap_or_else(|| panic!("the orphan is listed, got {:?}", text));
    assert!(
        !line.contains("removed") && line.contains("go with the space"),
        "it is told it goes with the space, not that it is gone, got {:?}",
        line
    );
    assert!(orphan.exists(), "and it is indeed still there");
}

/// The nesting rule reads only a repository under whose `worktrees`
/// directory the missing `<common>` sits. Its first form asked whether any
/// ancestor up to `/` looked like a repository, by the loose test that also
/// takes a directory holding `objects`, so a genuine orphan whose repos root
/// held a repo named `objects` was kept and told its gitfile was not git's
/// (skeptical review of PR #61, pass 3). It goes, as a genuine orphan does.
#[test]
fn a_genuine_orphan_beside_a_directory_that_looks_like_a_repository_is_removed() {
    let env = TestEnv::new();
    let repo = absolute_repo(&env, "alpha");
    std::fs::create_dir_all(env.repos_dir.join("objects")).unwrap();
    worktree_in_space(&env, &repo, "ws");
    std::fs::remove_dir_all(&repo).unwrap();

    remove_forced(&env, "ws").unwrap();

    assert!(
        !env.workspaces_dir.join("ws").exists(),
        "a space whose source repo is gone is removed"
    );
}

/// A source repo outside `TestEnv`'s repos root, with absolute worktree
/// gitdirs whatever the global config says (see `absolute_repo`).
fn absolute_repo_at(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path).unwrap();
    common::init_repo(path);
    git_ok(path, &["config", "worktree.useRelativePaths", "false"]);
    path.to_path_buf()
}

/// The blank-separated readings of a gitfile's line count only when one
/// names a live worktree or a worktree path of a repository that is there,
/// never merely something that exists. Counting existence kept a genuine
/// orphan whose clean gitdir runs through `My Projects` whenever a folder
/// `My` sat beside it, and told it its gitfile was not git's (independent
/// review of PR #61's `907fa8f`).
#[test]
fn a_genuine_orphan_whose_path_has_a_blank_is_removed_beside_a_folder_named_like_its_prefix() {
    let env = TestEnv::new();
    let repo = absolute_repo_at(&env.dir.path().join("My Projects").join("alpha"));
    std::fs::create_dir_all(env.dir.path().join("My")).unwrap();
    worktree_in_space(&env, &repo, "ws");
    std::fs::remove_dir_all(&repo).unwrap();

    remove_forced(&env, "ws").unwrap();

    assert!(
        !env.workspaces_dir.join("ws").exists(),
        "it goes, as a genuine orphan does"
    );
}

/// The nesting rule reads a missing `<common>` that leaves a repository's
/// `worktrees` directory into a name that does not exist, as a note glued to
/// the live admin path does, and nothing wider: a genuine orphan whose repos
/// root lies under a folder named `worktrees`, inside a directory that looks
/// like a repository, runs through real directories all the way down to the
/// deleted repo (independent review of PR #61's `907fa8f`).
#[test]
fn a_genuine_orphan_under_a_folder_named_worktrees_is_removed() {
    let env = TestEnv::new();
    let deep = env.dir.path().join("deep");
    std::fs::create_dir_all(deep.join("objects")).unwrap();
    let repo = absolute_repo_at(&deep.join("worktrees").join("projects").join("alpha"));
    worktree_in_space(&env, &repo, "ws");
    std::fs::remove_dir_all(&repo).unwrap();

    remove_forced(&env, "ws").unwrap();

    assert!(
        !env.workspaces_dir.join("ws").exists(),
        "it goes, as a genuine orphan does"
    );
}

/// The trimmed reading still matters beside the words: a path that itself
/// holds a blank splits into words that name nothing live, so only the
/// whole trimmed path finds the live admin directory. Without it this live
/// worktree would be told git has no record of it, which is false.
#[test]
fn a_gitfile_with_blanks_after_a_path_that_holds_a_blank_is_a_near_miss() {
    let env = TestEnv::new();
    let repo = absolute_repo_at(&env.dir.path().join("My Projects").join("alpha"));
    let wt = worktree_in_space(&env, &repo, "ws");
    let admin = admin_dir_of(&wt);
    std::fs::write(wt.join(".git"), format!("gitdir: {}  \n", admin.display())).unwrap();
    std::fs::write(wt.join("mine.txt"), "a day of work").unwrap();

    let text = remove_forced(&env, "ws")
        .expect_err("a gitfile git does not read is kept")
        .to_string();
    assert!(
        text.contains("in a form git does not read") && !text.contains("no record"),
        "it is told its gitfile is a near miss of a live worktree, got {:?}",
        text
    );
    assert!(wt.join("mine.txt").exists(), "and its work is still there");
}

// ---------------------------------------------------------------------------
// Ticket 42: pull, sync and status read the branch a branch tracks when that
// is its namesake on another remote, origin's as before otherwise, and
// refuse anything else on a remote that is not origin.
// ---------------------------------------------------------------------------

use space::core::workspace::{pull_repo, sync_repo, PullOutcome};

/// The bare repo `two_remote_repo` made for `upstream`.
fn upstream_bare(f: &TwoRemotes) -> PathBuf {
    f._tmp.path().join("upstream.git")
}

/// A new commit on `parent`, minted without touching any checkout.
fn mint(repo: &Path, parent: &str, msg: &str) -> String {
    let tree = format!("{}^{{tree}}", parent);
    git_ok(repo, &["commit-tree", &tree, "-p", parent, "-m", msg])
        .trim()
        .to_string()
}

/// Point `branch` on the bare repo at `sha`. Pushed by path, not by remote
/// name, so the clone's remote-tracking refs do not move: the clone sees the
/// commit only once something fetches that remote.
fn publish(repo: &Path, bare: &Path, sha: &str, branch: &str) {
    let refspec = format!("{}:refs/heads/{}", sha, branch);
    git_ok(
        repo,
        &["push", "-q", "-f", bare.to_str().unwrap(), &refspec],
    );
}

fn rev(repo: &Path, name: &str) -> String {
    git_ok(repo, &["rev-parse", name]).trim().to_string()
}

fn ref_present(repo: &Path, refname: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet", refname])
        .current_dir(repo)
        .status()
        .unwrap()
        .success()
}

fn is_ancestor(repo: &Path, ancestor: &str, of: &str) -> bool {
    Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, of])
        .current_dir(repo)
        .status()
        .unwrap()
        .success()
}

/// The space `s` made from `upstream/feat` by ticket 25's path, so `feat`
/// tracks `refs/remotes/upstream/feat` exactly as git wrote it.
fn upstream_feat_space(env: &TestEnv, f: &TwoRemotes) -> PathBuf {
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "s",
        &BranchStrategy::ExistingBranch("upstream/feat".to_string()),
    )
    .unwrap();
    assert_eq!(
        upstream_of(&f.repo, "feat"),
        "refs/remotes/upstream/feat",
        "fixture: feat tracks upstream's feat"
    );
    wt
}

/// T1. A pull of a branch that tracks its namesake on upstream merges
/// upstream's branch, never origin's. Origin's `feat` has diverged from it
/// and upstream's has moved on; on master the pull merged origin's `feat`
/// into the upstream-tracking branch.
#[test]
fn a_pull_follows_a_namesake_on_another_remote() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let origin_only = mint(&f.repo, &f.origin_feat, "origin-only");
    publish(&f.repo, &f.origin, &origin_only, "feat");
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");
    git_ok(&f.repo, &["fetch", "-q", "--all"]);

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::FastForwarded,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "Fast-forwarded feat to upstream/feat (1 commit(s))."
    );
    assert_eq!(rev(&wt, "HEAD"), upstream_next, "at upstream's new tip");
    assert!(
        !is_ancestor(&wt, &origin_only, "HEAD"),
        "origin's commit is not in the branch"
    );
}

/// T2. The pull fetches the remote the branch tracks, and its merge arm
/// merges that remote's branch: a commit that reached upstream after the
/// space was made is merged with the branch's own commit, though nothing in
/// the clone has fetched upstream since. Fetching origin and merging
/// upstream's ref would report the branch ahead; merging origin's ref here
/// would merge nothing of upstream's.
#[test]
fn a_pull_fetches_and_merges_the_remote_the_branch_tracks() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");
    assert_eq!(
        rev(&f.repo, "refs/remotes/upstream/feat"),
        f.upstream_feat,
        "fixture: the clone has not seen upstream's new commit"
    );
    git_ok(&wt, &["commit", "-q", "--allow-empty", "-m", "mine"]);

    let result = pull_repo(&wt);

    assert_eq!(result.outcome, PullOutcome::Merged, "{}", result.message);
    assert_eq!(
        result.message,
        "Merged upstream/feat into feat (1 ahead, 1 behind)."
    );
    assert!(
        is_ancestor(&wt, &upstream_next, "HEAD"),
        "upstream's new commit is in the branch"
    );
    assert_eq!(
        git_ok(&wt, &["log", "-1", "--format=%s"]).trim(),
        "Merge remote-tracking branch 'upstream/feat' into feat"
    );
}

/// T3. A branch that tracks a branch of another name on a remote that is
/// not origin is refused with a sentence, and nothing runs: `fix` tracks
/// `upstream/main`, origin has a `fix` strictly ahead of it, and upstream's
/// `main` has moved too. Falling back to origin would fast-forward to
/// origin's `fix`; following git's upstream would pull `main` into it;
/// reading the namesake on upstream would report `upstream/fix` missing.
#[test]
fn a_pull_refuses_an_upstream_of_another_name_on_another_remote() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let main = rev(&f.repo, "refs/remotes/upstream/main");
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "fix",
            "refs/remotes/upstream/main",
        ],
    );
    assert_eq!(
        upstream_of(&f.repo, "fix"),
        "refs/remotes/upstream/main",
        "fixture: fix tracks upstream's main"
    );
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "s",
        &BranchStrategy::ExistingBranch("fix".to_string()),
    )
    .unwrap();
    let origin_fix = mint(&f.repo, &main, "origin-fix");
    publish(&f.repo, &f.origin, &origin_fix, "fix");
    let upstream_main = mint(&f.repo, &main, "upstream-main");
    publish(&f.repo, &upstream_bare(&f), &upstream_main, "main");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::NoUpstream,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "fix tracks upstream/main, which space does not pull; nothing was pulled."
    );
    assert_eq!(rev(&wt, "HEAD"), main, "fix is where it was");
    assert!(
        !ref_present(&f.repo, "refs/remotes/origin/fix"),
        "and nothing was fetched from origin"
    );
    assert!(
        !wt.join(git_ok(&wt, &["rev-parse", "--git-path", "FETCH_HEAD"]).trim())
            .exists(),
        "or from anywhere"
    );
}

/// T4. A new branch started from `origin/main` tracks `origin/main` (git's
/// `branch.autoSetupMerge`), and a pull of it still reads `origin/<branch>`
/// as before ticket 42: the base is neither fast-forwarded nor merged into
/// the branch, which is what following git's upstream literally would do.
#[test]
fn a_pull_of_a_new_branch_still_reads_its_own_name_on_origin() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = create_worktree(
        &f.repo,
        &env.workspaces_dir,
        "newb",
        &BranchStrategy::NewBranch("newb".to_string()),
    )
    .unwrap();
    assert_eq!(
        upstream_of(&f.repo, "newb"),
        "refs/remotes/origin/main",
        "fixture: git set the new branch to track its base"
    );
    let start = rev(&wt, "HEAD");
    let main_next = mint(&f.repo, &start, "main-next");
    publish(&f.repo, &f.origin, &main_next, "main");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::NoUpstream,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "newb has no upstream (origin/newb) to pull."
    );
    assert_eq!(rev(&wt, "HEAD"), start, "the base is not pulled into it");
}

/// T5. A branch that pulls from upstream and pushes to origin (a fork's
/// triangular setup, `branch.<name>.pushRemote`) is pulled from upstream:
/// where a push goes (`push_target`) is not what a pull reads.
#[test]
fn a_pull_reads_where_a_branch_pulls_from_not_where_it_pushes() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    git_ok(&f.repo, &["config", "branch.feat.pushRemote", "origin"]);
    let origin_only = mint(&f.repo, &f.origin_feat, "origin-only");
    publish(&f.repo, &f.origin, &origin_only, "feat");
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::FastForwarded,
        "{}",
        result.message
    );
    assert_eq!(rev(&wt, "HEAD"), upstream_next, "at upstream's new tip");
    assert!(
        !is_ancestor(&wt, &origin_only, "HEAD"),
        "origin's commit is not in the branch"
    );
}

/// T6. A `branch.<name>.remote` that is not UTF-8 cannot be read, and an
/// upstream that cannot be read is never taken for origin: the pull says
/// so and runs nothing, though origin has a `feat` it could fast-forward to.
#[test]
fn a_pull_refuses_a_branch_whose_upstream_cannot_be_read() {
    use std::os::unix::ffi::OsStrExt;
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let origin_next = mint(&f.repo, &f.upstream_feat, "origin-next");
    publish(&f.repo, &f.origin, &origin_next, "feat");
    let fetched = rev(&f.repo, "refs/remotes/origin/feat");
    let out = Command::new("git")
        .arg("config")
        .arg("branch.feat.remote")
        .arg(std::ffi::OsStr::from_bytes(b"up\xffstream"))
        .current_dir(&f.repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "fixture: git stores the bytes");

    let result = pull_repo(&wt);

    assert_eq!(result.outcome, PullOutcome::Failed, "{}", result.message);
    assert!(
        result
            .message
            .starts_with("Could not read what feat tracks (")
            && result.message.ends_with("); nothing was pulled."),
        "got {:?}",
        result.message
    );
    assert_eq!(rev(&wt, "HEAD"), f.upstream_feat, "feat is where it was");
    assert_eq!(
        rev(&f.repo, "refs/remotes/origin/feat"),
        fetched,
        "and nothing was fetched"
    );
}

/// T7. The sync fast-forwards a local branch that tracks its namesake on
/// upstream to upstream's branch, not origin's, and its tracking stays as
/// it was. On master it moved `feat` onto origin's `feat`, and the
/// `branch -f` re-pointed it to track origin.
#[test]
fn a_sync_forwards_a_namesake_of_another_remote_to_that_remote() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "feat",
            "refs/remotes/upstream/feat",
        ],
    );
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");
    git_ok(&f.repo, &["fetch", "-q", "upstream"]);
    let origin_next = mint(&f.repo, &f.upstream_feat, "origin-next");
    publish(&f.repo, &f.origin, &origin_next, "feat");

    let result = sync_repo(&f.repo);

    assert_eq!(
        result.forwarded,
        vec!["feat".to_string()],
        "skipped: {:?}",
        result.skipped
    );
    assert_eq!(
        rev(&f.repo, "refs/heads/feat"),
        upstream_next,
        "at upstream's tip"
    );
    assert_eq!(
        upstream_of(&f.repo, "feat"),
        "refs/remotes/upstream/feat",
        "still tracking upstream"
    );
}

/// T8. The sync leaves alone a branch that tracks a branch of another name
/// on a remote that is not origin, though origin has a `fix` strictly ahead
/// of it and upstream's `main` has moved too.
#[test]
fn a_sync_leaves_an_upstream_of_another_name_on_another_remote() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let main = rev(&f.repo, "refs/remotes/upstream/main");
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "fix",
            "refs/remotes/upstream/main",
        ],
    );
    let origin_fix = mint(&f.repo, &main, "origin-fix");
    publish(&f.repo, &f.origin, &origin_fix, "fix");
    let upstream_main = mint(&f.repo, &main, "upstream-main");
    publish(&f.repo, &upstream_bare(&f), &upstream_main, "main");
    git_ok(&f.repo, &["fetch", "-q", "upstream"]);

    let result = sync_repo(&f.repo);

    assert!(
        result.forwarded.is_empty() && result.skipped.is_empty(),
        "nothing is forwarded or tried: {:?} {:?}",
        result.forwarded,
        result.skipped
    );
    assert_eq!(rev(&f.repo, "refs/heads/fix"), main, "fix is where it was");
    assert_eq!(
        upstream_of(&f.repo, "fix"),
        "refs/remotes/upstream/main",
        "and still tracks upstream's main"
    );
}

/// T11. A branch whose namesake on upstream is gone (deleted there and
/// pruned here) has no upstream to pull, and the pull says so naming
/// upstream's branch; origin's `feat`, strictly ahead of it, is not read.
#[test]
fn a_pull_reports_a_namesake_gone_from_its_remote_without_reading_origin() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let origin_next = mint(&f.repo, &f.upstream_feat, "origin-next");
    publish(&f.repo, &f.origin, &origin_next, "feat");
    git_ok(
        &f.repo,
        &[
            "push",
            "-q",
            upstream_bare(&f).to_str().unwrap(),
            ":refs/heads/feat",
        ],
    );
    git_ok(&f.repo, &["update-ref", "-d", "refs/remotes/upstream/feat"]);

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::NoUpstream,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "feat has no upstream (upstream/feat) to pull."
    );
    assert_eq!(rev(&wt, "HEAD"), f.upstream_feat, "feat is where it was");
}

/// T13. A branch whose `branch.<name>.remote` names a remote the repo does
/// not have is refused, though its merge names its own branch: `git fetch
/// -- ghost` would read the name as a path, and here a repository sits at
/// `ghost` beside the worktree, so the pull would fetch from it.
#[test]
fn a_pull_refuses_a_namesake_on_a_remote_the_repo_does_not_have() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let ghost = wt.join("ghost");
    git_ok(
        &wt,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "feat",
            ghost.to_str().unwrap(),
        ],
    );
    publish(&f.repo, &ghost, &f.upstream_feat, "feat");
    git_ok(&f.repo, &["config", "branch.feat.remote", "ghost"]);
    let fetch_head = wt.join(git_ok(&wt, &["rev-parse", "--git-path", "FETCH_HEAD"]).trim());
    assert!(
        !fetch_head.exists(),
        "fixture: the worktree has not fetched"
    );

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::NoUpstream,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "feat tracks ghost/feat, which space does not pull; nothing was pulled."
    );
    assert!(!fetch_head.exists(), "and nothing was fetched from ./ghost");
}

/// T14. A branch with two `branch.<name>.merge` values tracks two branches
/// (git's pull merges both), which is not the one namesake space pulls: it
/// is refused rather than read by whichever value was added last, here its
/// own name, while upstream's `feat` has moved on.
#[test]
fn a_pull_refuses_a_branch_that_tracks_two_branches() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    git_ok(
        &f.repo,
        &[
            "config",
            "--replace-all",
            "branch.feat.merge",
            "refs/heads/main",
        ],
    );
    git_ok(
        &f.repo,
        &["config", "--add", "branch.feat.merge", "refs/heads/feat"],
    );
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::NoUpstream,
        "{}",
        result.message
    );
    assert_eq!(
        result.message,
        "feat tracks upstream/main and upstream/feat, which space does not pull; \
         nothing was pulled."
    );
    assert_eq!(rev(&wt, "HEAD"), f.upstream_feat, "feat is where it was");
}

/// T15. A `branch.<name>.merge` written with no value (a bare `merge` line,
/// which only a hand edit makes) cannot be read: the pull says so and runs
/// nothing, rather than panicking on the missing value.
#[test]
fn a_pull_refuses_a_branch_whose_merge_key_has_no_value() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let config = f.repo.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[branch \"feat\"]\n\tmerge\n");
    std::fs::write(&config, text).unwrap();

    let result = pull_repo(&wt);

    assert_eq!(result.outcome, PullOutcome::Failed, "{}", result.message);
    assert_eq!(
        result.message,
        "Could not read what feat tracks (branch.feat.merge: set with no value); \
         nothing was pulled."
    );
    assert_eq!(rev(&wt, "HEAD"), f.upstream_feat, "feat is where it was");
}

/// T16. A local branch named `upstream/feat` (the guide's `alice/fix` beside
/// a remote `alice`) wins git's lookup of the short name over
/// `refs/remotes/upstream/feat`, so the merge arm must not hand git the
/// short name when it names another commit: upstream's commit is merged,
/// the local branch's is not.
#[test]
fn a_pull_merges_the_remote_branch_though_a_local_branch_shares_its_short_name() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let decoy = mint(&f.repo, &f.origin_feat, "local-decoy");
    git_ok(&f.repo, &["branch", "upstream/feat", &decoy]);
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");
    git_ok(&wt, &["commit", "-q", "--allow-empty", "-m", "mine"]);

    let result = pull_repo(&wt);

    assert_eq!(result.outcome, PullOutcome::Merged, "{}", result.message);
    assert!(
        is_ancestor(&wt, &upstream_next, "HEAD"),
        "upstream's commit is merged"
    );
    assert!(
        !is_ancestor(&wt, &decoy, "HEAD"),
        "the local branch named upstream/feat is not"
    );
}

/// T17. A remote whose fetch refspecs include a negative one
/// (`^refs/heads/wip/*`, which git accepts) is still a remote the repo has:
/// libgit2 cannot load it (`find_remote` fails), but it is listed, and a
/// branch tracking its namesake there is pulled from it.
#[test]
fn a_pull_follows_a_namesake_on_a_remote_with_a_negative_refspec() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    git_ok(
        &f.repo,
        &[
            "config",
            "--add",
            "remote.upstream.fetch",
            "^refs/heads/wip/*",
        ],
    );
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::FastForwarded,
        "{}",
        result.message
    );
    assert_eq!(rev(&wt, "HEAD"), upstream_next, "at upstream's new tip");
}

/// T18. A tag named like the branch makes `git symbolic-ref --short HEAD`
/// answer `heads/feat`, a name with no `branch.heads/feat.*` keys; the pull
/// still reads the branch's own tracking, fast-forwards to upstream, and
/// names the branch `feat`.
#[test]
fn a_pull_follows_a_namesake_though_a_tag_shares_the_branch_name() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    git_ok(&f.repo, &["tag", "feat", &f.origin_feat]);
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::FastForwarded,
        "{}",
        result.message
    );
    assert_eq!(
        result.message, "Fast-forwarded feat to upstream/feat (1 commit(s)).",
        "named feat, not heads/feat"
    );
    assert_eq!(rev(&wt, "HEAD"), upstream_next, "at upstream's new tip");
}

/// T19. A `branch.<name>.remote` written with no value (a bare `remote`
/// line after the real one, which git reads last) cannot be read: the pull
/// says so, rather than describing a remote named nothing.
#[test]
fn a_pull_refuses_a_branch_whose_remote_key_has_no_value() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    let config = f.repo.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[branch \"feat\"]\n\tremote\n");
    std::fs::write(&config, text).unwrap();

    let result = pull_repo(&wt);

    assert_eq!(result.outcome, PullOutcome::Failed, "{}", result.message);
    assert_eq!(
        result.message,
        "Could not read what feat tracks (branch.feat.remote: set with no value); \
         nothing was pulled."
    );
    assert_eq!(rev(&wt, "HEAD"), f.upstream_feat, "feat is where it was");
}

/// T20. A `branch.<name>.remote` set twice is read as git reads it, by its
/// last value: `origin` then `upstream` pulls from upstream, whose `feat`
/// has moved on, and not from origin, whose `feat` has diverged.
#[test]
fn a_pull_reads_the_last_value_of_a_remote_key_set_twice() {
    let env = TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = upstream_feat_space(&env, &f);
    git_ok(
        &f.repo,
        &["config", "--replace-all", "branch.feat.remote", "origin"],
    );
    git_ok(
        &f.repo,
        &["config", "--add", "branch.feat.remote", "upstream"],
    );
    let origin_only = mint(&f.repo, &f.origin_feat, "origin-only");
    publish(&f.repo, &f.origin, &origin_only, "feat");
    let upstream_next = mint(&f.repo, &f.upstream_feat, "upstream-next");
    publish(&f.repo, &upstream_bare(&f), &upstream_next, "feat");

    let result = pull_repo(&wt);

    assert_eq!(
        result.outcome,
        PullOutcome::FastForwarded,
        "{}",
        result.message
    );
    assert_eq!(rev(&wt, "HEAD"), upstream_next, "at upstream's new tip");
}

// ---------------------------------------------------------------------------
// Ticket 41: `switch_worktree_branch` asks for each ref exactly, and reads a
// picked `<remote>/<x>` the way the create path does since ticket 25.
// ---------------------------------------------------------------------------

/// A worktree of the two-remote fixture, detached at `main`, for the switch
/// to move.
fn detached_wt(env: &TestEnv, f: &TwoRemotes, ws: &str) -> PathBuf {
    create_worktree(
        &f.repo,
        &env.workspaces_dir,
        ws,
        &BranchStrategy::DetachedHead,
    )
    .unwrap()
}

/// A commit that is no branch's tip, for a shadowing tag to point at.
fn switch_decoy(f: &TwoRemotes) -> String {
    let decoy = git_ok(
        &f.repo,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-m",
            "t41-decoy",
        ],
    )
    .trim()
    .to_string();
    assert_ne!(
        decoy, f.origin_feat,
        "fixture: the decoy is not origin's feat"
    );
    assert_ne!(decoy, f.upstream_feat, "fixture: nor upstream's");
    decoy
}

/// Whether `refname` exists exactly, the way `ref_exists` asks.
fn has_ref(repo: &Path, refname: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet", refname])
        .current_dir(repo)
        .status()
        .unwrap()
        .success()
}

/// Whether `wt`'s HEAD is detached (no symbolic ref).
fn is_detached(wt: &Path) -> bool {
    !Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(wt)
        .status()
        .unwrap()
        .success()
}

/// T1, the local probe. A tag literally named `refs/heads/feat` is not a
/// local branch `feat`. Master asked `rev-parse --verify refs/heads/feat`,
/// which falls back to that tag, took the branch as present and ran `git
/// switch -- feat`; git's own remote guess then failed because three
/// remotes carry `feat` (with one it would have guessed right, which hid
/// the bug). Asked exactly, the probe says no and `origin/feat` becomes a
/// local `feat` tracking it, at origin's tip.
#[test]
fn switch_finds_no_local_branch_through_a_tag_named_like_its_ref() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let decoy = switch_decoy(&f);
    git_ok(&f.repo, &["tag", "refs/heads/feat", &decoy]);
    assert!(
        !has_ref(&f.repo, "refs/heads/feat"),
        "fixture: no local feat"
    );
    let wt = detached_wt(&env, &f, "t41-1");

    space::core::workspace::switch_worktree_branch(&wt, "origin/feat", false)
        .expect("origin/feat becomes a local tracking branch");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/origin/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.origin_feat, "at origin's tip");
}

/// T2, the remote probe. A branch no remote has is not found through a tag
/// of either name it could be read as: `origin/<x>` (master's bare probe
/// resolved it and created `<x>` at the tag) or `refs/remotes/origin/<x>`
/// (`rev-parse --verify` on the qualified name falls back to that tag, and
/// `switch -c <x> refs/remotes/origin/<x>` then starts at it). Asked
/// exactly, both say no and git reports the name; nothing is created.
#[test]
fn switch_finds_no_remote_branch_through_a_tag_of_either_name() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let decoy = switch_decoy(&f);
    git_ok(&f.repo, &["tag", "origin/gone-bare", &decoy]);
    git_ok(
        &f.repo,
        &["tag", "refs/remotes/origin/gone-qualified", &decoy],
    );
    let wt = detached_wt(&env, &f, "t41-2");

    for name in ["gone-bare", "gone-qualified"] {
        assert!(
            !has_ref(&f.repo, &format!("refs/remotes/origin/{}", name)),
            "fixture: origin has no {}",
            name
        );
        let picked = format!("origin/{}", name);
        let err = space::core::workspace::switch_worktree_branch(&wt, &picked, false)
            .expect_err("no remote branch has this name")
            .to_string();
        assert!(
            err.contains(name),
            "git reports the name {}, got {:?}",
            name,
            err
        );
        assert!(
            is_detached(&wt),
            "the worktree has not moved for {}",
            picked
        );
        assert!(
            !has_ref(&f.repo, &format!("refs/heads/{}", name)),
            "no local {} is created",
            name
        );
    }
}

/// T3, the start point. A tag `origin/feat` beside the real remote branch
/// made master's `git switch -c feat origin/feat` fail with `ambiguous
/// object name`; the qualified start point names the remote-tracking ref
/// alone, so `feat` starts at origin's tip, not the tag's, and tracks it.
#[test]
fn switch_starts_a_remote_branch_at_it_beside_a_tag_of_its_name() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let decoy = switch_decoy(&f);
    git_ok(&f.repo, &["tag", "origin/feat", &decoy]);
    let wt = detached_wt(&env, &f, "t41-3");

    space::core::workspace::switch_worktree_branch(&wt, "origin/feat", false)
        .expect("origin/feat becomes a local tracking branch beside the tag");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/origin/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.origin_feat, "at origin's tip");
    assert_ne!(head, decoy, "not the tag's commit");
}

/// T4, ticket 25's split. `upstream/feat` from the picker becomes a local
/// `feat` tracking `refs/remotes/upstream/feat`, at upstream's tip and not
/// origin's, as the create path makes it. Master stripped `origin/` only,
/// so git was handed `upstream/feat` and refused with `a branch is
/// expected, got remote branch`.
#[test]
fn switch_to_another_remotes_branch_tracks_that_remote() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = detached_wt(&env, &f, "t41-4");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect("upstream/feat becomes a local tracking branch");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.upstream_feat, "at upstream's tip");
    assert_ne!(head, f.origin_feat, "and not origin's");
}

/// T5, ticket 25's local-first rule. A local branch literally named
/// `upstream/feat` wins over the remote one of that name: the switch checks
/// it out, as master did, and makes no local `feat`.
#[test]
fn switch_to_a_local_branch_named_like_another_remotes_branch_checks_it_out() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let decoy = switch_decoy(&f);
    git_ok(&f.repo, &["branch", "-q", "upstream/feat", &decoy]);
    let wt = detached_wt(&env, &f, "t41-5");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect("the local branch is checked out");

    assert_eq!(head_symref(&wt), "refs/heads/upstream/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, decoy, "at the local branch's own commit");
    assert!(
        !has_ref(&f.repo, "refs/heads/feat"),
        "no local feat is created"
    );
}

/// T6, the fallthrough. A picked `origin/only-up` that origin does not
/// have (a prune elsewhere between the picker and Enter) is an error, not
/// a branch tracking whichever remote git's guess finds. Master ran `git
/// switch -- only-up`, and with `only-up` on upstream alone git guessed
/// and tracked upstream, a remote nobody picked.
#[test]
fn switch_to_a_remote_branch_that_is_gone_guesses_no_other_remote() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(&f.repo, &["remote", "remove", "a/b"]);
    assert!(
        has_ref(&f.repo, "refs/remotes/upstream/only-up"),
        "fixture: upstream has only-up"
    );
    assert!(
        !has_ref(&f.repo, "refs/remotes/a/b/only-up")
            && !has_ref(&f.repo, "refs/remotes/origin/only-up"),
        "fixture: and no other remote does"
    );
    let wt = detached_wt(&env, &f, "t41-6");

    let err = space::core::workspace::switch_worktree_branch(&wt, "origin/only-up", false)
        .expect_err("origin has no only-up")
        .to_string();

    assert!(
        err.contains("only-up"),
        "git reports the name, got {:?}",
        err
    );
    assert!(is_detached(&wt), "the worktree has not moved");
    assert!(
        !has_ref(&f.repo, "refs/heads/only-up"),
        "no local only-up is created"
    );
}

/// T7. For a remote other than origin, an existing local `feat` that tracks
/// another line (here origin's) is not switched to in place of the picked
/// `upstream/feat`: the switch is refused with a sentence naming both, and
/// nothing moves. Head `1edf10d` switched to it silently, so the dashboard
/// showed origin's code under a pick of upstream's.
#[test]
fn switch_to_another_remotes_branch_refuses_a_local_branch_on_another_line() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "feat",
            "refs/remotes/origin/feat",
        ],
    );
    let wt = detached_wt(&env, &f, "t41-7");

    let err = space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("the local feat tracks origin, not upstream")
        .to_string();

    assert!(
        err.contains("feat exists and does not track upstream"),
        "the refusal names the pick, got {:?}",
        err
    );
    assert!(is_detached(&wt), "the worktree has not moved");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/origin/feat");
}

/// T8. A local `feat` that already tracks `upstream/feat` (the first pick
/// of the row made it) is where a second pick of that row goes, although
/// the remote-tracking ref exists too.
#[test]
fn switch_to_another_remotes_branch_goes_to_the_local_branch_tracking_it() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "feat",
            "refs/remotes/upstream/feat",
        ],
    );
    let wt = detached_wt(&env, &f, "t41-8");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect("the local feat tracking upstream/feat is checked out");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
}

/// T9. `origin/<x>` keeps master's rule: an existing local `feat` is
/// checked out whatever it tracks, here nothing, at its own commit, beside
/// the remote-tracking ref of the same name.
#[test]
fn switch_to_origins_branch_checks_out_the_local_branch_whatever_it_tracks() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let decoy = switch_decoy(&f);
    git_ok(&f.repo, &["branch", "-q", "--no-track", "feat", &decoy]);
    let wt = detached_wt(&env, &f, "t41-9");

    space::core::workspace::switch_worktree_branch(&wt, "origin/feat", false)
        .expect("the local feat is checked out");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, decoy, "at the local branch's own commit");
    assert_eq!(upstream_of(&f.repo, "feat"), "", "still tracking nothing");
}

/// T10. The new branch tracks the picked remote-tracking ref whatever
/// `branch.autoSetupMerge` says: `false` and `inherit` set no upstream from
/// a remote-tracking start point unless `--track` is passed (probed on git
/// 2.50.1), and a branch with no upstream would later be offered `push -u
/// origin`, publishing upstream's commits there.
#[test]
fn switch_tracks_the_picked_remote_whatever_auto_setup_merge_says() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = detached_wt(&env, &f, "t41-10");

    for (mode, picked, local) in [
        ("false", "upstream/feat", "feat"),
        ("inherit", "upstream/only-up", "only-up"),
    ] {
        git_ok(&f.repo, &["config", "branch.autoSetupMerge", mode]);
        space::core::workspace::switch_worktree_branch(&wt, picked, false)
            .unwrap_or_else(|e| panic!("{} under {}: {}", picked, mode, e));
        assert_eq!(
            upstream_of(&f.repo, local),
            format!("refs/remotes/{}", picked),
            "{} tracks {} under autoSetupMerge {}",
            local,
            picked,
            mode
        );
    }
}

/// T11. A remote whose name holds a slash splits at the longest configured
/// remote through the switch too: `a/b/feat` is `feat` on remote `a/b`
/// (upstream's bare repo), tracking `refs/remotes/a/b/feat`.
#[test]
fn switch_splits_a_remote_named_with_a_slash_at_the_remote() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let wt = detached_wt(&env, &f, "t41-11");

    space::core::workspace::switch_worktree_branch(&wt, "a/b/feat", false)
        .expect("a/b/feat becomes a local tracking branch");

    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/a/b/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.upstream_feat, "at upstream's tip");
}

/// T12, the remote probe where nothing else stands behind it. With a
/// configured remote, `switch -c <x> --track <ref>` also refuses a tag as
/// its start point (`not a branch`), so T2 alone cannot tell an exact probe
/// from `rev-parse --verify`. A repo with no `origin` configured still reads
/// `origin/<x>` as origin's (the default remote), gets no `--track`, and
/// there only the exact probe keeps a tag of either name, or of both, from
/// becoming the new branch.
#[test]
fn switch_finds_no_remote_branch_through_a_tag_without_an_origin_remote() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("plain");
    assert_eq!(git_ok(&repo, &["remote"]).trim(), "", "fixture: no remotes");
    let decoy = git_ok(
        &repo,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-m",
            "t41-decoy",
        ],
    )
    .trim()
    .to_string();
    git_ok(&repo, &["tag", "origin/gone-bare", &decoy]);
    git_ok(
        &repo,
        &["tag", "refs/remotes/origin/gone-qualified", &decoy],
    );
    // Both at once: a bare probe finds the first, and the qualified start
    // point then resolves to the second.
    git_ok(&repo, &["tag", "origin/gone-both", &decoy]);
    git_ok(&repo, &["tag", "refs/remotes/origin/gone-both", &decoy]);
    let wt = create_worktree(
        &repo,
        &env.workspaces_dir,
        "t41-12",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    for name in ["gone-bare", "gone-qualified", "gone-both"] {
        let picked = format!("origin/{}", name);
        let err = space::core::workspace::switch_worktree_branch(&wt, &picked, false)
            .expect_err("no remote branch has this name")
            .to_string();
        assert!(
            err.contains(name),
            "git reports the name {}, got {:?}",
            name,
            err
        );
        assert!(
            is_detached(&wt),
            "the worktree has not moved for {}",
            picked
        );
        assert!(
            !has_ref(&repo, &format!("refs/heads/{}", name)),
            "no local {} is created",
            name
        );
    }
}

/// A commit on `main` that adds `t41.txt`, so its tree differs from every
/// fixture tip and a worktree half-switched to it shows in `git status`.
/// Every other fixture commit shares one tree, which is why no earlier test
/// could see a refusal that left files behind. Minted in a throwaway
/// worktree, removed again.
fn commit_adding_a_file(f: &TwoRemotes) -> String {
    let mint = f._tmp.path().join("t41-mint");
    git_ok(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            mint.to_str().unwrap(),
            "main",
        ],
    );
    std::fs::write(mint.join("t41.txt"), "from the remote\n").unwrap();
    git_ok(&mint, &["add", "t41.txt"]);
    git_ok(&mint, &["commit", "-q", "-m", "t41-tree"]);
    let sha = git_ok(&mint, &["rev-parse", "HEAD"]).trim().to_string();
    git_ok(
        &f.repo,
        &["worktree", "remove", "--force", mint.to_str().unwrap()],
    );
    sha
}

fn status_of(wt: &Path) -> String {
    git_ok(wt, &["status", "--porcelain"]).trim().to_string()
}

/// T13. A configured remote whose fetch refspec does not map the picked ref
/// (here narrowed to `main` after the fetch). For a remote other than
/// origin the pick is refused before anything is written: the branch could
/// track nothing, status, sync and pull would read it against origin, and
/// master and the create path both refuse it. Origin keeps master's
/// untracked branch. `--track` there is git's `starting point ... is not a
/// branch`, which head `f0db95f` passed for any configured remote.
#[test]
fn switch_to_a_ref_no_refspec_maps_is_refused_except_for_origin() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    for remote in ["upstream", "origin"] {
        git_ok(
            &f.repo,
            &[
                "config",
                "--replace-all",
                &format!("remote.{}.fetch", remote),
                &format!("+refs/heads/main:refs/remotes/{}/main", remote),
            ],
        );
    }
    let wt = detached_wt(&env, &f, "t41-13");

    let err = space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("no fetch refspec of upstream maps upstream/feat")
        .to_string();
    assert!(
        err.contains("upstream/feat is not fetched by upstream"),
        "the refusal names the ref, got {:?}",
        err
    );
    assert!(is_detached(&wt), "the worktree has not moved");
    assert!(!has_ref(&f.repo, "refs/heads/feat"), "no branch is made");

    space::core::workspace::switch_worktree_branch(&wt, "origin/feat", false)
        .expect("origin/feat is still a branch to switch to");
    assert_eq!(head_symref(&wt), "refs/heads/feat");
    let head = git_ok(&wt, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(head, f.origin_feat, "at origin's tip");
    assert_eq!(upstream_of(&f.repo, "feat"), "", "tracking nothing");
}

/// T14. When two remotes' fetch refspecs map the picked ref, git refuses to
/// track it (`ambiguous information`), and does so after it has made the
/// branch and rewritten the worktree (probed). The switch refuses first:
/// no branch, no file of the target, HEAD where it was.
#[test]
fn switch_to_a_ref_two_refspecs_map_is_refused_before_anything_is_written() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let target = commit_adding_a_file(&f);
    git_ok(
        &f.repo,
        &["update-ref", "refs/remotes/upstream/feat", &target],
    );
    git_ok(
        &f.repo,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/upstream/*",
        ],
    );
    let wt = detached_wt(&env, &f, "t41-14");

    let err = space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("two remotes map refs/remotes/upstream/feat")
        .to_string();

    assert!(
        err.contains("upstream/feat is fetched by several remotes"),
        "the refusal names the ref, got {:?}",
        err
    );
    assert!(is_detached(&wt), "the worktree has not moved");
    assert!(!has_ref(&f.repo, "refs/heads/feat"), "no branch is left");
    assert_eq!(status_of(&wt), "", "and no file of the target");
}

/// T15. A ref in the way of the new branch (`refs/heads/feat/sub` blocks
/// `refs/heads/feat`) is refused by `git branch` before any file is touched.
/// `git switch -c` refused it only after rewriting the worktree and index to
/// the target, leaving them there with HEAD where it was (probed).
#[test]
fn switch_whose_new_branch_is_blocked_by_a_ref_leaves_the_worktree_as_it_was() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let target = commit_adding_a_file(&f);
    git_ok(
        &f.repo,
        &["update-ref", "refs/remotes/upstream/feat", &target],
    );
    git_ok(&f.repo, &["branch", "-q", "feat/sub", "main"]);
    let wt = detached_wt(&env, &f, "t41-15");

    let err = space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("refs/heads/feat/sub is in the way")
        .to_string();

    assert!(err.contains("feat"), "git names the ref, got {:?}", err);
    assert!(is_detached(&wt), "the worktree has not moved");
    assert_eq!(status_of(&wt), "", "no file of the target is left behind");
    assert!(!wt.join("t41.txt").exists());
}

/// T16. A switch git refuses for the worktree's own state (an untracked file
/// the target would overwrite) takes the branch it had just made back out,
/// so the next pick of the row is not refused as a local branch on another
/// line, and the user's file is untouched.
#[test]
fn switch_refused_by_the_worktree_leaves_no_new_branch_behind() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let target = commit_adding_a_file(&f);
    git_ok(
        &f.repo,
        &["update-ref", "refs/remotes/upstream/feat", &target],
    );
    let wt = detached_wt(&env, &f, "t41-16");
    std::fs::write(wt.join("t41.txt"), "mine\n").unwrap();

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("the untracked t41.txt would be overwritten");

    assert!(is_detached(&wt), "the worktree has not moved");
    assert!(!has_ref(&f.repo, "refs/heads/feat"), "no branch is left");
    assert_eq!(
        std::fs::read_to_string(wt.join("t41.txt")).unwrap(),
        "mine\n",
        "the user's file is untouched"
    );
    let remote_key = Command::new("git")
        .args(["config", "--get", "branch.feat.remote"])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(
        !remote_key.success(),
        "and no tracking config is left for a branch that is gone"
    );
}

/// T17. A remote carrying a negative fetch refspec (`^refs/heads/<x>`, read
/// by git since 2.29) cannot be read by libgit2 at all, so the mapper count
/// is unknown. Another remote's pick then passes `--track`: git tracks
/// upstream's `feat` even under `branch.autoSetupMerge=false`, and refuses
/// the `only-up` the negative refspec excludes (`not a branch`) with no
/// branch made. Counting the unreadable remote as mapping nothing (head
/// `9bbc48f`) made `feat` untracked; git's bare default (head `520c11f`)
/// would have made both untracked under this config.
#[test]
fn switch_lets_git_decide_when_a_remotes_refspecs_cannot_be_read() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "config",
            "--add",
            "remote.upstream.fetch",
            "^refs/heads/only-up",
        ],
    );
    git_ok(&f.repo, &["config", "branch.autoSetupMerge", "false"]);
    assert!(
        git2::Repository::open(&f.repo)
            .unwrap()
            .find_remote("upstream")
            .is_err(),
        "fixture: libgit2 cannot read upstream's refspecs"
    );
    assert!(
        has_ref(&f.repo, "refs/remotes/upstream/only-up"),
        "fixture: the excluded ref is still there"
    );
    let wt = detached_wt(&env, &f, "t41-17");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect("git tracks upstream/feat");
    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/only-up", false)
        .expect_err("no refspec of upstream maps the excluded only-up");
    assert_eq!(
        head_symref(&wt),
        "refs/heads/feat",
        "the worktree has not moved"
    );
    assert!(!has_ref(&f.repo, "refs/heads/only-up"), "no branch is made");
}

/// T18. Origin's pick under an unknown count keeps git's default, as on
/// master; another remote carries a negative refspec, so the count is
/// unknown. With origin not configured (a hand-made
/// `refs/remotes/origin/feat`) the new `feat` is untracked, where `--track`
/// would be git's `not a branch`. Once origin is configured and its refspec
/// maps the ref, the default tracks it, where `--no-track` would not.
#[test]
fn switch_to_origin_under_an_unknown_count_keeps_gits_default() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("plain");
    git_ok(&repo, &["remote", "add", "other", "/nonexistent/other"]);
    git_ok(
        &repo,
        &["config", "--add", "remote.other.fetch", "^refs/heads/skip"],
    );
    assert!(
        git2::Repository::open(&repo)
            .unwrap()
            .find_remote("other")
            .is_err(),
        "fixture: libgit2 cannot read other's refspecs"
    );
    let head = git_ok(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    git_ok(&repo, &["update-ref", "refs/remotes/origin/feat", &head]);
    let wt = create_worktree(
        &repo,
        &env.workspaces_dir,
        "t41-18",
        &BranchStrategy::DetachedHead,
    )
    .unwrap();

    space::core::workspace::switch_worktree_branch(&wt, "origin/feat", false)
        .expect("origin/feat becomes a branch, as on master");
    assert_eq!(head_symref(&wt), "refs/heads/feat");
    assert_eq!(upstream_of(&repo, "feat"), "", "tracking nothing");

    git_ok(&repo, &["remote", "add", "origin", "/nonexistent/origin"]);
    git_ok(&repo, &["update-ref", "refs/remotes/origin/feat2", &head]);
    space::core::workspace::switch_worktree_branch(&wt, "origin/feat2", false)
        .expect("origin/feat2 becomes a tracking branch");
    assert_eq!(head_symref(&wt), "refs/heads/feat2");
    assert_eq!(upstream_of(&repo, "feat2"), "refs/remotes/origin/feat2");
}

/// T19. An unreadable refspec on an unrelated remote (here `a/b`) makes the
/// count unknown for every pick, so the no-mapper refusal cannot see that
/// upstream's own refspec, narrowed to `main`, maps nothing. The pick still
/// passes `--track`, and git refuses it (`not a branch`) with nothing made,
/// as the known-count refusal would. Head `520c11f` used git's bare default
/// here and made an untracked `feat` (the delta review of `520c11f`).
#[test]
fn switch_refuses_an_unmappable_pick_when_another_remote_is_unreadable() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "config",
            "--replace-all",
            "remote.upstream.fetch",
            "+refs/heads/main:refs/remotes/upstream/main",
        ],
    );
    git_ok(
        &f.repo,
        &[
            "config",
            "--add",
            "remote.a/b.fetch",
            "^refs/heads/unrelated",
        ],
    );
    assert!(
        git2::Repository::open(&f.repo)
            .unwrap()
            .find_remote("a/b")
            .is_err(),
        "fixture: libgit2 cannot read a/b's refspecs"
    );
    let wt = detached_wt(&env, &f, "t41-19");

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("no refspec maps upstream/feat");

    assert!(is_detached(&wt), "the worktree has not moved");
    assert!(!has_ref(&f.repo, "refs/heads/feat"), "no branch is made");
}

/// T20. The cleanup after a refused switch takes back only what it made. A
/// `reference-transaction` hook plays another actor who advances the new
/// `feat` to a commit of their own the moment `git branch` has made it; the
/// switch to it is then refused (an untracked file the new tip would
/// overwrite), and a delete by name would lose that commit from the ref.
/// The cleanup deletes only while the ref still points where it was made,
/// so the moved branch is kept and the error says so.
#[test]
fn switch_cleanup_keeps_a_new_branch_someone_moved_in_between() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let work = commit_adding_a_file(&f);
    let wt = detached_wt(&env, &f, "t41-20");
    std::fs::write(wt.join("t41.txt"), "mine\n").unwrap();
    let marker = f._tmp.path().join("t41-20-race");
    std::fs::write(&marker, "").unwrap();
    let hook = f
        .repo
        .join(".git")
        .join("hooks")
        .join("reference-transaction");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\ncat >/dev/null\n[ \"$1\" = committed ] || exit 0\n\
             [ -f '{m}' ] || exit 0\nrm -f '{m}'\ngit update-ref refs/heads/feat {w}\n",
            m = marker.display(),
            w = work
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let err = space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("the moved feat's tree would overwrite the untracked t41.txt")
        .to_string();

    assert!(!marker.exists(), "fixture: the other actor ran");
    assert!(
        err.contains("feat kept, it moved meanwhile"),
        "the error says the branch was kept, got {:?}",
        err
    );
    assert_eq!(
        git_ok(&f.repo, &["rev-parse", "refs/heads/feat"]).trim(),
        work,
        "the other actor's commit is still on feat"
    );
    assert!(is_detached(&wt), "the worktree has not moved");
    assert_eq!(
        std::fs::read_to_string(wt.join("t41.txt")).unwrap(),
        "mine\n",
        "the user's file is untouched"
    );
}

/// T21. A switch to a local branch that already exists never cleans
/// anything up: here `feat` tracks the pick (so there is no early refusal),
/// git refuses the switch for the worktree's own reason (an untracked file
/// its tree would overwrite), and `feat` keeps its tip and its upstream.
#[test]
fn switch_to_an_existing_branch_git_refuses_leaves_the_branch_as_it_was() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    let tip = commit_adding_a_file(&f);
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "feat",
            "refs/remotes/upstream/feat",
        ],
    );
    git_ok(&f.repo, &["update-ref", "refs/heads/feat", &tip]);
    let wt = detached_wt(&env, &f, "t41-21");
    std::fs::write(wt.join("t41.txt"), "mine\n").unwrap();

    space::core::workspace::switch_worktree_branch(&wt, "upstream/feat", false)
        .expect_err("feat's tree would overwrite the untracked t41.txt");

    assert_eq!(
        git_ok(&f.repo, &["rev-parse", "refs/heads/feat"]).trim(),
        tip,
        "feat keeps its tip"
    );
    assert_eq!(upstream_of(&f.repo, "feat"), "refs/remotes/upstream/feat");
    assert!(is_detached(&wt), "the worktree has not moved");
}

/// T22. A branch typed as new starts from HEAD and tracks nothing, whatever
/// `branch.autoSetupMerge` says. On a HEAD that tracks `upstream/feat`,
/// master's `git switch -c` copied that upstream under `inherit` and
/// tracked `feat` itself under `always` (probed on git 2.50.1).
#[test]
fn switch_to_a_new_branch_tracks_nothing_whatever_auto_setup_merge_says() {
    let env = common::TestEnv::new();
    let f = two_remote_repo(&env);
    git_ok(
        &f.repo,
        &[
            "branch",
            "-q",
            "--track",
            "feat",
            "refs/remotes/upstream/feat",
        ],
    );
    let wt = detached_wt(&env, &f, "t41-22");

    for (mode, name) in [("inherit", "t41-inherit"), ("always", "t41-always")] {
        git_ok(&f.repo, &["config", "branch.autoSetupMerge", mode]);
        git_ok(&wt, &["switch", "-q", "feat"]);
        space::core::workspace::switch_worktree_branch(&wt, name, true)
            .unwrap_or_else(|e| panic!("{} under {}: {}", name, mode, e));
        assert_eq!(head_symref(&wt), format!("refs/heads/{}", name));
        assert_eq!(
            upstream_of(&f.repo, name),
            "",
            "{} tracks nothing under autoSetupMerge {}",
            name,
            mode
        );
    }
}
