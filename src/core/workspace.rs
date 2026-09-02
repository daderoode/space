use crate::core::git::{self, RepoStatus};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, serde::Serialize)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
    pub repos: Vec<WorkspaceRepo>,
}

#[derive(Debug, serde::Serialize)]
pub struct WorkspaceRepo {
    pub name: String,
    /// Absolute path to the worktree on disk. Used by the TUI (v0.2.0).
    #[allow(dead_code)]
    pub path: PathBuf,
    pub branch: String,
    pub status: RepoStatus,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug)]
pub enum BranchStrategy {
    /// Create a new branch with this name off the repo's default branch.
    NewBranch(String),
    /// Checkout an existing branch (local or remote-tracking).
    ExistingBranch(String),
    /// Detached HEAD at the default branch.
    DetachedHead,
}

/// List all workspace directories inside `ws_dir`.
pub fn list_workspaces(ws_dir: &Path) -> Result<Vec<Workspace>> {
    let mut workspaces = Vec::new();
    if !ws_dir.exists() {
        return Ok(workspaces);
    }
    for entry in std::fs::read_dir(ws_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            workspaces.push(Workspace {
                name,
                path,
                repos: vec![],
            });
        }
    }
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workspaces)
}

/// Return lightweight repo stubs for a workspace without opening any git repos.
/// Used to populate the repos pane immediately on navigation while
/// `workspace_detail` loads in the background.
pub fn workspace_repo_skeletons(ws_dir: &Path, name: &str) -> Vec<WorkspaceRepo> {
    let ws_path = ws_dir.join(name);
    let mut repos = Vec::new();
    let Ok(entries) = std::fs::read_dir(&ws_path) else {
        return repos;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let repo_path = entry.path();
        if !repo_path.join(".git").exists() {
            continue;
        }
        let repo_name = entry.file_name().to_string_lossy().to_string();
        repos.push(WorkspaceRepo {
            name: repo_name,
            path: repo_path,
            branch: "...".to_string(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    repos
}

/// Return a workspace with populated repo details (branch, status, ahead/behind).
pub fn workspace_detail(ws_dir: &Path, name: &str) -> Result<Workspace> {
    let t = std::time::Instant::now();
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
        tracing::warn!(kind = "not_found", "workspace_detail failed");
        anyhow::bail!("workspace '{}' not found", name);
    }
    let mut repos = Vec::new();
    for entry in std::fs::read_dir(&ws_path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let repo_path = entry.path();
        let repo_name = entry.file_name().to_string_lossy().to_string();
        if !repo_path.join(".git").exists() {
            continue;
        }
        let (branch, status, ahead, behind) = match git2::Repository::open(&repo_path) {
            Ok(repo) => {
                let branch =
                    git::current_branch_from_repo(&repo).unwrap_or_else(|_| "?".to_string());
                let status = git::repo_status_from_repo(&repo).unwrap_or_default();
                let (ahead, behind) = git::ahead_behind_from_repo(&repo).unwrap_or((0, 0));
                (branch, status, ahead, behind)
            }
            Err(_) => ("?".to_string(), git::RepoStatus::default(), 0, 0),
        };
        repos.push(WorkspaceRepo {
            name: repo_name,
            path: repo_path,
            branch,
            status,
            ahead,
            behind,
        });
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    tracing::info!(
        elapsed_ms = t.elapsed().as_millis() as u64,
        repo_count = repos.len(),
        "workspace_detail completed"
    );
    Ok(Workspace {
        name: name.to_string(),
        path: ws_path,
        repos,
    })
}

/// Create a git worktree for `repo_path` inside `ws_dir/<ws_name>/<repo_name>`.
/// Run a git command, capturing stdout+stderr. On non-zero exit, returns an
/// error that includes the first non-empty line of stderr so the TUI can show
/// the real git message (e.g. "branch already checked out at …").
fn git_worktree_add(args: &[&str], cwd: &Path) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| "failed to spawn git")?;

    if out.status.success() {
        return Ok(());
    }

    // Git writes progress ("Preparing worktree...") and errors ("fatal: ...") to
    // stderr. Prefer the fatal line; fall back to any non-empty line.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .find(|l| l.starts_with("fatal:"))
        .map(|l| l.trim_start_matches("fatal:").trim())
        .or_else(|| {
            stderr.lines().map(|l| l.trim()).find(|l| {
                !l.is_empty()
                    && !l.starts_with("Preparing worktree")
                    && !l.starts_with("HEAD is now")
            })
        })
        .unwrap_or("git worktree add failed");

    anyhow::bail!("{}", msg)
}

/// Helper: run a git command inside `cwd`, capturing stderr.
/// On non-zero exit, returns an error with the first meaningful git error line.
#[allow(dead_code)] // used by switch_worktree_branch; bin crate has private mod core
fn run_git_in(cwd: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| "failed to spawn git")?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .find(|l| l.starts_with("error:") || l.starts_with("fatal:"))
        .and_then(|l| l.split_once(':').map(|(_, r)| r.trim()))
        .or_else(|| stderr.lines().map(|l| l.trim()).find(|l| !l.is_empty()))
        .unwrap_or("git switch failed");
    anyhow::bail!("{}", msg)
}

/// Switch an existing worktree to a different branch.
///
/// - `new_branch = true`:  creates the branch from the current HEAD (`git switch -c <branch>`).
///   This works even from detached HEAD.
/// - `new_branch = false`: checks for a local branch first; if absent, looks for
///   `origin/<branch>` and creates a local tracking branch; if neither, passes through
///   to git (which will error with a clear message).
#[allow(dead_code)] // public API; called from integration tests and future callers
pub fn switch_worktree_branch(wt_path: &Path, branch: &str, new_branch: bool) -> Result<()> {
    if new_branch {
        return run_git_in(wt_path, &["switch", "-c", branch]);
    }

    // Normalize: if the caller passes "origin/<name>" (from the full branch picker),
    // strip the prefix so we check/create the local name and avoid "origin/origin/<name>".
    let (local_name, remote_ref) = if let Some(name) = branch.strip_prefix("origin/") {
        (name, branch.to_string())
    } else {
        (branch, format!("origin/{}", branch))
    };

    // Check local branch (refs/heads/ scopes the lookup to branches only, not tags)
    let local_ref = format!("refs/heads/{}", local_name);
    let local_exists = Command::new("git")
        .args(["rev-parse", "--verify", &local_ref])
        .current_dir(wt_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if local_exists {
        return run_git_in(wt_path, &["switch", "--", local_name]);
    }

    // Check remote branch
    let remote_exists = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(wt_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if remote_exists {
        return run_git_in(wt_path, &["switch", "-c", local_name, &remote_ref]);
    }

    // Let git provide the error message
    run_git_in(wt_path, &["switch", "--", local_name])
}

/// Why a branch that was strictly behind `origin/<name>` was not fast-forwarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// git refused because the branch is checked out in the worktree at this path.
    CheckedOutAt(PathBuf),
    /// Any refusal the parser does not recognise; carries git's stderr line verbatim.
    Other(String),
}

/// A branch the sync tried to fast-forward and git refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedBranch {
    pub name: String,
    pub reason: SkipReason,
}

/// How the `git fetch` half of a sync ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Ok,
    /// git exited non-zero or could not be started. `exit_code` is `None` when
    /// git was killed by a signal or never ran; `stderr` is everything it wrote.
    Failed {
        exit_code: Option<i32>,
        stderr: String,
    },
    /// The wall-clock limit expired and git's whole process group was stopped.
    /// `stderr` is whatever arrived before the kill, usually nothing.
    TimedOut {
        after: Duration,
        stderr: String,
    },
}

/// The per-repo result inside a sync report (glossary: sync outcome).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    pub fetch: FetchOutcome,
    /// Branches fast-forwarded, in the order they were tried.
    pub forwarded: Vec<String>,
    /// Branches that were behind but git refused to move, with the reason.
    pub skipped: Vec<SkippedBranch>,
}

impl SyncOutcome {
    pub fn fetch_ok(&self) -> bool {
        matches!(self.fetch, FetchOutcome::Ok)
    }

    fn without_branch_work(fetch: FetchOutcome) -> Self {
        Self {
            fetch,
            forwarded: vec![],
            skipped: vec![],
        }
    }
}

/// Wall-clock limit on the fetch of a sync. Fixed in Wave 1; the fast-forward
/// calls are local and run without a limit.
pub const SYNC_FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a timed-out fetch gets to clean up after SIGTERM before SIGKILL.
const SYNC_KILL_GRACE: Duration = Duration::from_secs(2);
const SYNC_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long to wait for git's stderr pipe to close after git itself exited.
/// A helper that outlived git and still holds the pipe must not stall the
/// sync: after this the captured text is used as is.
const SYNC_READER_GRACE: Duration = Duration::from_secs(1);

