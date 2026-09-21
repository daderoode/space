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
/// tree`. git2's `is_worktree` is what `is_worktree_of` uses for this exact
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

/// Skeptical review, pass 2: the orphan arm deleted a directory whenever its
/// admin directory failed to resolve, for any reason. This needs no unusual
/// input at all: a worktree copied beside its original shares the original's
/// `.git` file, so removing the original destroyed the admin directory that
/// the copy also named, and the copy was reclassified from worktree to
/// orphan mid-run and deleted with its uncommitted work. Classification is
/// one pass now, and acting on it is the next.
#[test]
fn remove_workspace_keeps_a_worktree_copied_beside_its_original() {
    let env = common::TestEnv::new();
    let repo = env.create_repo("a-repo");
    let original = worktree_in_space(&env, &repo, "copy-ws");
    let copy = env.workspaces_dir.join("copy-ws").join("z-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(original.join(".git"), copy.join(".git")).unwrap();
    std::fs::write(copy.join("experiment.txt"), "a day of work").unwrap();

    let err = space::core::workspace::remove_workspace(&env.workspaces_dir, "copy-ws", true)
        .expect_err("the copy is not a worktree git knows, so it is kept");
    assert!(
        err.to_string().contains("z-copy"),
        "the report names it, got {:?}",
        err.to_string()
    );
    // git's own line names the original's admin path, which reads as though
    // it were about some other repo; space says what happened before it.
    assert!(
        err.to_string().contains("removed earlier in this run"),
        "the reason says the copy shared a worktree removed in this run, got {:?}",
        err.to_string()
    );
    // Removing the space again would delete it as an orphan, which is the
    // agreed policy for an orphan, so the user is told before, not after.
    assert!(
        err.to_string().contains("deletes this copy"),
        "and warns that removing the space again deletes it, got {:?}",
        err.to_string()
    );
    assert_eq!(
        std::fs::read_to_string(copy.join("experiment.txt")).unwrap(),
        "a day of work",
        "and the work in it is still there"
    );
    assert!(!original.exists(), "the original was still removed");
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
/// "orphan", which deletes.
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
/// repo. Reading only git's way, without the near-miss check, would delete
/// it as an orphan; trimming instead of reading git's way would delete the
/// worktree whose real path ends in a blank (the test after next).
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
        text.contains("copy of the worktree at") && text.contains("deletes this copy"),
        "it is told what it is and what removing the space again would do, got {:?}",
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