/// Fetch from `origin` and fast-forward all local branches that are strictly
/// behind their `origin/<branch>` ref (0 ahead, N behind); this assumes a single
/// remote named `origin` rather than each branch's configured upstream. Branches
/// with local commits ahead, diverged, or currently checked out are left
/// untouched and the refusals are reported as skips.
///
/// The fetch runs under the unattended-run policy (see `fetch_origin_unattended`)
/// with the fixed `SYNC_FETCH_TIMEOUT`. When it does not succeed the outcome
/// carries the failure and no branch work is attempted; the caller continues
/// with local refs.
#[allow(dead_code)] // public API; the TUI worker calls sync_repo_cancellable
pub fn sync_repo(repo_path: &Path) -> SyncOutcome {
    sync_repo_with_timeout(repo_path, SYNC_FETCH_TIMEOUT)
}

/// `sync_repo` with an explicit fetch limit. The limit is a parameter so tests
/// can use a short one; the user-facing value is `SYNC_FETCH_TIMEOUT`.
#[allow(dead_code)] // public API; the TUI worker calls sync_repo_cancellable
pub fn sync_repo_with_timeout(repo_path: &Path, timeout: Duration) -> SyncOutcome {
    sync_repo_cancellable(repo_path, timeout, &AtomicBool::new(false))
}

/// `sync_repo_with_timeout` that stops before every git call once `cancel` is
/// set: the in-flight call runs to completion, nothing further is started.
/// The outcome returned after a cancellation is partial and callers that
/// cancelled should discard it.
pub fn sync_repo_cancellable(
    repo_path: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> SyncOutcome {
    if cancel.load(Ordering::Relaxed) {
        return SyncOutcome::without_branch_work(FetchOutcome::Failed {
            exit_code: None,
            stderr: "sync cancelled".to_string(),
        });
    }
    let fetch = fetch_origin_unattended(repo_path, timeout);
    if fetch != FetchOutcome::Ok {
        return SyncOutcome::without_branch_work(fetch);
    }

    let mut forwarded = vec![];
    let mut skipped = vec![];
    for branch in git::branches_behind_upstream(repo_path) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let remote_ref = format!("origin/{}", branch);
        // `LC_ALL=C` pins git's output language so `parse_skip_reason` sees
        // the English refusal; a localized git would turn every skip into
        // `Other`.
        let out = Command::new("git")
            .args(["branch", "-f", &branch, &remote_ref])
            .env("LC_ALL", "C")
            .current_dir(repo_path)
            .stdin(Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => forwarded.push(branch),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                skipped.push(SkippedBranch {
                    name: branch,
                    reason: parse_skip_reason(&stderr),
                });
            }
            Err(e) => skipped.push(SkippedBranch {
                name: branch,
                reason: SkipReason::Other(format!("failed to run git: {}", e)),
            }),
        }
    }

    SyncOutcome {
        fetch: FetchOutcome::Ok,
        forwarded,
        skipped,
    }
}

/// Best-effort parse of git's refusal to `branch -f`. Recognises
/// `fatal: cannot force update the branch '<b>' used by worktree at '<path>'`
/// (git 2.42 and later) and the `checked out at '<path>'` wording of git 2.38
/// to 2.41; anything else, including the pre-2.38 `Cannot force update the
/// current branch.`, is kept verbatim so nothing is swallowed. The caller
/// runs git with `LC_ALL=C` so the wording is not localized.
fn parse_skip_reason(stderr: &str) -> SkipReason {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("fatal:") || l.starts_with("error:"))
        .or_else(|| stderr.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("git branch -f failed");
    for marker in ["used by worktree at '", "checked out at '"] {
        if let Some((_, rest)) = line.split_once(marker) {
            if let Some(path) = rest.strip_suffix('\'') {
                return SkipReason::CheckedOutAt(PathBuf::from(path));
            }
        }
    }
    SkipReason::Other(line.to_string())
}

/// Run `git fetch --quiet origin` under the unattended-run policy (see
/// `run_unattended`). `GIT_TERMINAL_PROMPT=0` makes the https refusal read
/// "terminal prompts disabled". `GIT_SSH_COMMAND` is deliberately not set so
/// the user's own ssh configuration applies.
fn fetch_origin_unattended(repo_path: &Path, timeout: Duration) -> FetchOutcome {
    let mut cmd = Command::new("git");
    cmd.args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0");
    match run_unattended(cmd, timeout) {
        Unattended::Exited { status, stderr } => {
            if status.success() {
                FetchOutcome::Ok
            } else {
                FetchOutcome::Failed {
                    exit_code: status.code(),
                    stderr,
                }
            }
        }
        Unattended::TimedOut { stderr } => FetchOutcome::TimedOut {
            after: timeout,
            stderr,
        },
        Unattended::SpawnFailed(e) => FetchOutcome::Failed {
            exit_code: None,
            stderr: format!("failed to spawn git: {}", e),
        },
        Unattended::WaitFailed { stderr } => FetchOutcome::Failed {
            exit_code: None,
            stderr: format!("could not wait for git\n{}", stderr),
        },
    }
}

/// How a child run by `run_unattended` ended. `stderr` is what the child
/// wrote before it ended (or before it was stopped).
#[derive(Debug)]
enum Unattended {
    /// The child exited on its own, within the limit or during the grace
    /// after SIGTERM; a success status here means the work completed.
    Exited {
        status: ExitStatus,
        stderr: String,
    },
    /// The limit passed and the child did not exit cleanly afterwards.
    TimedOut {
        stderr: String,
    },
    SpawnFailed(std::io::Error),
    /// `try_wait` failed (the child was reaped elsewhere): the child was no
    /// longer ours to signal, so its group was not killed.
    WaitFailed {
        stderr: String,
    },
}

/// Run `cmd` under the unattended-run policy: the child gets its own session
/// (so no controlling terminal: prompts for credentials, passphrases and host
/// keys fail instead of waiting), stdin is null, stdout and stderr are
/// captured. The session is also the process group the timeout stops: SIGTERM
/// to the group (git and its ssh or https helper), a short grace so git can
/// drop its lockfiles, then SIGKILL to whatever is left. A child that exits
/// with success during that grace still counts as `Exited`, so work that
/// completed at the deadline is not thrown away.
///
/// The stderr reader is joined with a bound on every path, so a helper that
/// keeps the pipe open never stalls the caller.
fn run_unattended(mut cmd: Command, timeout: Duration) -> Unattended {
    use std::os::unix::process::CommandExt;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setsid(2) is async-signal-safe and touches no memory shared with
    // the parent, which is all `pre_exec` requires of the closure.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Unattended::SpawnFailed(e),
    };
    let pgid = child.id() as libc::pid_t;
    let (stderr, stderr_reader) = capture_in_background(child.stderr.take());
    let (_stdout, _) = capture_in_background(child.stdout.take());

    match wait_until(&mut child, Instant::now() + timeout) {
        Wait::Exited(status) => {
            // The child has exited, so the pipe normally reaches end-of-file
            // at once: give the reader a moment to copy the last write, but
            // never wait on a helper that outlived the child.
            join_bounded(&stderr_reader, SYNC_READER_GRACE);
            Unattended::Exited {
                status,
                stderr: snapshot(&stderr),
            }
        }
        Wait::Timeout => {
            let status = stop_process_group(&mut child, pgid);
            // Nothing in the group survives the SIGKILL, so the pipe closes
            // and the reader finishes; the bound is a backstop.
            join_bounded(&stderr_reader, SYNC_READER_GRACE);
            let stderr = snapshot(&stderr);
            match status {
                Some(status) if status.success() => Unattended::Exited { status, stderr },
                _ => Unattended::TimedOut { stderr },
            }
        }
        Wait::Error => Unattended::WaitFailed {
            stderr: snapshot(&stderr),
        },
    }
}

/// Wait up to `limit` for a reader thread to reach end-of-file.
fn join_bounded(reader: &std::thread::JoinHandle<()>, limit: Duration) {
    let deadline = Instant::now() + limit;
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(SYNC_POLL_INTERVAL);
    }
}

/// Drain a child pipe on a thread into a shared buffer, so the child never
/// blocks on a full pipe and a snapshot is available even when the reader has
/// not reached end-of-file (the timed-out path). Join the handle before
/// reading the buffer when the pipe is known to be closed.
fn capture_in_background(
    pipe: Option<impl Read + Send + 'static>,
) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let reader = std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = [0u8; 4096];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => sink
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend_from_slice(&chunk[..n]),
                }
            }
        }
    });
    (buf, reader)
}

fn snapshot(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    let bytes = buf.lock().unwrap_or_else(|e| e.into_inner());
    String::from_utf8_lossy(&bytes).into_owned()
}

/// How a bounded wait for the child ended.
enum Wait {
    Exited(ExitStatus),
    Timeout,
    /// `try_wait` failed (the child was reaped elsewhere): the child is no
    /// longer ours to signal, so callers must not kill its group.
    Error,
}

/// Poll the child until it exits, `deadline` passes, or waiting fails.
fn wait_until(child: &mut Child, deadline: Instant) -> Wait {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Wait::Exited(status),
            Ok(None) => {}
            Err(_) => return Wait::Error,
        }
        let now = Instant::now();
        if now >= deadline {
            return Wait::Timeout;
        }
        std::thread::sleep(SYNC_POLL_INTERVAL.min(deadline - now));
    }
}

/// SIGTERM the child's process group, give the leader up to `SYNC_KILL_GRACE`
/// to exit, then SIGKILL the whole group regardless: a helper in the session
/// that ignored SIGTERM (askpass, a credential helper) must not outlive git
/// holding the remote connection and the stderr pipe. The leader is only
/// reaped after the SIGKILL, so the group id is guaranteed to still be ours
/// when it is sent. Returns the leader's exit status once reaped, which is
/// its own status when it exited during the grace.
fn stop_process_group(child: &mut Child, pgid: libc::pid_t) -> Option<ExitStatus> {
    let pid = child.id() as libc::pid_t;
    // SAFETY: killpg on the group we created with setsid; the leader is our
    // unreaped child (only `Wait::Timeout` reaches here), so the group id
    // cannot have been recycled.
    unsafe {
        libc::killpg(pgid, libc::SIGTERM);
    }
    let deadline = Instant::now() + SYNC_KILL_GRACE;
    while !has_exited_unreaped(pid) && Instant::now() < deadline {
        std::thread::sleep(SYNC_POLL_INTERVAL);
    }
    // SAFETY: as above; the leader is still unreaped (`has_exited_unreaped`
    // never reaps), so the group id is still ours. ESRCH, when everything
    // already left, is fine.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
    child.wait().ok()
}

/// Whether `pid`, an unreaped child of ours, has exited. Uses `waitid` with
/// `WNOWAIT` so the zombie stays in place for `Child::wait`; `kill(pid, 0)`
/// would not do, as it succeeds for a zombie too. A failed `waitid` (the
/// child is not ours to wait for) reports `true` so the caller stops polling.
fn has_exited_unreaped(pid: libc::pid_t) -> bool {
    // SAFETY: `siginfo_t` is plain data for which all-zero is a valid value,
    // and waitid only writes into it. With WNOHANG and no state change the
    // struct is left with `si_signo == 0`; a reported child sets SIGCHLD.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let rc = libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        rc != 0 || info.si_signo == libc::SIGCHLD
    }
}

/// The classification of what `pull_repo` did (or why it did nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
    /// `git fetch` failed (offline / no remote).
    FetchFailed,
    /// No current branch (detached HEAD) — nothing to pull onto.
    DetachedHead,
    /// The current branch has no `origin/<branch>` upstream to pull from.
    NoUpstream,
    /// Local already matches upstream (0 ahead, 0 behind).
    UpToDate,
    /// Local is only ahead of upstream — nothing to pull.
    Ahead,
    /// Local was behind and was fast-forwarded to the upstream.
    FastForwarded,
    /// Local and upstream diverged and were merged cleanly (merge commit).
    Merged,
    /// Local and upstream diverged with conflicts; the merge was aborted and
    /// the worktree restored to its pre-merge state.
    Conflicted,
    /// The pull could not be applied (e.g. a blocked fast-forward); git's
    /// error output is in the message.
    Failed,
}

/// Result of pulling a single repo: the outcome plus a human-readable message.
pub struct PullResult {
    pub outcome: PullOutcome,
    pub message: String,
}

impl PullResult {
    /// Whether the pull left the repo in a good state: true for up to date,
    /// ahead, fast-forwarded, or merged; false for fetch failures, detached
    /// HEAD, no upstream, conflicts, and failed fast-forwards.
    pub fn success(&self) -> bool {
        matches!(
            self.outcome,
            PullOutcome::UpToDate
                | PullOutcome::Ahead
                | PullOutcome::FastForwarded
                | PullOutcome::Merged
        )
    }
}

/// Pull the current branch of `repo_path` from its `origin/<branch>` upstream.
///
/// Fetches first, then classifies the branch state and acts:
/// - behind only  → fast-forward (`FastForwarded`)
/// - diverged     → real merge; on conflict, `git merge --abort` (`Merged`/`Conflicted`)
/// - up to date / only ahead → no-op (`UpToDate`/`Ahead`)
/// - detached HEAD / no upstream / fetch failure → report without acting.
pub fn pull_repo(repo_path: &Path) -> PullResult {
    // Detached HEAD is checked BEFORE the fetch: a detached-HEAD pull must
    // report without acting at all (not even mutating remote-tracking refs).
    let branch = match current_branch_name(repo_path) {
        Some(b) => b,
        None => {
            return PullResult {
                outcome: PullOutcome::DetachedHead,
                message: "Detached HEAD: no branch to pull.".to_string(),
            };
        }
    };

    // `.output()` (not `.status()`): capture stderr both to surface the real
    // failure cause (auth, DNS, missing remote) and to keep git from writing
    // to the inherited stderr, which would scribble over the raw-mode TUI.
    let fetch = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .output();
    let fetch_failed_message = match &fetch {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Some(if stderr.is_empty() {
                "Fetch failed: no remote or offline.".to_string()
            } else {
                format!("Fetch failed: {}", stderr)
            })
        }
        Err(err) => Some(format!("Fetch failed: {}", err)),
    };
    if let Some(message) = fetch_failed_message {
        return PullResult {
            outcome: PullOutcome::FetchFailed,
            message,
        };
    }

    // No `origin/<branch>` upstream to pull from (checked post-fetch so the
    // remote-tracking refs are fresh).
    let remote_exists = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{}", branch),
        ])
        .current_dir(repo_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !remote_exists {
        return PullResult {
            outcome: PullOutcome::NoUpstream,
            message: format!("{} has no upstream (origin/{}) to pull.", branch, branch),
        };
    }

    let (ahead, behind) = git::ahead_behind(repo_path).unwrap_or((0, 0));

    if behind > 0 && ahead == 0 {
        let remote_ref = format!("origin/{}", branch);
        let output = Command::new("git")
            .args(["merge", "--ff-only", &remote_ref])
            .current_dir(repo_path)
            .output();
        match output {
            Ok(o) if o.status.success() => {
                return PullResult {
                    outcome: PullOutcome::FastForwarded,
                    message: format!(
                        "Fast-forwarded {} to {} ({} commit(s)).",
                        branch, remote_ref, behind
                    ),
                };
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let detail = stderr.trim();
                let detail = if detail.is_empty() {
                    "git reported no error output"
                } else {
                    detail
                };
                return PullResult {
                    outcome: PullOutcome::Failed,
                    message: format!(
                        "Fast-forward of {} to {} failed: {}",
                        branch, remote_ref, detail
                    ),
                };
            }
            Err(err) => {
                return PullResult {
                    outcome: PullOutcome::Failed,
                    message: format!(
                        "Fast-forward of {} to {} failed: {}",
                        branch, remote_ref, err
                    ),
                };
            }
        }
    }

    if ahead > 0 && behind > 0 {
        let remote_ref = format!("origin/{}", branch);
        let merged = Command::new("git")
            .args(["merge", "--no-edit", &remote_ref])
            .current_dir(repo_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if merged {
            return PullResult {
                outcome: PullOutcome::Merged,
                message: format!(
                    "Merged {} into {} ({} ahead, {} behind).",
                    remote_ref, branch, ahead, behind
                ),
            };
        }
        // Merge left conflicts: restore the pre-merge worktree so `space` never
        // leaves the repo half-merged.
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo_path)
            .status();
        return PullResult {
            outcome: PullOutcome::Conflicted,
            message: format!(
                "Merge of {} into {} conflicted; aborted and restored a clean worktree.",
                remote_ref, branch
            ),
        };
    }

    if ahead > 0 && behind == 0 {
        return PullResult {
            outcome: PullOutcome::Ahead,
            message: format!(
                "{} is {} commit(s) ahead of upstream; nothing to pull.",
                branch, ahead
            ),
        };
    }

    PullResult {
        outcome: PullOutcome::UpToDate,
        message: format!("{} is already up to date.", branch),
    }
}

/// Result of pushing the current branch: whether git accepted the push, plus a
/// human-readable message. On rejection the message carries git's own output so
/// the non-fast-forward reason surfaces verbatim.
pub struct PushResult {
    pub success: bool,
    pub message: String,
}

/// Push the current branch of `repo_path` to `origin`.
///
/// - `set_upstream == true`  → `git push -u origin <branch>` (first publish of a
///   branch with no upstream; also records the tracking ref).
/// - `set_upstream == false` → `git push` (branch already has an upstream).
///
/// Never forces. A rejected push (remote ahead / non-fast-forward) returns
/// `success == false` with git's rejection text in `message`, so callers can
/// surface why the push was refused (typically: pull first).
pub fn push_repo(repo_path: &Path, set_upstream: bool) -> PushResult {
    let branch = match current_branch_name(repo_path) {
        Some(b) => b,
        None => {
            return PushResult {
                success: false,
                message: "Detached HEAD: no branch to push.".to_string(),
            };
        }
    };

    let args: Vec<String> = if set_upstream {
        vec![
            "push".to_string(),
            "-u".to_string(),
            "origin".to_string(),
            branch.clone(),
        ]
    } else {
        vec!["push".to_string()]
    };

    let out = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) => {
            // git writes both its success summary and rejection details to stderr.
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let message = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                format!("Pushed {}.", branch)
            };
            PushResult {
                success: o.status.success(),
                message,
            }
        }
        Err(e) => PushResult {
            success: false,
            message: format!("Failed to run git push: {}", e),
        },
    }
}

/// Result of committing staged changes: whether git accepted the commit, plus a
/// human-readable summary (git's own stdout/stderr) callers can surface
/// verbatim on both success and failure.
pub struct CommitResult {
    pub success: bool,
    pub message: String,
}

/// Commit the currently staged changes in `repo_path` with `message`.
///
/// Shells out to `git commit -m <message>`, letting git build the tree, resolve
/// the parent (or create the initial commit on an unborn HEAD), and apply the
/// user's signature/gpg settings. A commit with nothing staged returns
/// `success == false` with git's "nothing to commit" text in `message`.
pub fn commit_repo(repo_path: &Path, message: &str) -> CommitResult {
    let out = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) => {
            // git writes the commit summary to stdout; refusals ("nothing to
            // commit") also land on stdout, so prefer it and fall back to stderr.
            let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let message = if !stdout.is_empty() {
                stdout
            } else if !stderr.is_empty() {
                stderr
            } else {
                "Committed.".to_string()
            };
            CommitResult {
                success: o.status.success(),
                message,
            }
        }
        Err(e) => CommitResult {
            success: false,
            message: format!("Failed to run git commit: {}", e),
        },
    }
}

/// The classification of what `rebase_repo` did (or why it did nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// HEAD is detached — no branch to rebase (defensive; the TUI pre-flight
    /// already blocks this).
    DetachedHead,
    /// The branch was replayed onto the target.
    Rebased,
    /// The branch is already up to date with the target (nothing replayed).
    UpToDate,
    /// The rebase hit conflicts; it was aborted and the branch restored.
    Conflicted,
    /// The rebase could not start or apply (e.g. an unknown target ref); git's
    /// error output is in the message.
    Failed,
}

/// Result of rebasing a single repo: the outcome plus a human-readable message.
pub struct RebaseResult {
    pub outcome: RebaseOutcome,
    pub message: String,
}

impl RebaseResult {
    /// Whether the rebase left the branch in the intended state: true for a
    /// completed rebase or an already-up-to-date branch; false for detached
    /// HEAD, conflicts (aborted), and failures.
    pub fn success(&self) -> bool {
        matches!(
            self.outcome,
            RebaseOutcome::Rebased | RebaseOutcome::UpToDate
        )
    }
}

/// Rebase the current branch of `repo_path` onto `onto`, replaying local commits
/// on top of the target.
///
/// Fetches `origin` best-effort first (non-fatal) so `origin/*` targets are
/// current, then runs `git rebase <onto>`. On conflict the rebase is aborted
/// (`git rebase --abort`) and the branch restored, so `space` never leaves a
/// worktree mid-rebase.
///
/// Conflict vs. immediate failure is classified by the abort result rather than
/// probing `.git/rebase-merge` (which is fragile under worktrees): if
/// `git rebase --abort` succeeds, a rebase was in progress and hit conflicts;
/// if it fails ("no rebase in progress"), the rebase never started (e.g. an
/// unknown target) and the failure is reported verbatim.
pub fn rebase_repo(repo_path: &Path, onto: &str) -> RebaseResult {
    let branch = match current_branch_name(repo_path) {
        Some(b) => b,
        None => {
            return RebaseResult {
                outcome: RebaseOutcome::DetachedHead,
                message: "Detached HEAD: no branch to rebase.".to_string(),
            };
        }
    };

    // Reject a target that starts with '-': `git rebase -foo` would parse the
    // target as an option, not a revspec (argument injection). The TUI picker
    // only yields real branch names, but a plumbing-created ref could begin
    // with a dash, so guard the boundary rather than trust the caller.
    if onto.starts_with('-') {
        return RebaseResult {
            outcome: RebaseOutcome::Failed,
            message: format!(
                "Refusing to rebase onto '{}': a target may not begin with '-'.",
                onto
            ),
        };
    }

    // Best-effort refresh so a rebase onto `origin/<x>` replays onto the latest
    // remote state. A fetch failure (offline / no remote) is non-fatal: the
    // rebase can still proceed onto a local target. `GIT_TERMINAL_PROMPT=0`
    // keeps this optional fetch from opening a credential prompt on /dev/tty,
    // which would hang or scribble over the raw-mode TUI.
    let _ = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(repo_path)
        .output();

    // `LC_ALL=C` pins git's output language so the up-to-date classification
    // below (`stdout.contains("is up to date")`) is stable: git localizes that
    // message via gettext, and a non-English LANG would misclassify UpToDate
    // as Rebased.
    let out = Command::new("git")
        .args(["rebase", onto])
        .env("LC_ALL", "C")
        .current_dir(repo_path)
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains("is up to date") {
                RebaseResult {
                    outcome: RebaseOutcome::UpToDate,
                    message: format!("{} is already up to date with {}.", branch, onto),
                }
            } else {
                RebaseResult {
                    outcome: RebaseOutcome::Rebased,
                    message: format!("Rebased {} onto {}.", branch, onto),
                }
            }
        }
        Ok(o) => {
            // The rebase either hit a conflict mid-replay or failed to start.
            // `git rebase --abort` succeeds only when a rebase is in progress,
            // so its result cleanly distinguishes the two without touching the
            // worktree's git-dir layout.
            let aborted = Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(repo_path)
                .output()
                .map(|a| a.status.success())
                .unwrap_or(false);
            if aborted {
                RebaseResult {
                    outcome: RebaseOutcome::Conflicted,
                    // Two lines: what happened, then the next step (the worker
                    // streams each line separately in the Running overlay).
                    message: format!(
                        "Rebase of {} onto {} conflicted; aborted and restored the branch.\n\
                         Resolve the conflicts manually: run 'git rebase {}' in a terminal.",
                        branch, onto, onto
                    ),
                }
            } else {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let stdout = String::from_utf8_lossy(&o.stdout);
                let detail = stderr.trim();
                let detail = if detail.is_empty() {
                    stdout.trim()
                } else {
                    detail
                };
                let detail = if detail.is_empty() {
                    "git reported no error output"
                } else {
                    detail
                };
                RebaseResult {
                    outcome: RebaseOutcome::Failed,
                    message: format!("Rebase of {} onto {} failed: {}", branch, onto, detail),
                }
            }
        }
        Err(err) => RebaseResult {
            outcome: RebaseOutcome::Failed,
            message: format!("Failed to run git rebase: {}", err),
        },
    }
}

/// The current checked-out branch name, or `None` when HEAD is detached (no
/// symbolic branch ref).
fn current_branch_name(repo_path: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Returns the path to the created worktree.
pub fn create_worktree(
    repo_path: &Path,
    ws_dir: &Path,
    ws_name: &str,
    strategy: &BranchStrategy,
) -> Result<PathBuf> {
    let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
    let wt_path = ws_dir.join(ws_name).join(repo_name.as_ref());

    std::fs::create_dir_all(wt_path.parent().unwrap())?;

    let base_branch = git::detect_base_branch(repo_path);
    let wt = wt_path.to_string_lossy();

    // Auto-fetch — ignore errors for offline use
    let _ = Command::new("git")
        .args(["fetch", "--quiet", "origin"])
        .current_dir(repo_path)
        .status();

    match strategy {
        BranchStrategy::NewBranch(branch_name) => {
            // 1. Local branch exists?
            let local_exists = Command::new("git")
                .args(["rev-parse", "--verify", branch_name])
                .current_dir(repo_path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            // 2. Remote branch exists?
            let remote_ref = format!("origin/{}", branch_name);
            let remote_exists = Command::new("git")
                .args(["rev-parse", "--verify", &remote_ref])
                .current_dir(repo_path)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if local_exists {
                git_worktree_add(&["worktree", "add", &wt, branch_name], repo_path)?;
            } else if remote_exists {
                git_worktree_add(
                    &[
                        "worktree",
                        "add",
                        "--track",
                        "-b",
                        branch_name,
                        &wt,
                        &remote_ref,
                    ],
                    repo_path,
                )?;
            } else {
                // Prefer origin/<base> so the new branch starts at the remote tip rather than
                // a potentially stale local ref. Trade-off: if <base> has unpushed local
                // commits they are NOT included in the new worktree. That is intentional —
                // the sync step guarantees origin/<base> is the freshest shared state.
                // Fall back to local only if the remote ref doesn't exist (offline / no remote).
                let origin_base = format!("origin/{}", base_branch);
                let origin_base_exists = Command::new("git")
                    .args(["rev-parse", "--verify", &origin_base])
                    .current_dir(repo_path)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                let start_point: &str = if origin_base_exists {
                    &origin_base
                } else {
                    &base_branch
                };
                git_worktree_add(
                    &["worktree", "add", "-b", branch_name, &wt, start_point],
                    repo_path,
                )?;
            }
        }

        BranchStrategy::ExistingBranch(branch_name) => {
            let local = branch_name.strip_prefix("origin/").unwrap_or(branch_name);
            if branch_name.starts_with("origin/") {
                git_worktree_add(
                    &["worktree", "add", "--track", "-b", local, &wt, branch_name],
                    repo_path,
                )?;
            } else {
                git_worktree_add(&["worktree", "add", &wt, local], repo_path)?;
            }
        }

        BranchStrategy::DetachedHead => {
            git_worktree_add(
                &["worktree", "add", "--detach", &wt, &base_branch],
                repo_path,
            )?;
        }
    }

    Ok(wt_path)
}

/// Remove a workspace: call `git worktree remove` for each repo worktree,
/// then delete the directory.
pub fn remove_workspace(ws_dir: &Path, name: &str, force: bool) -> Result<()> {
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
        anyhow::bail!("workspace '{}' not found", name);
    }

    for entry in std::fs::read_dir(&ws_path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let wt_path = entry.path();
        if !wt_path.join(".git").exists() {
            continue;
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        let wt_str = wt_path.to_string_lossy().to_string();
        args.push(&wt_str);

        if let Some(main_repo) = find_main_repo(&wt_path) {
            Command::new("git")
                .args(&args)
                .current_dir(&main_repo)
                .status()
                .ok();
        }
    }

    std::fs::remove_dir_all(&ws_path)
        .with_context(|| format!("removing workspace directory {}", ws_path.display()))?;
    Ok(())
}

/// Given a worktree path, read its `.git` file to find the main repo root.
fn find_main_repo(wt_path: &Path) -> Option<PathBuf> {
    let git_file = wt_path.join(".git");
    if git_file.is_file() {
        let content = std::fs::read_to_string(&git_file).ok()?;
        let gitdir = content.trim().strip_prefix("gitdir: ")?;
        let path = PathBuf::from(gitdir);
        path.ancestors()
            .find(|p| p.join("config").exists() && p.ends_with(".git"))
            .and_then(|p| p.parent())
            .map(PathBuf::from)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;

    fn git(args: &[&str], dir: &Path) {
        let out = Cmd::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed:\n{}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_setup(dir: &Path) {
        git(&["config", "user.email", "t@local"], dir);
        git(&["config", "user.name", "T"], dir);
        git(&["config", "commit.gpgsign", "false"], dir);
    }

    fn get_sha(dir: &Path, refname: &str) -> String {
        let out = Cmd::new("git")
            .args(["rev-parse", refname])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Returns `(tmp, local_path)` where:
    /// - `local/main` is checked out and 1 behind `origin/main`
    /// - `local/dev` is NOT checked out and 1 behind `origin/dev`
    fn make_behind_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");

        Cmd::new("git")
            .args(["init", "--bare", "-b", "main", "origin.git"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["clone", "origin.git", "local"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&local);

        git(&["commit", "--allow-empty", "-m", "init"], &local);
        git(&["push", "-u", "origin", "main"], &local);

        git(&["checkout", "-b", "dev"], &local);
        git(&["commit", "--allow-empty", "-m", "dev-init"], &local);
        git(&["push", "-u", "origin", "dev"], &local);
        git(&["checkout", "main"], &local);

        let helper = tmp.path().join("helper");
        Cmd::new("git")
            .args(["clone", "origin.git", "helper"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&helper);

        git(&["commit", "--allow-empty", "-m", "main-remote"], &helper);
        git(&["push", "origin", "main"], &helper);

        Cmd::new("git")
            .args(["checkout", "-b", "dev", "origin/dev"])
            .current_dir(&helper)
            .output()
            .unwrap();
        git(&["commit", "--allow-empty", "-m", "dev-remote"], &helper);
        git(&["push", "origin", "dev"], &helper);

        // Do NOT fetch in local — sync_repo's internal fetch handles that
        (tmp, local)
    }

    /// Bare `origin.git` + a `local` clone with a single pushed commit on
    /// `main` (tracked as `origin/main`), containing `base.txt`. `local` is up
    /// to date with `origin/main` on return.
    fn origin_and_local() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("local");
        Cmd::new("git")
            .args(["init", "--bare", "-b", "main", "origin.git"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Cmd::new("git")
            .args(["clone", "origin.git", "local"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(&local);
        std::fs::write(local.join("base.txt"), "base\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "init"], &local);
        git(&["push", "-u", "origin", "main"], &local);
        (tmp, local)
    }

    /// Clone `origin.git` (already created by `origin_and_local`) into a fresh
    /// `helper` worktree on `main`, run `edit` to stage/commit a change, and
    /// push it to `origin/main` — advancing the remote so `local` falls behind.
    fn advance_origin(tmp: &Path, edit: impl FnOnce(&Path)) {
        let helper = tmp.join("helper");
        Cmd::new("git")
            .args(["clone", "origin.git", "helper"])
            .current_dir(tmp)
            .output()
            .unwrap();
        git_setup(&helper);
        edit(&helper);
        git(&["push", "origin", "main"], &helper);
    }

    #[test]
    fn sync_repo_returns_fetch_failed_without_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(tmp.path());
        git(&["commit", "--allow-empty", "-m", "init"], tmp.path());

        let result = sync_repo(tmp.path());
        assert!(!result.fetch_ok(), "fetch must fail when no remote");
        match &result.fetch {
            FetchOutcome::Failed { exit_code, stderr } => {
                assert_eq!(*exit_code, Some(128), "git reports a missing remote as 128");
                assert!(
                    stderr.contains("'origin' does not appear to be a git repository"),
                    "stderr must be captured in full, got: {:?}",
                    stderr
                );
            }
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        }
        assert!(
            result.forwarded.is_empty() && result.skipped.is_empty(),
            "no branch work happens when the fetch fails"
        );
    }

    #[test]
    fn sync_repo_fast_forwards_non_checked_out_branch_behind_remote() {
        let (_tmp, local) = make_behind_repo();

        let sha_before = get_sha(&local, "dev");
        let result = sync_repo(&local);

        assert!(result.fetch_ok(), "fetch must succeed");
        assert_eq!(
            result.forwarded,
            vec!["dev".to_string()],
            "dev must be the only fast-forwarded branch: {:?}",
            result.forwarded
        );

        let sha_after = get_sha(&local, "dev");
        let origin_sha = get_sha(&local, "origin/dev");
        assert_ne!(sha_before, sha_after, "dev must have advanced");
        assert_eq!(
            sha_after, origin_sha,
            "dev must equal origin/dev after fast-forward"
        );
    }

    #[test]
    fn sync_repo_reports_checked_out_branch_as_skipped_with_worktree_path() {
        let (_tmp, local) = make_behind_repo();

        let result = sync_repo(&local);

        assert!(result.fetch_ok(), "fetch must succeed");
        assert!(
            !result.forwarded.contains(&"main".to_string()),
            "main must not be fast-forwarded when checked out: {:?}",
            result.forwarded
        );
        let skip = result
            .skipped
            .iter()
            .find(|s| s.name == "main")
            .unwrap_or_else(|| panic!("main must be reported as skipped: {:?}", result.skipped));
        match &skip.reason {
            SkipReason::CheckedOutAt(path) => assert_eq!(
                path.canonicalize().unwrap(),
                local.canonicalize().unwrap(),
                "the skip must carry the worktree path git named"
            ),
            other => panic!("expected CheckedOutAt, got {:?}", other),
        }
    }

    #[test]
    fn parse_skip_reason_keeps_unrecognised_refusals_verbatim() {
        let reason = parse_skip_reason("fatal: Cannot force update the current branch.\n");
        assert_eq!(
            reason,
            SkipReason::Other("fatal: Cannot force update the current branch.".to_string())
        );
        let reason = parse_skip_reason(
            "fatal: cannot force update the branch 'x' used by worktree at '/w/x'\n",
        );
        assert_eq!(reason, SkipReason::CheckedOutAt(PathBuf::from("/w/x")));
        assert_eq!(
            parse_skip_reason(""),
            SkipReason::Other("git branch -f failed".to_string())
        );
    }

    #[test]
    fn parse_skip_reason_recognises_both_worktree_wordings() {
        // git 2.42 and later.
        let reason = parse_skip_reason(
            "fatal: cannot force update the branch 'main' used by worktree at '/w/main'\n",
        );
        assert_eq!(reason, SkipReason::CheckedOutAt(PathBuf::from("/w/main")));
        // git 2.38 to 2.41.
        let reason = parse_skip_reason(
            "fatal: cannot force update the branch 'main' checked out at '/w/main'\n",
        );
        assert_eq!(reason, SkipReason::CheckedOutAt(PathBuf::from("/w/main")));
    }

    /// A helper inside git's session that ignores SIGTERM (an askpass or
    /// credential helper can) must not outlive the timeout: after the grace the
    /// whole group gets SIGKILL even though git itself left on SIGTERM. The
    /// upload-pack script backgrounds such a helper and records its pid.
    #[test]
    fn sync_repo_timeout_kills_helper_that_ignores_sigterm() {
        let (tmp, local) = make_behind_repo();
        let helper_pidfile = tmp.path().join("helper.pid");
        let script = tmp.path().join("stubborn-upload-pack.sh");
        std::fs::write(
            &script,
            format!(
                "/bin/sh -c 'trap \"\" TERM; echo $$ > \"{}\"; exec sleep 30' &\nexec sleep 30\n",
                helper_pidfile.display()
            ),
        )
        .unwrap();
        let origin_url = format!("file://{}", tmp.path().join("origin.git").display());
        git(&["remote", "set-url", "origin", &origin_url], &local);
        git(
            &[
                "config",
                "remote.origin.uploadpack",
                &format!("/bin/sh {}", script.display()),
            ],
            &local,
        );

        let limit = Duration::from_millis(1000);
        let started = Instant::now();
        let result = sync_repo_with_timeout(&local, limit);
        let elapsed = started.elapsed();

        match &result.fetch {
            FetchOutcome::TimedOut { after, .. } => assert_eq!(*after, limit),
            other => panic!("expected FetchOutcome::TimedOut, got {:?}", other),
        }
        assert!(
            elapsed < Duration::from_secs(5),
            "sync must return shortly after the limit, took {:?}",
            elapsed
        );

        let pid: libc::pid_t = std::fs::read_to_string(&helper_pidfile)
            .expect("the stubborn helper must have started and recorded its pid")
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            // SAFETY: signal 0 only checks for existence; no signal is delivered.
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            if Instant::now() >= deadline {
                // SAFETY: our own test helper; stop it so it does not outlive the test.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                panic!(
                    "helper {} that ignored SIGTERM must be gone after the timeout",
                    pid
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A child that ignores SIGTERM stands in for one that exited just as
    /// the limit passed: its success status must reach the caller as
    /// `Exited`, and the grace must end as soon as it exits rather than
    /// running its full length.
    #[test]
    fn run_unattended_reports_success_for_child_exiting_during_kill_grace() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "trap '' TERM; sleep 0.6; echo late >&2; exit 0"]);

        let started = Instant::now();
        let outcome = run_unattended(cmd, Duration::from_millis(200));
        let elapsed = started.elapsed();

        match &outcome {
            Unattended::Exited { status, stderr } => {
                assert!(
                    status.success(),
                    "status must be the child's own: {:?}",
                    status
                );
                assert_eq!(
                    stderr, "late\n",
                    "stderr written after SIGTERM must be kept"
                );
            }
            other => panic!("expected Unattended::Exited, got {:?}", other),
        }
        assert!(
            elapsed < SYNC_KILL_GRACE,
            "the grace must end when the child exits, took {:?}",
            elapsed
        );
    }

    /// The same child that never exits is still a timeout, and the SIGKILL
    /// after the grace ends it (it ignores SIGTERM, so the grace alone would
    /// leave it running).
    #[test]
    fn run_unattended_times_out_child_that_ignores_sigterm() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "trap '' TERM; sleep 30; exit 0"]);

        let started = Instant::now();
        let outcome = run_unattended(cmd, Duration::from_millis(200));
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, Unattended::TimedOut { .. }),
            "expected Unattended::TimedOut, got {:?}",
            outcome
        );
        assert!(
            elapsed >= SYNC_KILL_GRACE && elapsed < SYNC_KILL_GRACE + Duration::from_secs(2),
            "the child gets the full grace and is then killed, took {:?}",
            elapsed
        );
    }

    /// The fetch must give up after the limit and leave no child behind. The
    /// remote is a `file://` origin whose upload-pack is a script that records
    /// its pid and sleeps, so nothing touches the network. The script runs as
    /// `/bin/sh <script>` rather than as an executable: macOS assesses a
    /// freshly written executable on its first exec, which can take longer
    /// than the limit.
    #[test]
    fn sync_repo_times_out_when_remote_never_answers_and_child_is_gone() {
        let (tmp, local) = make_behind_repo();
        let pidfile = tmp.path().join("upload-pack.pid");
        let script = tmp.path().join("slow-upload-pack.sh");
        std::fs::write(
            &script,
            format!("echo $$ > '{}'\nexec sleep 30\n", pidfile.display()),
        )
        .unwrap();
        let origin_url = format!("file://{}", tmp.path().join("origin.git").display());
        git(&["remote", "set-url", "origin", &origin_url], &local);
        git(
            &[
                "config",
                "remote.origin.uploadpack",
                &format!("/bin/sh {}", script.display()),
            ],
            &local,
        );

        let limit = Duration::from_millis(1000);
        let started = Instant::now();
        let result = sync_repo_with_timeout(&local, limit);
        let elapsed = started.elapsed();

        match &result.fetch {
            FetchOutcome::TimedOut { after, .. } => assert_eq!(*after, limit),
            other => panic!("expected FetchOutcome::TimedOut, got {:?}", other),
        }
        assert!(
            elapsed < Duration::from_secs(5),
            "sync must return shortly after the limit, took {:?}",
            elapsed
        );
        assert!(
            result.forwarded.is_empty() && result.skipped.is_empty(),
            "a timed-out repo never has fast-forwards or skips"
        );

        // The upload-pack helper ran inside git's session; the group kill must
        // have taken it with git. Poll: the kernel reaps the orphan a moment
        // after SIGTERM lands.
        let pid: libc::pid_t = std::fs::read_to_string(&pidfile)
            .expect("the slow upload-pack must have started and recorded its pid")
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            // SAFETY: signal 0 only checks for existence; no signal is delivered.
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "upload-pack child {} must be gone after the timeout",
                pid
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A remote that would prompt for credentials fails at once with git's
    /// own "terminal prompts disabled" text instead of hanging. Needs the
    /// network: skipped when github.com is unreachable or an askpass helper
    /// is configured (it would answer the prompt instead).
    #[test]
    fn sync_repo_reports_refused_https_prompt_as_fetch_failed() {
        use std::net::{TcpStream, ToSocketAddrs};
        let core_askpass = Cmd::new("git")
            .args(["config", "--get", "core.askPass"])
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false);
        if core_askpass
            || std::env::var_os("GIT_ASKPASS").is_some()
            || std::env::var_os("SSH_ASKPASS").is_some()
        {
            eprintln!("skipping: an askpass helper is configured");
            return;
        }
        let reachable = "github.com:443"
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .map(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok())
            .unwrap_or(false);
        if !reachable {
            eprintln!("skipping: github.com is unreachable");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(tmp.path());
        git(
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/space-wayfinder-probe-nonexistent/repo.git",
            ],
            tmp.path(),
        );
        // An empty value resets the helper list, so a stored credential on the
        // machine cannot answer the 401 before the prompt logic runs.
        git(&["config", "credential.helper", ""], tmp.path());

        let result = sync_repo_with_timeout(tmp.path(), Duration::from_secs(20));

        match &result.fetch {
            FetchOutcome::Failed { exit_code, stderr } => {
                assert_eq!(*exit_code, Some(128));
                assert!(
                    stderr.contains("terminal prompts disabled"),
                    "expected git's refused-prompt text, got: {:?}",
                    stderr
                );
            }
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        }
    }

    #[test]
    fn pull_repo_fast_forwards_checked_out_branch_behind_remote() {
        // make_behind_repo leaves `local` on `main`, one commit behind
        // origin/main (0 ahead). pull_repo fetches then fast-forwards.
        let (_tmp, local) = make_behind_repo();

        let sha_before = get_sha(&local, "main");
        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::FastForwarded),
            "behind branch must fast-forward, got {:?}: {}",
            result.outcome,
            result.message
        );
        let sha_after = get_sha(&local, "main");
        let origin_sha = get_sha(&local, "origin/main");
        assert_ne!(sha_before, sha_after, "main must have advanced");
        assert_eq!(
            sha_after, origin_sha,
            "main must equal origin/main after fast-forward"
        );
    }

    #[test]
    fn pull_repo_reports_fetch_failed_without_remote() {
        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        git_setup(tmp.path());
        git(&["commit", "--allow-empty", "-m", "init"], tmp.path());

        let result = pull_repo(tmp.path());
        assert!(
            result.message.contains("origin"),
            "the fetch failure must surface git's actual error (naming the \
             missing remote), got: {}",
            result.message
        );
        assert!(
            matches!(result.outcome, PullOutcome::FetchFailed),
            "no remote must yield FetchFailed, got {:?}: {}",
            result.outcome,
            result.message
        );
    }

    #[test]
    fn pull_repo_reports_up_to_date_when_synced() {
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::UpToDate),
            "synced branch must report UpToDate, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "up-to-date pull must not move main"
        );
    }

    #[test]
    fn pull_repo_reports_ahead_when_local_has_unpushed_commit() {
        let (_tmp, local) = origin_and_local();
        let init_sha = get_sha(&local, "main");

        // Local advances but does not push — 1 ahead, 0 behind.
        git(&["commit", "--allow-empty", "-m", "local-only"], &local);
        let ahead_sha = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Ahead),
            "ahead-only branch must report Ahead, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            ahead_sha,
            get_sha(&local, "main"),
            "ahead-only pull must not move main"
        );
        assert_eq!(
            init_sha,
            get_sha(&local, "origin/main"),
            "ahead-only pull must not push (origin/main unchanged)"
        );
    }

    #[test]
    fn pull_repo_merges_diverged_non_conflicting_changes() {
        let (tmp, local) = origin_and_local();

        // Remote advances main with a new file the local side never touches.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("helper.txt"), "helper\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-side"], helper);
        });

        // Local advances main with a different, non-conflicting file.
        std::fs::write(local.join("local.txt"), "local\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-side"], &local);

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Merged),
            "clean diverge must merge, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            local.join("local.txt").exists() && local.join("helper.txt").exists(),
            "merged worktree must contain both sides' files"
        );
        // A real merge commit has two parents (HEAD^2 resolves).
        let two_parents = Cmd::new("git")
            .args(["rev-parse", "--verify", "HEAD^2"])
            .current_dir(&local)
            .output()
            .unwrap()
            .status
            .success();
        assert!(two_parents, "a merge commit (two parents) must exist");
    }

    #[test]
    fn pull_repo_aborts_conflicting_merge_and_leaves_clean_worktree() {
        let (tmp, local) = origin_and_local();

        // Remote and local change the SAME file to different content.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "helper-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "local-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-edit"], &local);
        let sha_before = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Conflicted),
            "conflicting diverge must report Conflicted, got {:?}: {}",
            result.outcome,
            result.message
        );
        // The merge must have been aborted: no MERGE_HEAD, clean status,
        // no conflict markers, and HEAD back where it started.
        assert!(
            !local.join(".git/MERGE_HEAD").exists(),
            "MERGE_HEAD must be gone after abort"
        );
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after merge --abort"
        );
        let base = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(
            !base.contains("<<<<<<<"),
            "base.txt must not contain conflict markers after abort"
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "aborted merge must leave main at its pre-merge commit"
        );
    }

    #[test]
    fn pull_repo_reports_failure_when_fast_forward_is_blocked() {
        let (tmp, local) = origin_and_local();
        // Remote advances base.txt; local has an UNCOMMITTED edit to the same
        // file, so `git merge --ff-only` refuses (local changes would be
        // overwritten). Reported by Copilot on PR #23: this used to fall
        // through to UpToDate and auto-close the overlay claiming success.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "remote-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "remote-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "dirty-local\n").unwrap();

        let result = pull_repo(&local);

        assert!(
            !result.success(),
            "a blocked fast-forward must not report success, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            !matches!(result.outcome, PullOutcome::UpToDate),
            "a blocked fast-forward must not report UpToDate"
        );
        assert!(
            !result.message.is_empty(),
            "the failure must surface git's error output"
        );
        // The dirty local edit must survive untouched.
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            "dirty-local\n",
            "the blocked pull must not clobber the local uncommitted change"
        );
    }

    #[test]
    fn pull_repo_reports_no_upstream_for_unpublished_branch() {
        let (_tmp, local) = origin_and_local();
        // A local-only branch: fetch succeeds (remote exists) but there is no
        // origin/feature to pull from.
        git(&["checkout", "-b", "feature"], &local);
        let sha_before = get_sha(&local, "feature");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::NoUpstream),
            "an unpublished branch must report NoUpstream (not DetachedHead), got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "NoUpstream is not a success");
        assert_eq!(
            sha_before,
            get_sha(&local, "feature"),
            "a NoUpstream pull must not move the branch"
        );
    }

    #[test]
    fn pull_repo_reports_detached_head_without_acting() {
        let (tmp, local) = origin_and_local();
        git(&["checkout", "--detach"], &local);
        let sha_before = get_sha(&local, "HEAD");
        // The remote advances after we detach; a detached-HEAD pull must
        // report WITHOUT acting (story 37), so not even the fetch may run —
        // local's origin/main remote-tracking ref must stay where it was.
        let origin_ref_before = get_sha(&local, "origin/main");
        advance_origin(tmp.path(), |helper| {
            git(
                &["commit", "--allow-empty", "-m", "remote-moves-on"],
                helper,
            );
        });

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::DetachedHead),
            "detached HEAD must report DetachedHead, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "HEAD"),
            "detached-HEAD pull must not move HEAD"
        );
        assert_eq!(
            origin_ref_before,
            get_sha(&local, "origin/main"),
            "detached-HEAD pull must not act at all — no fetch, so the \
             remote-tracking ref must be unchanged"
        );
    }

    #[test]
    fn push_repo_sets_upstream_on_new_branch() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // A new local branch with no upstream, carrying a distinct commit.
        git(&["checkout", "-b", "feature"], &local);
        git(&["commit", "--allow-empty", "-m", "feature-work"], &local);
        let feature_sha = get_sha(&local, "feature");

        let result = push_repo(&local, true);

        assert!(
            result.success,
            "pushing a new branch with -u must succeed: {}",
            result.message
        );
        // origin/feature now exists in the bare remote at the pushed commit.
        assert_eq!(
            feature_sha,
            get_sha(&bare, "refs/heads/feature"),
            "bare origin must have refs/heads/feature at the pushed commit"
        );
        // The local branch now tracks origin/feature.
        let upstream = Cmd::new("git")
            .args(["rev-parse", "--abbrev-ref", "feature@{upstream}"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&upstream.stdout).trim(),
            "origin/feature",
            "feature must track origin/feature after push -u"
        );
    }

    #[test]
    fn push_repo_plain_push_succeeds_when_ahead_with_upstream() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // Local advances main (already tracked) by one commit, then plain-pushes.
        git(&["commit", "--allow-empty", "-m", "local-ahead"], &local);
        let local_sha = get_sha(&local, "main");

        let result = push_repo(&local, false);

        assert!(
            result.success,
            "plain push of an ahead branch must succeed: {}",
            result.message
        );
        assert_eq!(
            local_sha,
            get_sha(&bare, "refs/heads/main"),
            "bare origin/main must equal local main after push"
        );
    }

    #[test]
    fn push_repo_reports_failure_on_rejected_non_fast_forward() {
        let (tmp, local) = origin_and_local();
        let bare = tmp.path().join("origin.git");

        // Remote advances main via a helper clone (local never sees this commit).
        advance_origin(tmp.path(), |helper| {
            git(&["commit", "--allow-empty", "-m", "remote-ahead"], helper);
        });
        let remote_sha = get_sha(&bare, "refs/heads/main");

        // Local also advances main with its own commit → diverged / non-fast-forward.
        git(&["commit", "--allow-empty", "-m", "local-ahead"], &local);
        let local_sha = get_sha(&local, "main");

        let result = push_repo(&local, false);

        assert!(
            !result.success,
            "a non-fast-forward push must be rejected, message: {}",
            result.message
        );
        assert!(
            !result.message.is_empty(),
            "a rejected push must carry a message"
        );
        assert!(
            result.message.to_lowercase().contains("reject"),
            "message should include git's rejection, got: {}",
            result.message
        );
        // The bare remote's main must NOT have moved to local's commit.
        assert_eq!(
            remote_sha,
            get_sha(&bare, "refs/heads/main"),
            "rejected push must not move origin/main"
        );
        assert_ne!(
            local_sha,
            get_sha(&bare, "refs/heads/main"),
            "origin/main must not equal local's rejected commit"
        );
    }

    #[test]
    fn commit_repo_commits_staged_changes() {
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "HEAD");

        // Stage a new file, then commit it via commit_repo.
        std::fs::write(local.join("new.txt"), "hello\n").unwrap();
        git(&["add", "."], &local);

        let result = commit_repo(&local, "add new file");

        assert!(result.success, "commit must succeed: {}", result.message);
        let sha_after = get_sha(&local, "HEAD");
        assert_ne!(sha_before, sha_after, "HEAD must advance after commit");

        // The new commit's subject is exactly the message.
        let subject = Cmd::new("git")
            .args(["log", "-1", "--pretty=%s"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&subject.stdout).trim(),
            "add new file",
            "the new commit's subject must be the message"
        );

        // The staged change is now committed: clean porcelain.
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after committing the staged change"
        );
    }

    #[test]
    fn commit_repo_creates_initial_commit_on_unborn_head() {
        // Fresh repo, a file staged, no commits yet (unborn HEAD).
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(repo)
            .output()
            .unwrap();
        git_setup(repo);
        std::fs::write(repo.join("first.txt"), "first\n").unwrap();
        git(&["add", "."], repo);

        // No commits yet: rev-parse HEAD fails.
        let before = Cmd::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            !before.status.success(),
            "HEAD must be unborn before the initial commit"
        );

        let result = commit_repo(repo, "initial");

        assert!(
            result.success,
            "initial commit on unborn HEAD must succeed: {}",
            result.message
        );
        let after = Cmd::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            after.status.success(),
            "HEAD must resolve after the initial commit"
        );
    }

    #[test]
    fn commit_repo_fails_when_nothing_staged() {
        // Clean repo with an existing commit and nothing staged: git refuses.
        let (_tmp, local) = origin_and_local();

        let result = commit_repo(&local, "nothing to do");

        assert!(
            !result.success,
            "commit with nothing staged must fail, message: {}",
            result.message
        );
        assert!(
            !result.message.is_empty(),
            "a failed commit must carry a message"
        );
    }

    #[test]
    fn rebase_repo_replays_branch_onto_advanced_target() {
        let (tmp, local) = origin_and_local();

        // Remote advances main with a new file; local advances main with a
        // different, non-conflicting file — a diverge that rebases cleanly.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("helper.txt"), "helper\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-side"], helper);
        });
        std::fs::write(local.join("local.txt"), "local\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-side"], &local);

        // rebase_repo's internal fetch refreshes origin/main before replaying.
        let result = rebase_repo(&local, "origin/main");

        assert!(
            matches!(result.outcome, RebaseOutcome::Rebased),
            "clean diverge must rebase, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            local.join("local.txt").exists() && local.join("helper.txt").exists(),
            "rebased worktree must contain both sides' files"
        );
        // Linear history: no merge commit (HEAD^2 must not resolve) and the
        // replayed commit must sit directly on top of origin/main.
        let two_parents = Cmd::new("git")
            .args(["rev-parse", "--verify", "HEAD^2"])
            .current_dir(&local)
            .output()
            .unwrap()
            .status
            .success();
        assert!(!two_parents, "a rebase must not create a merge commit");
        assert_eq!(
            get_sha(&local, "HEAD^"),
            get_sha(&local, "origin/main"),
            "the replayed commit must sit directly on origin/main"
        );
    }

    #[test]
    fn rebase_repo_reports_up_to_date_when_target_is_ancestor() {
        let (_tmp, local) = origin_and_local();
        // Local is ahead of origin/main; rebasing onto an ancestor is a no-op.
        git(&["commit", "--allow-empty", "-m", "local-only"], &local);
        let sha_before = get_sha(&local, "main");

        let result = rebase_repo(&local, "origin/main");

        assert!(
            matches!(result.outcome, RebaseOutcome::UpToDate),
            "rebasing onto an ancestor must report UpToDate, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(result.success(), "UpToDate is a success");
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "an up-to-date rebase must not move the branch"
        );
    }

    #[test]
    fn rebase_repo_aborts_conflicting_rebase_and_restores_branch() {
        let (tmp, local) = origin_and_local();

        // Remote and local change the SAME file to different content.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "helper-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "local-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-edit"], &local);
        let sha_before = get_sha(&local, "main");

        let result = rebase_repo(&local, "origin/main");

        assert!(
            matches!(result.outcome, RebaseOutcome::Conflicted),
            "conflicting rebase must report Conflicted, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "Conflicted is not a success");
        assert!(
            result.message.contains("git rebase"),
            "the conflict message must instruct the user to rebase manually, got: {}",
            result.message
        );
        // The rebase must have been aborted: no rebase in progress, clean
        // status, no conflict markers, and the branch back where it started.
        let no_rebase_in_progress = !Cmd::new("git")
            .args(["rebase", "--abort"])
            .current_dir(&local)
            .output()
            .unwrap()
            .status
            .success();
        assert!(
            no_rebase_in_progress,
            "no rebase may be in progress after the auto-abort"
        );
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after rebase --abort"
        );
        let base = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(
            !base.contains("<<<<<<<"),
            "base.txt must not contain conflict markers after abort"
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "aborted rebase must leave main at its pre-rebase commit"
        );
    }

    #[test]
    fn rebase_repo_reports_failure_on_unknown_target() {
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "main");

        let result = rebase_repo(&local, "no-such-branch");

        assert!(
            matches!(result.outcome, RebaseOutcome::Failed),
            "an unknown target must report Failed (not Conflicted), got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            result.message.contains("no-such-branch"),
            "the failure must surface git's error output naming the bad target, got: {}",
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "a failed rebase must not move the branch"
        );
    }

    #[test]
    fn rebase_repo_rejects_leading_dash_target() {
        // A target beginning with '-' would be parsed by `git rebase` as an
        // option, not a revspec (argument injection). rebase_repo must reject
        // it up front without running git or moving the branch.
        let (_tmp, local) = origin_and_local();
        let sha_before = get_sha(&local, "main");

        let result = rebase_repo(&local, "--onto=evil");

        assert!(
            matches!(result.outcome, RebaseOutcome::Failed),
            "a leading-dash target must report Failed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "a rejected target is not a success");
        assert!(
            result.message.contains('-'),
            "the rejection must name the offending target, got: {}",
            result.message
        );
        assert_eq!(
            sha_before,
            get_sha(&local, "main"),
            "a rejected rebase must not move the branch"
        );
    }

    #[test]
    fn rebase_repo_reports_detached_head_without_acting() {
        let (_tmp, local) = origin_and_local();
        git(&["checkout", "--detach"], &local);
        let sha_before = get_sha(&local, "HEAD");

        let result = rebase_repo(&local, "origin/main");

        assert!(
            matches!(result.outcome, RebaseOutcome::DetachedHead),
            "detached HEAD must report DetachedHead, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "DetachedHead is not a success");
        assert_eq!(
            sha_before,
            get_sha(&local, "HEAD"),
            "a detached-HEAD rebase must not move HEAD"
        );
    }

    #[test]
    fn rebase_repo_aborts_conflict_inside_linked_worktree() {
        let (tmp, local) = origin_and_local();

        // Remote advances main with a conflicting change to base.txt.
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "helper-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-edit"], helper);
        });

        // Create a real linked worktree on a NEW branch `feature` off local's
        // current HEAD. `local` stays on `main`; git forbids the same branch in
        // two worktrees, so `feature` must be a distinct branch. The worktree
        // shares `.git/config` with `local`, so git_setup's identity applies.
        let wt = tmp.path().join("wt");
        git(
            &["worktree", "add", "-b", "feature", wt.to_str().unwrap()],
            &local,
        );

        // In the worktree, commit a conflicting change to the same file.
        std::fs::write(wt.join("base.txt"), "local-version\n").unwrap();
        git(&["add", "."], &wt);
        git(&["commit", "-m", "feature-edit"], &wt);
        let sha_before = get_sha(&wt, "HEAD");

        // rebase_repo fetches origin internally, so rebasing `feature` onto the
        // conflicting origin/main must conflict and auto-abort. The rebase
        // state lives in `.git/worktrees/wt/rebase-merge`, so this exercises the
        // abort-based classification path inside a real linked worktree.
        let result = rebase_repo(&wt, "origin/main");

        assert!(
            matches!(result.outcome, RebaseOutcome::Conflicted),
            "conflicting rebase in a linked worktree must report Conflicted, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "Conflicted is not a success");
        // No rebase left in progress IN THE WORKTREE: --abort must now fail.
        let no_rebase_in_progress = !Cmd::new("git")
            .args(["rebase", "--abort"])
            .current_dir(&wt)
            .output()
            .unwrap()
            .status
            .success();
        assert!(
            no_rebase_in_progress,
            "no rebase may be in progress in the worktree after the auto-abort"
        );
        let porcelain = Cmd::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&wt)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&porcelain.stdout).trim().is_empty(),
            "worktree must be clean after rebase --abort"
        );
        let base = std::fs::read_to_string(wt.join("base.txt")).unwrap();
        assert!(
            !base.contains("<<<<<<<"),
            "base.txt must not contain conflict markers after abort"
        );
        assert_eq!(
            sha_before,
            get_sha(&wt, "HEAD"),
            "aborted rebase must leave feature at its pre-rebase commit"
        );
    }
}
