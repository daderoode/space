use crate::core::git::{self, RepoStatus};
use crate::core::spawn;
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

#[derive(Debug, Clone)]
pub enum BranchStrategy {
    /// Create a new branch with this name off the repo's default branch.
    NewBranch(String),
    /// Checkout an existing branch (local or remote-tracking).
    ExistingBranch(String),
    /// Detached HEAD at the default branch.
    DetachedHead,
}

/// A character that can break a one-row display or a log line, or make a
/// name read as a different name: the C0 and C1 control blocks and DEL
/// (`char::is_control`); the Unicode line and paragraph separators, which
/// are not controls but are line breaks to a terminal or a log; and the
/// invisible formatting characters (general category Cf), which include the
/// bidi overrides that render `safe\u{202e}elif.exe` reversed and the
/// zero-width space that makes `..\u{200b}` look like `..`. git refuses the
/// C0 block and DEL in a branch name but accepts everything else here, so
/// for those this is the only place they are refused. Whitespace that is
/// not a control (a no-break space, say) passes; at the ends of a name the
/// whitespace clause of the creation rule catches it.
fn is_control_like(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') || is_format_char(c)
}

/// Unicode general category Cf (format), Unicode 16.0, as the standard
/// library has no category query and a dependency for one table is not
/// worth it. Ranges from UnicodeData.txt; the soft hyphen, the Arabic
/// number signs, the zero-width and bidi characters, the word joiner and
/// invisible operators, the byte order mark, the interlinear annotation
/// characters, the Kaithi and Egyptian format controls, the Duployan and
/// musical formatting characters, and the tag characters.
fn is_format_char(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// The lookup guard: `name` must be one plain path component before it is
/// joined onto `workspaces.dir`. Rejects the empty name, `.` and `..`, any
/// `/` or `\`, and control or formatting characters (`is_control_like`). `Path::join` gives `..` the
/// parent of `ws_dir`, an absolute name replaces `ws_dir` entirely, and the
/// empty name is `ws_dir` itself, so without this every lookup by name can
/// read, create under or remove a directory the caller never configured.
///
/// Deliberately no stricter than that: `list_workspaces` hands the TUI
/// whatever directories exist, and a hand-made `.old` or `-scratch` space
/// must stay viewable and removable from the dashboard. The stricter rule for
/// names being created is `validate_space_name`.
pub fn require_plain_component(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Space name cannot be empty");
    }
    if name == "." || name == ".." {
        anyhow::bail!("Space name cannot be '.' or '..'");
    }
    if name.contains(['/', '\\']) {
        anyhow::bail!("Space name cannot contain '/' or '\\'");
    }
    if name.chars().any(is_control_like) {
        anyhow::bail!("Space name cannot contain control or formatting characters");
    }
    Ok(())
}

/// The creation rule for a space name, applied where a name is chosen: the
/// TUI name stage and MCP `create_workspace`. It includes the lookup guard
/// and adds: no leading or trailing whitespace (the TUI trims first, so this
/// clause is for programs, which should send the name that will be created),
/// and no leading `-` or `.` (one clause covers hidden directories and the
/// argv ambiguity of `space rm -x` and of the default branch `-x` landing in
/// `git worktree add -b`). Everything else is allowed, including interior
/// spaces, dots and non-ASCII; there is no length cap because the OS reports
/// `File name too long` truthfully and any number would be a guess across
/// filesystems. Names are rejected, never rewritten: a sanitised name would
/// create a directory the caller did not ask for and cannot find again.
pub fn validate_space_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Space name cannot be empty");
    }
    if name.trim() != name {
        anyhow::bail!("Space name cannot start or end with whitespace");
    }
    if name.contains(['/', '\\']) {
        anyhow::bail!("Space name cannot contain '/' or '\\'");
    }
    if name.starts_with('-') || name.starts_with('.') {
        anyhow::bail!("Space name cannot start with '-' or '.'");
    }
    if name.chars().any(is_control_like) {
        anyhow::bail!("Space name cannot contain control or formatting characters");
    }
    require_plain_component(name)
}

/// `require_plain_component` with the offending name in the message, for the
/// core functions that take a name and join it: `invalid space name "..":
/// Space name cannot be '.' or '..'`. `{:?}` so a control character prints
/// escaped rather than acting on the terminal.
fn checked_space_name(name: &str) -> Result<()> {
    require_plain_component(name)
        .map_err(|e| anyhow::anyhow!("invalid space name {:?}: {}", name, e))
}

/// Whether git accepts `name` as a branch name, by asking git:
/// `git check-ref-format --branch <name>`. Not mirrored in Rust because the
/// rule has subtleties (a trailing dot is a whole-name rule, `.lock` is per
/// component, `@` alone is fine but `HEAD` is not) that a mirror would drift
/// from, and the only test that could catch the drift is running git.
///
/// `--branch` takes the next argv verbatim, so `-foo` is checked rather than
/// parsed (`--branch -- x` is a usage error, so no `--` is passed). It works
/// outside any repository. `LC_ALL=C` pins the sentence the caller shows.
///
/// If git cannot be spawned at all the check passes. That is safe because
/// the one hazard this check exists to close, a leading-dash name reaching
/// the `-b` slot of `git worktree add`, is closed independently and without
/// a spawn by the guard in `create_worktree_cancellable`; and any other bad
/// name then fails a moment later in the add itself with `failed to spawn
/// git`, which is the truthful report when there is no git.
///
/// On refusal the error is git's own line with `fatal: ` stripped, e.g.
/// `'-foo' is not a valid branch name`.
pub fn check_branch_name(name: &str) -> Result<()> {
    let out = match spawn::output(
        Command::new("git")
            .args(["check-ref-format", "--branch", name])
            .env("LC_ALL", "C"),
    ) {
        Ok(out) => out,
        Err(_) => return Ok(()),
    };
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = stderr
        .lines()
        .find_map(|l| l.strip_prefix("fatal:"))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("'{}' is not a valid branch name", name));
    anyhow::bail!("{}", msg)
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
    checked_space_name(name)?;
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

/// Run a git command, capturing stdout+stderr. On non-zero exit, returns an
/// error that includes the first non-empty line of stderr so the TUI can show
/// the real git message (e.g. "'main' is already used by worktree at ...").
///
/// `LC_ALL=C` pins git's output language, as the `branch -f` runner in
/// `sync_repo_cancellable` does, because `refuses_because_checked_out` reads
/// this message: a localized git would turn every checked-out refusal into
/// the generic failure and the strategy-picker bounce would be dead again.
/// The cost is that every `worktree add` refusal reaches the log in English.
///
/// No time limit and no `setsid` here, and a test depends on that:
/// `creating_esc_stops_the_run_and_leaves_the_partial_space` in
/// `tests/tui_test.rs` holds this very call open through a `post-checkout`
/// hook until after its Esc, so bounding it the way `run_unattended` does
/// (`setsid` plus `killpg`) would kill that hook and its receipt (ticket 22).
/// Whoever bounds this call has to move that hold first.
fn git_worktree_add(args: &[&str], cwd: &Path) -> Result<()> {
    let out = spawn::output(
        Command::new("git")
            .args(args)
            .env("LC_ALL", "C")
            .current_dir(cwd),
    )
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
    let out = spawn::output(Command::new("git").args(args).current_dir(cwd))
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
    let local_exists = spawn::output(
        Command::new("git")
            .args(["rev-parse", "--verify", &local_ref])
            .current_dir(wt_path),
    )
    .map(|o| o.status.success())
    .unwrap_or(false);

    if local_exists {
        return run_git_in(wt_path, &["switch", "--", local_name]);
    }

    // Check remote branch
    let remote_exists = spawn::output(
        Command::new("git")
            .args(["rev-parse", "--verify", &remote_ref])
            .current_dir(wt_path),
    )
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

/// Start of the `stderr` of a `FetchOutcome::Failed` whose git never ran.
/// The sync report matches on this prefix to label the row `git did not
/// start`, so the wording is part of the contract.
pub const SPAWN_FAILURE_PREFIX: &str = "failed to spawn git";

/// How the `git fetch` half of a sync ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Ok,
    /// git exited non-zero or the fetch never got a result. `exit_code` is
    /// `None` when git did not start (`stderr` begins with
    /// `SPAWN_FAILURE_PREFIX`), when the wait itself failed, when the run was
    /// cancelled before the fetch, or when git was stopped by a signal;
    /// otherwise `stderr` is everything git wrote. `elapsed` is the wall
    /// clock of the whole unattended run: the child's own time plus the
    /// bounded wait (at most `UNATTENDED_READER_GRACE`) for its stderr to
    /// drain, which normally ends at once because the pipe closes with the
    /// child. It also counts any wait for the spawn gate (`core::spawn`),
    /// which is the length of other threads' spawns: milliseconds, far below
    /// `SLOW_FETCH_THRESHOLD`. It is what `is_slow` reads: a failure says
    /// nothing about whether repeating it is cheap, and the duration does. It
    /// is `Duration::ZERO` when nothing ran.
    Failed {
        exit_code: Option<i32>,
        stderr: String,
        elapsed: Duration,
    },
    /// The wall-clock limit expired and git's whole process group was stopped.
    /// `stderr` is whatever arrived before the kill, usually nothing. There is
    /// no `elapsed`: `after` already says the run spent the whole limit.
    TimedOut {
        after: Duration,
        stderr: String,
    },
}

impl FetchOutcome {
    /// Whether repeating this fetch would very likely cost what it cost the
    /// first time, which is what the pre-create skip rule turns on. See
    /// `is_slow_with` for the rule; the threshold is `SLOW_FETCH_THRESHOLD`.
    pub fn is_slow(&self) -> bool {
        self.is_slow_with(SLOW_FETCH_THRESHOLD)
    }

    /// `is_slow` against an explicit threshold, so the comparison can be
    /// tested without a five-second fetch.
    ///
    /// `TimedOut` is slow whatever the threshold: it spent the whole limit by
    /// definition, and tests run limits of a few hundred milliseconds that
    /// must still count. `Failed` is slow when it took at or above the
    /// threshold. `Ok` is never slow: a fetch that worked is not repeated
    /// at all.
    pub fn is_slow_with(&self, threshold: Duration) -> bool {
        match self {
            FetchOutcome::Ok => false,
            FetchOutcome::Failed { elapsed, .. } => *elapsed >= threshold,
            FetchOutcome::TimedOut { .. } => true,
        }
    }
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

/// Wall-clock limit on a fetch run under the unattended-run policy. Fixed in
/// Wave 1; a sync's fast-forward calls are local and run without a limit.
pub const UNATTENDED_FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// A fetch that failed at or above this is not run again before the worktree
/// is created: repeating it would very likely cost the same again, and every
/// repo behind it waits.
///
/// Why 5s. Every fast refusal measured on this machine lands between 0.45s
/// and 1.6s (prompts disabled, host key refused, passphrase key under
/// `BatchMode`), and the slow cases at about 62s (an ssh agent that never
/// answers) and 75s (git's connect timeout on a black-hole https host), with
/// an explicit ssh `ConnectTimeout=5` at 5.06s. 5s sits in the empty band
/// between the two groups with margin either side. The retry this still
/// accepts is bounded by the threshold itself: a fetch that failed in 4s will
/// likely fail in about 4s again, now on the worker with a live footer, and
/// it is worth that because it puts git's fresh refusal into the Creating
/// log. Must stay well below `UNATTENDED_FETCH_TIMEOUT`, or a timeout would
/// again be the only thing the rule catches.
pub const SLOW_FETCH_THRESHOLD: Duration = Duration::from_secs(5);
/// How long a timed-out child gets to clean up after SIGTERM before SIGKILL.
const UNATTENDED_KILL_GRACE: Duration = Duration::from_secs(2);
/// How often an unattended run checks whether its child has exited, whether
/// the stderr reader has finished and, after SIGTERM, whether the group leader
/// is gone. Each check can notice a change up to one interval late, so up to
/// two intervals land in every recorded `elapsed`: one for the child's exit,
/// one for its reader. The reader grace and the kill grace can each run over
/// by up to one interval. Hence the relation asserted below: the interval is
/// at most a tenth of `UNATTENDED_READER_GRACE`, the shortest bound it cuts
/// up, and at most a tenth of `SLOW_FETCH_THRESHOLD`, which every recorded
/// `elapsed` is compared with. Until ticket 18, a 1s ceiling in a fetch test
/// was the only thing that caught a slow interval, and only by accident.
const UNATTENDED_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long to wait for git's stderr pipe to close after git itself exited.
/// A helper that outlived git and still holds the pipe must not stall the
/// caller: after this the captured text is used as is.
const UNATTENDED_READER_GRACE: Duration = Duration::from_secs(1);
const _: () = assert!(
    UNATTENDED_POLL_INTERVAL.as_nanos() * 10 <= UNATTENDED_READER_GRACE.as_nanos()
        && UNATTENDED_POLL_INTERVAL.as_nanos() * 10 <= SLOW_FETCH_THRESHOLD.as_nanos(),
    "UNATTENDED_POLL_INTERVAL must be at most a tenth of UNATTENDED_READER_GRACE and of \
     SLOW_FETCH_THRESHOLD"
);

/// Fetch from `origin` and fast-forward all local branches that are strictly
/// behind their `origin/<branch>` ref (0 ahead, N behind); this assumes a single
/// remote named `origin` rather than each branch's configured upstream. Branches
/// with local commits ahead, diverged, or currently checked out are left
/// untouched and the refusals are reported as skips.
///
/// The fetch runs under the unattended-run policy (see `fetch_origin_unattended`)
/// with the fixed `UNATTENDED_FETCH_TIMEOUT`. When it does not succeed the outcome
/// carries the failure and no branch work is attempted; the caller continues
/// with local refs.
#[allow(dead_code)] // public API; the TUI worker calls sync_repo_cancellable
pub fn sync_repo(repo_path: &Path) -> SyncOutcome {
    sync_repo_with_timeout(repo_path, UNATTENDED_FETCH_TIMEOUT)
}

/// `sync_repo` with an explicit fetch limit. The limit is a parameter so tests
/// can use a short one; the user-facing value is `UNATTENDED_FETCH_TIMEOUT`.
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
            // Nothing ran, so nothing is skipped downstream: the creation
            // still fetches this repo.
            elapsed: Duration::ZERO,
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
        let out = spawn::output(
            Command::new("git")
                .args(["branch", "-f", &branch, &remote_ref])
                .env("LC_ALL", "C")
                .current_dir(repo_path),
        );
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

/// The two spellings of git's "that branch is held by another worktree"
/// refusal: `used by worktree at '<path>'` (git 2.42 and later) and
/// `checked out at '<path>'` (git 2.38 to 2.41). Both `branch -f` and
/// `worktree add` refuse through git's `die_if_checked_out`, so they changed
/// wording together, and the two predicates that read them share this list
/// rather than each keeping a copy that can go stale on its own.
const WORKTREE_HOLDS_BRANCH_MARKERS: [&str; 2] = ["used by worktree at '", "checked out at '"];

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
    for marker in WORKTREE_HOLDS_BRANCH_MARKERS {
        if let Some((_, rest)) = line.split_once(marker) {
            if let Some(path) = rest.strip_suffix('\'') {
                return SkipReason::CheckedOutAt(PathBuf::from(path));
            }
        }
    }
    SkipReason::Other(line.to_string())
}

/// Run `git <args>` in `cwd` under the unattended-run policy (see
/// `run_unattended`), with `GIT_TERMINAL_PROMPT=0` so an https remote that
/// would prompt fails as "terminal prompts disabled" rather than
/// "Device not configured". `GIT_SSH_COMMAND` is deliberately not set so
/// the user's own ssh configuration applies.
///
/// `LC_ALL=C` pins git's output language, as `check_branch_name`,
/// `git_worktree_add` and the `branch -f` runner do. In production the
/// readers display rather than classify: the sync report's DETAIL column and
/// the Creating log's fetch note strip a `fatal: ` or `error: ` prefix
/// (`first_stderr_line`) and show the rest. git marks those prefixes for
/// translation (`usage.c`, git 2.50.1), so a gettext git under a non-English
/// locale would show a translated reason with its translated prefix left
/// on. The wording tests on this path mostly read literals git never
/// translates: `terminal prompts disabled` (`prompt.c`) for the https-prompt
/// tests' `prompt_probe`, and `'origin' does not appear to be a git
/// repository` (`builtin/upload-pack.c`) for the missing-remote sync test;
/// the one translated sentence a test asserts is `Could not read from
/// remote repository.` (`connect.c`), in the test of this pin. The cost is
/// that every fetch failure reaches the report in English. Unlike the three
/// siblings, which run local commands, this is the one pinned run that
/// reaches ssh, curl and a configured credential or askpass helper, and they
/// inherit the pin too; `LC_ALL` rather than `LC_MESSAGES` for consistency
/// with the siblings, as the ticket asked.
///
/// Only stderr is captured: `run_unattended` discards stdout, and
/// `Unattended` has nowhere to carry it. That suits a command whose useful
/// output is on stderr, which is why `git fetch --quiet` fits. It does not
/// suit `git pull`, whose fast-forward summary goes to stdout, or `git log`,
/// whose output is all stdout: an adopter that needs stdout has to extend
/// the helper to capture it first, or it will silently log nothing.
pub fn run_git_unattended(args: &[&str], cwd: &Path, timeout: Duration) -> Unattended {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    run_unattended(cmd, timeout)
}

/// Run `git fetch --quiet origin` under the unattended-run policy and report
/// it as a `FetchOutcome`.
fn fetch_origin_unattended(repo_path: &Path, timeout: Duration) -> FetchOutcome {
    // Brackets the fetch and nothing else: the caller's branch work runs
    // after this returns, so it cannot inflate a failure's elapsed time.
    let started = Instant::now();
    match run_git_unattended(&["fetch", "--quiet", "origin"], repo_path, timeout) {
        Unattended::Exited { status, stderr } => {
            if status.success() {
                FetchOutcome::Ok
            } else {
                FetchOutcome::Failed {
                    exit_code: status.code(),
                    stderr,
                    elapsed: started.elapsed(),
                }
            }
        }
        Unattended::TimedOut { stderr } => FetchOutcome::TimedOut {
            after: timeout,
            stderr,
        },
        Unattended::SpawnFailed(e) => FetchOutcome::Failed {
            exit_code: None,
            stderr: format!("{}: {}", SPAWN_FAILURE_PREFIX, e),
            elapsed: started.elapsed(),
        },
        Unattended::WaitFailed { stderr } => FetchOutcome::Failed {
            exit_code: None,
            stderr: format!("could not wait for git\n{}", stderr),
            elapsed: started.elapsed(),
        },
    }
}

/// How a child run by `run_unattended` ended. `stderr` is what the child
/// wrote before it ended (or before it was stopped).
#[derive(Debug)]
pub enum Unattended {
    /// The child exited on its own: within the limit with any status, or
    /// with a success status during the grace after SIGTERM (a non-success
    /// exit after the limit is a `TimedOut`).
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
/// keys fail instead of waiting), stdin is null, stdout is discarded and
/// stderr is captured. Only stderr is kept because `git fetch --quiet` writes
/// its summary there and every outcome reports stderr alone; draining stdout
/// would cost a reader thread per run for text nobody reads. The session is
/// also the process group the timeout stops: SIGTERM to the group (git and
/// its ssh or https helper), a short grace so git can drop its lockfiles,
/// then SIGKILL to whatever is left. A child that exits with success during
/// that grace still counts as `Exited`, so work that completed at the
/// deadline is not thrown away.
///
/// Whenever the child was seen to end, the stderr reader is given a bounded
/// moment to finish, so a helper that keeps the pipe open never stalls the
/// caller.
fn run_unattended(mut cmd: Command, timeout: Duration) -> Unattended {
    use std::os::unix::process::CommandExt;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
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
    let mut child = match spawn::spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => return Unattended::SpawnFailed(e),
    };
    let pgid = child.id() as libc::pid_t;
    let (stderr, stderr_reader) = capture_in_background(child.stderr.take());

    match wait_until(&mut child, Instant::now() + timeout) {
        Wait::Exited(status) => {
            // The child has exited, so the pipe normally reaches end-of-file
            // at once: give the reader a moment to copy the last write, but
            // never wait on a helper that outlived the child.
            join_bounded(&stderr_reader, UNATTENDED_READER_GRACE);
            Unattended::Exited {
                status,
                stderr: snapshot(&stderr),
            }
        }
        Wait::Timeout => {
            let status = stop_process_group(&mut child, pgid);
            // Nothing in the group survives the SIGKILL, so the pipe closes
            // and the reader finishes; the bound is a backstop.
            join_bounded(&stderr_reader, UNATTENDED_READER_GRACE);
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
        std::thread::sleep(UNATTENDED_POLL_INTERVAL);
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
        std::thread::sleep(UNATTENDED_POLL_INTERVAL.min(deadline - now));
    }
}

/// SIGTERM the child's process group, give the leader up to `UNATTENDED_KILL_GRACE`
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
    let deadline = Instant::now() + UNATTENDED_KILL_GRACE;
    loop {
        match leader_state(pid) {
            Leader::Running if Instant::now() < deadline => {
                std::thread::sleep(UNATTENDED_POLL_INTERVAL)
            }
            Leader::Running | Leader::Exited => break,
            // Not ours to wait for, so nothing pins the group id: it must
            // not be signalled. Cannot happen while `wait_until` is the only
            // other waiter, but the guarantee below depends on it.
            Leader::NotOurs => return child.wait().ok(),
        }
    }
    // SAFETY: as above; the leader is still unreaped (`leader_state` never
    // reaps), so the group id is still ours. ESRCH, when everything already
    // left, is fine.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
    child.wait().ok()
}

enum Leader {
    Running,
    /// Exited and left as a zombie for `Child::wait`.
    Exited,
    /// `waitid` refused the pid (ECHILD: reaped elsewhere, or never ours).
    NotOurs,
}

/// Whether `pid`, an unreaped child of ours, has exited. Uses `waitid` with
/// `WNOWAIT` so the zombie stays in place for `Child::wait`; `kill(pid, 0)`
/// would not do, as it succeeds for a zombie too.
fn leader_state(pid: libc::pid_t) -> Leader {
    // SAFETY: `siginfo_t` is plain data for which all-zero is a valid value,
    // and waitid only writes into it. A reported child sets `si_signo` to
    // SIGCHLD; with WNOHANG and no state change Linux writes zeros and XNU
    // leaves the struct untouched, so starting from zeros covers both.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let rc = libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        if rc != 0 {
            Leader::NotOurs
        } else if info.si_signo == libc::SIGCHLD {
            Leader::Exited
        } else {
            Leader::Running
        }
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
    /// Local and upstream diverged with conflicts and `git merge --abort`
    /// failed, so the repo is left mid-merge (MERGE_HEAD present, conflict
    /// markers in the tree) for the user to finish or abort by hand; git's
    /// reason is in the message.
    AbortFailed,
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
    /// HEAD, no upstream, conflicts, a failed abort, and failed fast-forwards.
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

/// Whether `repo_path` has a merge in progress: the `MERGE_HEAD` file exists
/// in its git dir, which is git's own test. The path comes from `git rev-parse
/// --git-path`, so a linked worktree's private git dir is honoured (a relative
/// answer is relative to the worktree). Not `rev-parse --verify MERGE_HEAD`,
/// which also resolves a branch or tag of that name, so a tag named
/// `MERGE_HEAD` on origin would read as a merge in progress after every fetch.
/// An error means git could not be asked; callers treat that as unknown and
/// act on nothing.
fn merge_in_progress(repo_path: &Path) -> std::io::Result<bool> {
    let out = spawn::output(
        Command::new("git")
            .args(["rev-parse", "--git-path", "MERGE_HEAD"])
            .env("LC_ALL", "C")
            .current_dir(repo_path),
    )?;
    if !out.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(repo_path.join(path).exists())
}

/// Pull the current branch of `repo_path` from its `origin/<branch>` upstream.
///
/// Fetches first, then classifies the branch state and acts:
/// - behind only  → fast-forward (`FastForwarded`)
/// - diverged     → real merge; on conflict, `git merge --abort` (`Merged`/`Conflicted`,
///   or `AbortFailed` when the abort itself fails and the repo stays mid-merge);
///   a merge that never started is `Failed` and nothing is aborted
/// - a merge already in progress (MERGE_HEAD) → `Failed` before the fetch, whatever
///   the branch state, so the abort only ever runs on a merge this call started
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

    // A merge the user left in progress is refused before anything runs,
    // whatever the branch state: "already up to date" or "nothing to pull"
    // would be false with conflict markers in the tree, git refuses a merge
    // or fast-forward over unmerged entries anyway, and the abort below must
    // only ever run on a merge this call started. Checked before the fetch,
    // like the detached-HEAD refusal: report without acting.
    match merge_in_progress(repo_path) {
        Ok(false) => {}
        Ok(true) => {
            return PullResult {
                outcome: PullOutcome::Failed,
                message: format!(
                    "{} is mid-merge (MERGE_HEAD present); run 'git merge --continue' \
                     or 'git merge --abort' in that repo before pulling.",
                    branch
                ),
            };
        }
        Err(err) => {
            return PullResult {
                outcome: PullOutcome::Failed,
                message: format!(
                    "Could not tell whether {} is mid-merge ({}); nothing was pulled.",
                    branch, err
                ),
            };
        }
    }

    // `spawn::output` (not `spawn::status`): capture stderr both to surface the real
    // failure cause (auth, DNS, missing remote) and to keep git from writing
    // to the inherited stderr, which would scribble over the raw-mode TUI.
    let fetch = spawn::output(
        Command::new("git")
            .args(["fetch", "--quiet", "origin"])
            .current_dir(repo_path),
    );
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
    let remote_exists = spawn::output(
        Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{}", branch),
            ])
            .current_dir(repo_path),
    )
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
        let output = spawn::output(
            Command::new("git")
                .args(["merge", "--ff-only", &remote_ref])
                .current_dir(repo_path),
        );
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
        // `LC_ALL=C` keeps git's text English for the readers of the merge's
        // and the abort's stderr: the report shown in the overlay, and the
        // tests that assert on its wording.
        let merge = spawn::output(
            Command::new("git")
                .args(["merge", "--no-edit", &remote_ref])
                .env("LC_ALL", "C")
                .current_dir(repo_path),
        );
        let merge = match merge {
            Ok(o) if o.status.success() => {
                return PullResult {
                    outcome: PullOutcome::Merged,
                    message: format!(
                        "Merged {} into {} ({} ahead, {} behind).",
                        remote_ref, branch, ahead, behind
                    ),
                };
            }
            Ok(o) => o,
            Err(err) => {
                return PullResult {
                    outcome: PullOutcome::Failed,
                    message: format!("Merge of {} into {} failed: {}", remote_ref, branch, err),
                };
            }
        };
        // Exit codes do not separate the shapes (a conflict exits 1, a dirty
        // tree 2, a locked index 128); MERGE_HEAD does. Absent, the merge never
        // started and there is nothing to abort: the tree is as the user left
        // it, and git's text says why.
        let in_progress = merge_in_progress(repo_path);
        if !matches!(in_progress, Ok(true)) {
            let stderr = String::from_utf8_lossy(&merge.stderr);
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                "git reported no error output"
            } else {
                detail
            };
            // Unknown is not "not started": say so, and abort nothing, since
            // an abort only ever runs on a merge this call is sure it began.
            let unknown = match in_progress {
                Err(err) => format!(
                    "\ngit could not say whether a merge was left in progress ({}); \
                     check 'git status' in that repo before pulling again.",
                    err
                ),
                Ok(_) => String::new(),
            };
            return PullResult {
                outcome: PullOutcome::Failed,
                message: format!(
                    "Merge of {} into {} failed: {}{}",
                    remote_ref, branch, detail, unknown
                ),
            };
        }
        // The merge conflicted: restore the pre-merge worktree so `space` never
        // leaves the repo half-merged. `spawn::output` keeps git's stderr off
        // the TUI's screen and in the report if the abort fails too.
        let abort = spawn::output(
            Command::new("git")
                .args(["merge", "--abort"])
                .env("LC_ALL", "C")
                .current_dir(repo_path),
        );
        let abort_failure = match abort {
            Ok(o) if o.status.success() => None,
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                // The first line only: git follows a lock refusal with six
                // lines of advice, and the Running overlay windows by line
                // then wraps, so a long report pushes its last line out.
                let first = stderr.lines().map(str::trim).find(|l| !l.is_empty());
                Some(first.unwrap_or("git reported no error output").to_string())
            }
            Err(err) => Some(err.to_string()),
        };
        return match abort_failure {
            None => PullResult {
                outcome: PullOutcome::Conflicted,
                message: format!(
                    "Merge of {} into {} conflicted; aborted and restored a clean worktree.",
                    remote_ref, branch
                ),
            },
            Some(reason) => PullResult {
                outcome: PullOutcome::AbortFailed,
                // Three lines, each streamed as its own overlay line: what
                // happened, git's reason, what to do.
                message: format!(
                    "Merge of {} into {} conflicted and 'git merge --abort' failed:\n\
                     {}\n\
                     {} is still mid-merge (MERGE_HEAD present); resolve the conflicts and \
                     run 'git merge --continue', or run 'git merge --abort'.",
                    remote_ref, branch, reason, branch
                ),
            },
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

/// Push the current branch of `repo_path`.
///
/// - `set_upstream == true`  → `git push -u origin <branch>` (first publish of a
///   branch with no upstream; also records the tracking ref).
/// - `set_upstream == false` → `git push` (branch already has an upstream),
///   which git routes to the branch's own push destination
///   (`git::push_target`): origin for a branch that tracks origin, another
///   remote for a branch that tracks it (a worktree made from
///   `upstream/<x>`, ticket 25). The git-ops overlay confirms that second
///   case before calling this.
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

    let out = spawn::output(Command::new("git").args(&args).current_dir(repo_path));

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
    let out = spawn::output(
        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(repo_path),
    );

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
    let _ = spawn::output(
        Command::new("git")
            .args(["fetch", "--quiet", "origin"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .current_dir(repo_path),
    );

    // `LC_ALL=C` pins git's output language so the up-to-date classification
    // below (`stdout.contains("is up to date")`) is stable: git localizes that
    // message via gettext, and a non-English LANG would misclassify UpToDate
    // as Rebased.
    let out = spawn::output(
        Command::new("git")
            .args(["rebase", onto])
            .env("LC_ALL", "C")
            .current_dir(repo_path),
    );

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
            let aborted = spawn::output(
                Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo_path),
            )
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
    let out = spawn::output(
        Command::new("git")
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .current_dir(repo_path),
    )
    .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

/// Whether `create_worktree` refreshes the repo's refs before it adds the
/// worktree, and under what limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreCreateFetch {
    /// No fetch is run. The caller has decided this repo does not need
    /// one: either the sync already fetched it, or the sync's own fetch of
    /// that remote was slow (`FetchOutcome::is_slow`) and repeating it here
    /// would very likely cost the same again. `create_worktree` does not
    /// distinguish the two; the caller does, and reports them differently.
    Skip,
    /// Fetch `origin` under the unattended-run policy with this wall-clock
    /// limit. The user-facing value is `UNATTENDED_FETCH_TIMEOUT`; tests
    /// pass a short one.
    Run(Duration),
}

/// A worktree creation attempt: how its pre-create fetch went, and what
/// the creation itself did. The fetch outcome is reported whether or not
/// the worktree was added, so a failed add on stale refs still shows the
/// fetch line that explains it.
#[derive(Debug)]
pub struct WorktreeAttempt {
    /// The fetch outcome; `None` when `PreCreateFetch::Skip` meant none
    /// ran, when the strategy reads no remote ref so none was needed
    /// (`strategy_reads_origin`), or when the attempt failed before the
    /// fetch could run.
    pub fetch: Option<FetchOutcome>,
    /// The new worktree's path, or why the creation refused.
    pub created: Result<PathBuf>,
}

/// Returns the path to the created worktree.
///
/// The pre-create fetch runs under the unattended-run policy with the fixed
/// `UNATTENDED_FETCH_TIMEOUT`, and its outcome is discarded: creation
/// continues with local refs whatever happened. Callers that want to report
/// the fetch use `create_worktree_with_fetch`.
pub fn create_worktree(
    repo_path: &Path,
    ws_dir: &Path,
    ws_name: &str,
    strategy: &BranchStrategy,
) -> Result<PathBuf> {
    create_worktree_with_fetch(
        repo_path,
        ws_dir,
        ws_name,
        strategy,
        PreCreateFetch::Run(UNATTENDED_FETCH_TIMEOUT),
    )
    .created
}

/// `create_worktree` that also reports its pre-create fetch. A fetch that
/// failed or timed out is not an error: the worktree is added from local
/// refs and the outcome is returned for the caller to log. It is returned
/// even when the add then refused, because stale refs are a likely reason
/// for that refusal.
pub fn create_worktree_with_fetch(
    repo_path: &Path,
    ws_dir: &Path,
    ws_name: &str,
    strategy: &BranchStrategy,
    fetch: PreCreateFetch,
) -> WorktreeAttempt {
    create_worktree_cancellable(
        repo_path,
        ws_dir,
        ws_name,
        strategy,
        fetch,
        &AtomicBool::new(false),
    )
}

/// Whether git refused a `worktree add` because the branch is checked out
/// elsewhere, which the Creating stage treats as "pick another strategy"
/// rather than as a plain failure.
///
/// One predicate because two layers act on it and they must not drift: the
/// Creating worker ends its run on it, and the App bounces the flow back to
/// the branch-strategy picker.
///
/// Both of git's wordings count: `fatal: '<branch>' is already used by
/// worktree at '<path>'` (git 2.42 and later, verified on git 2.50.1, Apple
/// Git-155) and `fatal: '<branch>' is already checked out at '<path>'` (git
/// 2.38 to 2.41). The markers are the ones `parse_skip_reason` uses for
/// `branch -f`, shared rather than repeated because both refusals come from
/// the same git function, `die_if_checked_out`.
pub fn refuses_because_checked_out(err: &str) -> bool {
    WORKTREE_HOLDS_BRANCH_MARKERS
        .iter()
        .any(|marker| err.contains(marker))
}

/// The path `create_worktree*` adds for `repo_path` in this space:
/// `<ws_dir>/<ws_name>/<the repo directory's name>`.
///
/// Shared rather than derived twice because the Creating worker asks
/// `placement_of` about this exact path before attempting the add. Two
/// copies of the rule would let the predicate check one path while the add
/// created another, and the skip would quietly stop matching.
pub fn worktree_path(ws_dir: &Path, ws_name: &str, repo_path: &Path) -> PathBuf {
    let repo_name = repo_path.file_name().unwrap_or_default().to_string_lossy();
    ws_dir.join(ws_name).join(repo_name.as_ref())
}

/// Where a repo stands at its place in a space: the Creating worker's and
/// MCP `place_repos`'s decision to skip it, refuse it, or attempt the add.
#[derive(Debug, PartialEq, Eq)]
pub enum Placement {
    /// A finished worktree of this repo is already there. It is skipped and
    /// reported as already created.
    Adopted,
    /// A worktree of this repo whose `git worktree add` never finished. The
    /// add is not run: it would refuse with `already exists`, and the tree
    /// may be missing files. The text is the row's reason and names the way
    /// out.
    HalfBuilt(String),
    /// Anything else: the add runs, and git's own refusal is the row.
    Attempt,
}

/// Where the repo at `repo_path` stands at `wt_path`, read from git's own
/// layout on disk by the same reader `remove_workspace` uses
/// (`classify_space_entry`), and not from libgit2.
///
/// libgit2 refuses any repository format extension its version does not
/// know, so the earlier predicate, which opened the path with git2,
/// answered "not a worktree" for a worktree made under
/// `worktree.useRelativePaths` (git 2.48 and later, `extensions.relativeWorktrees`)
/// and for one whose source repo is reftable (git 2.45 and later,
/// `extensions.refstorage`), and every retry for those users then failed on
/// `already exists`. The layout does not move with a format extension.
///
/// One notch stricter than the codebase's usual test for a repo in a space,
/// `<path>/.git` exists, which a clone dropped in the directory also
/// satisfies. A linked worktree's admin directory holds `commondir`, git's
/// own link to the repository it belongs to (`../..` from
/// `<source>/.git/worktrees/<id>`, relative to the admin directory, the
/// same under relative paths); resolving it and comparing with the source
/// repo's `.git` is what ties the directory to THIS repo. A submodule
/// checkout has no `commondir` (its gitdir is a repository of its own under
/// `<host>/.git/modules/`), so the reader calls it a repository and it is
/// attempted, as before. Both sides are canonicalised because git resolves
/// symlinks and the app does not: on macOS a space under `$TMPDIR` reads
/// back as `/private/var/...` while the config holds `/var/...`.
///
/// Branch and strategy are deliberately not checked. The retry this serves
/// exists BECAUSE the strategy changed, so the repos already in the space are
/// on the old one by design; demanding a match would re-attempt every one of
/// them and fail on `already exists`, which is the defect the skip removes.
///
/// Every answer the reader gives other than a worktree of this repo is
/// `Attempt`: a plain directory, a clone, a submodule, an orphan, a
/// directory git has no record of, a worktree of another repo, and a `.git`
/// file it cannot read or resolve. Each of
/// those still reaches `git worktree add` and fails exactly as before, which
/// is the truthful row for a path this space does not own; this side
/// deletes nothing, so the reader's caution has nothing to protect here and
/// git's refusal is the right message. The source-repo-is-itself-a-worktree
/// case falls the same way: its `.git` is a file, so the comparison cannot
/// match. No ordinary route reaches it, since `find_repos_in` requires
/// `.git` to be a directory and every list of repo paths in the app is the
/// scanner's output; the one way in is a hand-edited cache file.
///
/// A worktree of this repo that git never finished (`half_built`) is the
/// third answer, and the reason this is not a bool: adopting one reports a
/// tree that may be missing files as complete, and attempting the add fails
/// on `already exists` with nothing said about why.
pub fn placement_of(wt_path: &Path, repo_path: &Path) -> Placement {
    let admin = match classify_space_entry(wt_path) {
        SpaceEntry::Worktree { admin } => admin,
        _ => return Placement::Attempt,
    };
    if !admin_belongs_to(&admin, repo_path) {
        return Placement::Attempt;
    }
    if !half_built(&admin) {
        return Placement::Adopted;
    }
    Placement::HalfBuilt(format!(
        "{:?} holds a worktree of this repo whose `git worktree add` never finished: its \
         admin directory {:?} is still locked from the add and holds the checkout's \
         index.lock and no index; nothing was attempted. Remove the space and create it \
         again, or run `git worktree remove -f -f {:?}` in {:?}, then retry",
        wt_path, admin, wt_path, repo_path
    ))
}

/// Whether the admin directory's `commondir` leads to `repo_path`'s `.git`.
/// False on any error: a `commondir` that cannot be read or resolved is not
/// evidence that the worktree is this repo's.
fn admin_belongs_to(admin: &Path, repo_path: &Path) -> bool {
    let common = match std::fs::read_to_string(admin.join("commondir")) {
        Ok(common) => common,
        Err(_) => return false,
    };
    let common = resolve_against(admin, common.trim_end_matches(['\n', '\r']));
    match (
        std::fs::canonicalize(common),
        std::fs::canonicalize(repo_path.join(".git")),
    ) {
        (Ok(linked), Ok(source)) => linked == source,
        _ => false,
    }
}

/// Whether a worktree's admin directory says its `git worktree add` never
/// finished its checkout: `locked` present, `index.lock` present, no `index`.
///
/// `add_worktree` (`builtin/worktree.c`) writes `locked` before anything
/// else of the new worktree and unlinks it after the checkout child
/// returns, on success and on failure alike; a failure or a caught signal
/// also removes the admin directory and the tree. Only `kill -9`, a crash
/// or power loss leaves the lock. The checkout, a child `reset --hard`,
/// takes `index.lock`, writes every file, and commits `index` last by
/// rename, so a locked worktree whose `index.lock` is there and whose
/// `index` is not is one whose checkout was killed with files missing.
/// Probed three times on git 2.50.1 with parent and child killed together:
/// that shape every time.
///
/// The content of `locked` is deliberately not read. git writes
/// `_("initializing")` there while adding, but a user is free to type that
/// reason into `git worktree lock`, and the remove side deletes on this
/// answer; a rule that read the word would force-delete a finished tree on
/// a string. This rule cannot match a tree that ever finished a checkout:
/// git writes the index lock-then-rename, so a killed index writer in a
/// finished worktree leaves the old `index` in place (probed with a `git
/// add` killed mid-write). Nor can it match a `--no-checkout` worktree the
/// user locked, which has no `index` but no `index.lock` either, and may
/// hold files put there by hand. The one hand-made shape that does match
/// is that same locked `--no-checkout` worktree after a `git add` in it
/// was killed before its first index was ever written; that leaves the
/// three signs with no add in progress, and a forced removal of its space
/// then deletes it. Accepted: it takes a worktree shape the app never
/// makes, a lock, and a kill.
///
/// Accepted, decided with the coordinator: a `git worktree add` whose
/// parent alone was killed leaves its checkout child to finish, so the tree
/// is complete and has an `index` under a stale `initializing` lock; that
/// tree is adopted as the complete tree it is, and the lock meets ticket
/// 27's unlock hint at removal, which is friction, not loss. An add still
/// running in another process looks half-built for as long as its checkout
/// runs; a forced removal of its space then removes the tree under it,
/// where git alone would have refused on the lock. The tree being built
/// holds nothing of the user's. What git leaves of it goes with the next
/// removal as plain content if its `.git` file went too, and is otherwise
/// kept as `Unregistered` (its source repo is still there) for the user to
/// delete by hand, which is friction, not loss. When the checkout child
/// finished before the admin
/// directory went, the add exits 0 and the process that started it shows
/// that repo as created; that row is this race, not a worker bug.
fn half_built(admin: &Path) -> bool {
    admin.join("locked").exists()
        && admin.join("index.lock").is_file()
        && !admin.join("index").exists()
}

/// The remote the app fetches before an add and compares worktrees
/// against. Its prefix is read as a remote-tracking name in every repo,
/// configured or not, as it was before ticket 25 made the other remotes'
/// prefixes count.
const DEFAULT_REMOTE: &str = "origin";

/// The remotes configured in `repo_path`, by name, for `split_remote_branch`.
/// A repo git2 cannot open, or whose remotes it cannot list, gives an empty
/// list: `DEFAULT_REMOTE` still applies, and the add itself reports what is
/// wrong with the repo.
pub fn remote_names(repo_path: &Path) -> Vec<String> {
    git2::Repository::open(repo_path)
        .ok()
        .and_then(|repo| repo.remotes().ok())
        .map(|names| names.iter().flatten().map(String::from).collect())
        .unwrap_or_default()
}

/// Split an existing-branch name into `(remote, local)` when it is the
/// `<remote>/<local>` form the branch picker offers for a remote-tracking
/// branch: `remote` is one of `remotes` (the repo's configured remotes) or
/// `DEFAULT_REMOTE`, and `local` is what follows the `/`, the name of the
/// branch git will create to track it. The longest remote wins, because a
/// remote name may itself contain `/` (`git remote add a/b` is legal and its
/// refs live under `refs/remotes/a/b/`), so `a/b/feat` beside remotes `a`
/// and `a/b` is `feat` on `a/b`. A name whose prefix is no remote
/// (`feature/x`, or `nobody/feat`) is not split and goes to git as it is.
/// An empty local part (`upstream/`) is not a branch name and is not split
/// either.
fn split_remote_branch<'a>(name: &'a str, remotes: &[String]) -> Option<(&'a str, &'a str)> {
    let mut best: Option<(&str, &str)> = None;
    for remote in remotes
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(DEFAULT_REMOTE))
    {
        let Some(local) = name
            .strip_prefix(remote)
            .and_then(|rest| rest.strip_prefix('/'))
        else {
            continue;
        };
        if local.is_empty() {
            continue;
        }
        if best.is_none_or(|(found, _)| remote.len() > found.len()) {
            best = Some((&name[..remote.len()], local));
        }
    }
    best
}

/// `split_remote_branch` settled against the repo's own branches: for a
/// remote other than origin, a local branch named by the whole string wins
/// (`refs/heads/<name>` exists, asked exactly through `ref_exists`), so the
/// name is not split and goes to git as it is, which checks that branch
/// out. That is git's own precedence and what master did for those names
/// (a coworker's fork added as remote `alice` beside branches named
/// `alice/<x>` is the realistic collision). `origin/<x>` is always the
/// tracking form, as it has been since ticket 13, whatever local branch
/// exists. `add_worktree`, the skip rule and the `-b` guard all derive
/// through this one function.
fn split_remote_branch_in<'a>(
    repo_path: &Path,
    name: &'a str,
    remotes: &[String],
) -> Option<(&'a str, &'a str)> {
    let (remote, local) = split_remote_branch(name, remotes)?;
    if remote != DEFAULT_REMOTE && ref_exists(repo_path, &format!("refs/heads/{}", name)) {
        return None;
    }
    Some((remote, local))
}

/// The branch name a strategy hands to the `-b` slot of `git worktree add`,
/// exactly as `add_worktree` derives it: a `NewBranch` name verbatim, an
/// `ExistingBranch` name with its `<remote>/` prefix stripped
/// (`split_remote_branch_in`: the local branch git creates to track the
/// remote one), and none for `DetachedHead`. `remotes` is the repo's
/// configured remotes (`remote_names`) and `repo_path` the repo whose local
/// branches settle a collision; a caller with no repo in hand passes an
/// empty list and `None` and gets the `origin/` rule alone, which needs
/// neither.
///
/// One function because two places must agree on it: the guard in
/// `create_worktree_cancellable` and the entry points that ask git whether
/// the name is a branch name. Checking the caller's string instead of this
/// derived one is how `origin/-M` slipped past both: git accepts
/// `origin/-M` as a branch name, but the stripped `-M` is what reaches `-b`,
/// and git's child `git branch` then reads it as force-rename of the
/// checked-out branch of the source repo (reproduced on git 2.50.1).
pub fn branch_slot_name<'a>(
    strategy: &'a BranchStrategy,
    remotes: &[String],
    repo_path: Option<&Path>,
) -> Option<&'a str> {
    let split = match (strategy, repo_path) {
        (BranchStrategy::ExistingBranch(name), Some(repo)) => {
            split_remote_branch_in(repo, name, remotes)
        }
        (BranchStrategy::ExistingBranch(name), None) => split_remote_branch(name, remotes),
        _ => None,
    };
    slot_for(strategy, split)
}

/// The `-b` slot name for a strategy whose existing-branch split has
/// already been settled (`split` is `split_remote_branch_in`'s answer, or
/// `None` for the other strategies). `create_worktree_cancellable` settles
/// the split once, before the pre-create fetch, and hands the same value
/// to this guard, to the skip rule and to the add, so the name the guard
/// accepted is the name the add uses by construction: the fetch can delete
/// a local branch (a refspec writing under `refs/heads/` with
/// `fetch.prune`), and re-deriving after it turned an accepted `alice/-M`
/// into `-b -M` (reproduced: the source repo's checked-out branch was
/// renamed).
fn slot_for<'a>(
    strategy: &'a BranchStrategy,
    split: Option<(&'a str, &'a str)>,
) -> Option<&'a str> {
    match strategy {
        BranchStrategy::NewBranch(name) => Some(name),
        BranchStrategy::ExistingBranch(name) => {
            Some(split.map_or(name.as_str(), |(_, local)| local))
        }
        BranchStrategy::DetachedHead => None,
    }
}

/// `split_remote_branch_in` for an `ExistingBranch` strategy, `None` for
/// the others: the one derivation an attempt makes.
fn existing_split<'a>(
    repo_path: &Path,
    strategy: &'a BranchStrategy,
    remotes: &[String],
) -> Option<(&'a str, &'a str)> {
    match strategy {
        BranchStrategy::ExistingBranch(name) => split_remote_branch_in(repo_path, name, remotes),
        _ => None,
    }
}

/// `create_worktree_with_fetch` that can be stopped at a boundary, for the
/// Creating stage's background worker.
///
/// Cancellation is boundary-only and there are exactly two checkpoints in one
/// repo's attempt: before the pre-create fetch, and before the
/// `git worktree add`. An in-flight git call is never interrupted, because a
/// child killed part-way through the add would leave `.git/worktrees/<name>`
/// registered against an incomplete checkout, which is worse than a complete
/// worktree the user can delete. There is no third checkpoint between
/// `git::detect_base_branch`, the `rev-parse --verify` probes inside
/// `add_worktree` and the add they inform: those probes are local, read-only
/// and instantaneous, so a checkpoint there would only open a window where the
/// attempt aborts after deciding the strategy and before applying it.
///
/// A cancel at checkpoint 1 leaves `fetch: None`, so "the fetch did not run"
/// is observable by the caller. The attempt returned after a cancel is
/// partial and callers that cancelled must discard it, mirroring
/// `sync_repo_cancellable`.
///
/// # Invariant
/// `cancel` is created fresh per run and only ever stored `true`, never
/// cleared. It is therefore monotonic: once checkpoint 2 has fired, a later
/// read by the caller cannot see `false`. That is what lets the worker tell a
/// cancelled attempt from a completed one with a single check after this
/// call returns.
pub fn create_worktree_cancellable(
    repo_path: &Path,
    ws_dir: &Path,
    ws_name: &str,
    strategy: &BranchStrategy,
    fetch: PreCreateFetch,
    cancel: &AtomicBool,
) -> WorktreeAttempt {
    // Both refusals come before `worktree_path` and `create_dir_all`, so a
    // rejected call joins nothing and leaves no directory behind. The name
    // guard is the one place where "no name reaches `Path::join`
    // unvalidated" holds by construction, whichever surface called.
    if let Err(e) = checked_space_name(ws_name) {
        return WorktreeAttempt {
            fetch: None,
            created: Err(e),
        };
    }
    // A leading dash in the name that reaches `-b` (`branch_slot_name`, the
    // derived name, not the caller's string) is refused here without
    // spawning git, because that slot is not protected by `--`: git takes
    // the next argv verbatim and its child `git branch` re-parses it as
    // options, and short options bundle (`-bad` is `-b ad`). The commit-ish
    // and path slots are protected by `--` in `add_worktree` instead. Same
    // sentence as git's so the caller sees one wording whichever layer
    // refused.
    // The split is derived exactly once, here, before the pre-create
    // fetch, and the same value reaches the guard, the skip rule and the
    // add (`slot_for`): the fetch can create or delete a local branch (a
    // refspec writing into `refs/heads/*`, with `fetch.prune`), so a
    // derivation on either side of it could disagree with this one and
    // hand `-b` a name the guard never saw.
    let remotes = remote_names(repo_path);
    let split = existing_split(repo_path, strategy, &remotes);
    if let Some(branch) = slot_for(strategy, split) {
        if branch.starts_with('-') {
            return WorktreeAttempt {
                fetch: None,
                created: Err(anyhow::anyhow!("'{}' is not a valid branch name", branch)),
            };
        }
    }
    let wt_path = worktree_path(ws_dir, ws_name, repo_path);

    if let Err(e) = std::fs::create_dir_all(wt_path.parent().unwrap()) {
        return WorktreeAttempt {
            fetch: None,
            created: Err(e.into()),
        };
    }

    // Detected before the fetch, as it always has been, then moved into the
    // add: the fetch must not change which branch counts as the base.
    let base_branch = git::detect_base_branch(repo_path);

    // Pre-create fetch under the unattended-run policy; a failure is ignored
    // for offline use and creation continues with local refs. Checkpoint 1:
    // a cancelled run does not start one. A strategy whose add reads no
    // remote ref does not start one either (`strategy_reads_origin`): the
    // fetch could not change what gets checked out, and it is silent
    // because there is no age to warn about, unlike the slow-fetch skip.
    let fetch = match fetch {
        PreCreateFetch::Run(_) if cancel.load(Ordering::Relaxed) => None,
        PreCreateFetch::Skip => None,
        PreCreateFetch::Run(_) if !strategy_reads_origin(repo_path, strategy, split) => None,
        PreCreateFetch::Run(limit) => Some(fetch_origin_unattended(repo_path, limit)),
    };

    // Checkpoint 2: the fetch may have taken the whole limit, so the flag is
    // read again before the add rather than once at entry.
    if cancel.load(Ordering::Relaxed) {
        return WorktreeAttempt {
            fetch,
            created: Err(anyhow::anyhow!("worktree creation cancelled")),
        };
    }

    WorktreeAttempt {
        fetch,
        created: add_worktree(repo_path, &wt_path, base_branch, strategy, split),
    }
}

/// Whether `add_worktree` will read an `origin/*` ref for this strategy, and
/// so whether the pre-create fetch can change what it checks out. Read from
/// what each arm of `add_worktree` runs, not from the strategy's name:
///
/// - `NewBranch` probes `refs/remotes/origin/<branch>` and
///   `refs/remotes/origin/<base>`: always.
/// - `ExistingBranch("origin/x")` adds `--track` from
///   `refs/remotes/origin/x`: always.
/// - `ExistingBranch("<remote>/x")` for another configured remote
///   (`split_remote_branch`) adds `--track -b x` from
///   `refs/remotes/<remote>/x`, which `git fetch origin` never writes under
///   the default refspec, and creates `refs/heads/x`, which it never writes
///   either: reads origin iff origin's fetch refspec can write under
///   `refs/remotes/<remote>/` or under `refs/heads/` (`fetch_writes_under`;
///   the second because a fetch that creates `refs/heads/x` turns the add
///   into git's `already exists` refusal).
/// - `ExistingBranch("x")` runs `git worktree add <wt> x`. With a local
///   branch `x` git checks it out at its local tip and never looks at
///   `origin/x`. Without one, git's own DWIM resolves `x` to `origin/x`
///   when exactly one remote has it and adds `--track -b x origin/x`
///   (git 2.50.1, and independent of `worktree.guessRemote`, which governs
///   the no-commit-ish form only). So: reads origin iff no local `x`.
/// - `DetachedHead` adds `--detach` at the source repo's own `HEAD` name as
///   `refs/heads/<name>` (or `HEAD` when detached), a local ref.
///
/// The local-branch test runs before the fetch, which is sound because
/// under the default refspec `git fetch origin` never creates or moves
/// `refs/heads/*`; the sync stage is what fast-forwards local branches, and
/// it has already run. A repo whose fetch refspec does write into
/// `refs/heads/*` (`fetch_writes_under`) has no such fixed point, so it
/// fetches whatever the strategy; the other-remote arm asks the same guard
/// about `refs/remotes/<remote>/` as well. A repo git2 cannot open is
/// treated as reading origin, the side that fetches.
fn strategy_reads_origin(
    repo_path: &Path,
    strategy: &BranchStrategy,
    split: Option<(&str, &str)>,
) -> bool {
    // The remote-tracking namespace the add reads (the other-remote arm
    // only), and the local branch whose presence keeps the add off origin
    // (the plain-name arm only).
    let (tracking_under, local_name): (Option<String>, Option<&str>) = match strategy {
        BranchStrategy::NewBranch(_) => return true,
        BranchStrategy::ExistingBranch(name) => match split {
            Some((remote, _)) if remote == DEFAULT_REMOTE => return true,
            Some((remote, _)) => (Some(format!("refs/remotes/{}/", remote)), None),
            None => (None, Some(name.as_str())),
        },
        BranchStrategy::DetachedHead => (None, None),
    };
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return true;
    };
    if fetch_writes_under(&repo, "refs/heads/") {
        return true;
    }
    if let Some(prefix) = tracking_under {
        if fetch_writes_under(&repo, &prefix) {
            return true;
        }
    }
    match local_name {
        Some(name) => repo.find_branch(name, git2::BranchType::Local).is_err(),
        None => false,
    }
}

/// Whether a `git fetch origin` in this repo can write a ref under
/// `prefix` (`refs/heads/` for a local branch, `refs/remotes/<remote>/` for
/// another remote's tracking branch): true when any `remote.origin.fetch`
/// entry has a destination there, or one git qualifies to it (a destination
/// with no `refs/` prefix, as in `+refs/heads/feat:feat`, creates
/// `refs/heads/feat`; git 2.50.1, reproduced). The default
/// `+refs/heads/*:refs/remotes/origin/*` writes under neither; an entry with
/// no destination (`refs/heads/feat`) fetches into `FETCH_HEAD` only; a
/// negative entry (`^refs/heads/main`) excludes rather than writes. A repo
/// with no `origin` has nothing for the fetch to move; any other failure to
/// read the config takes the side that fetches.
///
/// Read as raw config text, not through git2's parsed refspecs: git2 0.19's
/// `Refspec::dst` unwraps a null destination, so the destination-less form
/// panicked the caller, and libgit2 1.8 rejects a negative refspec, so the
/// remote failed to load and the wildcard beside the negative entry went
/// unseen. Both forms are legal to git and the second is how a mirror-style
/// refspec is made to work in a repo with a checked-out branch.
fn fetch_writes_under(repo: &git2::Repository, prefix: &str) -> bool {
    // Every failure to read falls on the side that fetches, the same
    // default `strategy_reads_origin` takes for a repo git2 cannot open. An
    // absent key (no `origin`, or one with no fetch refspec) is not a
    // failure: the iterator is simply empty, and empty means there is
    // nothing for the fetch to move. Of the four returns below only the
    // non-UTF-8 value is reachable from a repo on disk, and a test pins it.
    // The other three are kept for the invariant, not because a repo can
    // reach them: `Repository::open` already parses the whole config,
    // includes too, so a config that fails to parse fails the open first
    // (the caller's fail-safe), and `multivar` and its iterator fail only
    // on an invalid key name or out of memory (probed on git2 0.19).
    let Ok(config) = repo.config() else {
        return true;
    };
    let Ok(mut entries) = config.multivar("remote.origin.fetch", None) else {
        return true;
    };
    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else {
            return true;
        };
        let Some(spec) = entry.value() else {
            // Not UTF-8: cannot parse.
            return true;
        };
        if refspec_writes_under(spec, prefix) {
            return true;
        }
    }
    false
}

/// One configured fetch refspec: see `fetch_writes_under`. A destination
/// writes under `prefix` when it starts with it, when it has no `refs/`
/// prefix and `prefix` is `refs/heads/` (git qualifies `feat` to
/// `refs/heads/feat`), or when it is a wildcard whose literal part is a
/// prefix of `prefix` (`refs/*`, the mirror form, expands to `refs/heads/*`
/// and `refs/remotes/upstream/*` among others). A negative refspec has no
/// destination by git's own grammar (`^a:b` is `fatal: invalid refspec`),
/// so the no-colon arm carries it and no separate guard is needed.
fn refspec_writes_under(spec: &str, prefix: &str) -> bool {
    let spec = spec.trim();
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    match spec.split_once(':') {
        None => false,
        Some((_, "")) => false,
        Some((_, dst)) => {
            if !dst.starts_with("refs/") {
                return prefix == "refs/heads/";
            }
            if dst.starts_with(prefix) {
                return true;
            }
            match dst.split_once('*') {
                Some((literal, _)) => prefix.starts_with(literal),
                None => false,
            }
        }
    }
}

/// `git worktree add` for one repo, per strategy. Split out of
/// `create_worktree_with_fetch` so its `?` short-circuits into the attempt's
/// `created` without discarding the fetch outcome alongside it.
///
/// Every form passes `--` before the path, so a leading dash in the path or
/// the commit-ish slot is reported by git as `invalid reference: -foo`
/// rather than parsed as an option (which surfaces as `unknown switch` plus
/// the whole usage text). `--` does not protect the value of `-b`; that slot
/// is guarded in `create_worktree_cancellable` before this runs, on the same
/// derived name (`branch_slot_name`) the `ExistingBranch` arm strips here.
/// `split` is the `ExistingBranch` name's settled split (`existing_split`),
/// derived once by the caller before the fetch so the guard and this arm
/// use one name; this function derives nothing itself.
fn add_worktree(
    repo_path: &Path,
    wt_path: &Path,
    base_branch: String,
    strategy: &BranchStrategy,
    split: Option<(&str, &str)>,
) -> Result<PathBuf> {
    let wt = wt_path.to_string_lossy();

    match strategy {
        BranchStrategy::NewBranch(branch_name) => {
            // 1. Local branch exists? Asked of `refs/heads/<name>`, never
            // the bare name, which `rev-parse` resolves to a tag first.
            let local_exists = ref_exists(repo_path, &format!("refs/heads/{}", branch_name));

            // 2. Remote branch exists? Same rule: a tag named `origin/x`
            // is not a remote branch.
            let remote_ref = remote_tracking_ref("origin", branch_name);
            let remote_exists = ref_exists(repo_path, &remote_ref);

            if local_exists {
                // The bare name, deliberately: `git worktree add <wt>
                // <name>` checks out the branch iff `refs/heads/<name>`
                // exists (which the probe just settled), while the
                // qualified `refs/heads/<name>` in this slot is taken as a
                // commit-ish and detaches (git 2.50.1).
                git_worktree_add(&["worktree", "add", "--", &wt, branch_name], repo_path)?;
            } else if remote_exists {
                git_worktree_add(
                    &[
                        "worktree",
                        "add",
                        "--track",
                        "-b",
                        branch_name,
                        "--",
                        &wt,
                        &remote_ref,
                    ],
                    repo_path,
                )?;
            } else {
                // Prefer origin/<base> so the new branch starts at the remote tip rather than
                // a potentially stale local ref. Trade-off: if <base> has unpushed local
                // commits they are NOT included in the new worktree. That is intentional:
                // the sync step guarantees origin/<base> is the freshest shared state.
                // Fall back to local only if the remote ref doesn't exist (offline / no remote).
                // Both start points are fully qualified: a tag named like
                // the base (or like `origin/<base>`) would otherwise make
                // the name ambiguous and the add fail.
                let origin_base = remote_tracking_ref("origin", &base_branch);
                let local_base = base_ref(&base_branch);
                let start_point: &str = if ref_exists(repo_path, &origin_base) {
                    &origin_base
                } else {
                    &local_base
                };
                git_worktree_add(
                    &["worktree", "add", "-b", branch_name, "--", &wt, start_point],
                    repo_path,
                )?;
            }
        }

        BranchStrategy::ExistingBranch(branch_name) => {
            if let Some((remote, local)) = split {
                // A remote-tracking name from the picker, `origin/<x>` or
                // any other configured remote's: a new local `<x>` tracking
                // the remote-tracking ref itself, so a tag named
                // `<remote>/<x>` cannot shadow it. A local `<x>` that
                // already exists is git's refusal (`a branch named '<x>'
                // already exists`), whatever it tracks: the picker lists the
                // local name for whoever wants that one.
                let remote_ref = remote_tracking_ref(remote, local);
                git_worktree_add(
                    &[
                        "worktree",
                        "add",
                        "--track",
                        "-b",
                        local,
                        "--",
                        &wt,
                        &remote_ref,
                    ],
                    repo_path,
                )?;
            } else {
                git_worktree_add(&["worktree", "add", "--", &wt, branch_name], repo_path)?;
            }
        }

        BranchStrategy::DetachedHead => {
            let start_point = base_ref(&base_branch);
            git_worktree_add(
                &["worktree", "add", "--detach", "--", &wt, &start_point],
                repo_path,
            )?;
        }
    }

    Ok(wt_path.to_path_buf())
}

/// Whether the ref named exactly `refname` exists in `repo_path`. Callers
/// pass a fully qualified name (`refs/heads/x`, `refs/remotes/origin/x`).
/// `show-ref --verify` is an exact lookup. `rev-parse --verify` is not, in
/// two ways: on a bare name it resolves a tag before a branch (how a
/// same-named tag used to be taken for a branch, ticket 24); and on a
/// qualified name that does not exist it falls back through git's ref
/// rules, so an absent `refs/heads/x` resolves a tag named
/// `refs/tags/refs/heads/x` (git 2.50.1, pinned by
/// `new_branch_ignores_a_tag_named_like_a_qualified_ref`). A qualified
/// name that exists is unaffected by a same-named tag under either
/// command. `--quiet` only silences the not-found line; the exit status is
/// the answer either way.
fn ref_exists(repo_path: &Path, refname: &str) -> bool {
    spawn::output(
        Command::new("git")
            .args(["show-ref", "--verify", "--quiet", refname])
            .current_dir(repo_path),
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// The remote-tracking ref for `<remote>/<name>`, fully qualified, for the
/// commit-ish slot of `git worktree add` and the probes before it. The
/// remote is a parameter because the `ExistingBranch` arm names whichever
/// remote the picked branch belongs to (ticket 25).
fn remote_tracking_ref(remote: &str, name: &str) -> String {
    format!("refs/remotes/{}/{}", remote, name)
}

/// The fully qualified form of the base `git::detect_base_branch` returns:
/// `refs/heads/<branch>` for a checked-out branch, and `HEAD` itself when
/// the source repo is detached (`refs/heads/HEAD` is an invalid reference,
/// and `HEAD` cannot be a branch name: `git check-ref-format --branch HEAD`
/// refuses it, so the two cases cannot be confused).
fn base_ref(base_branch: &str) -> String {
    if base_branch == "HEAD" {
        base_branch.to_string()
    } else {
        format!("refs/heads/{}", base_branch)
    }
}

/// Remove a workspace: hand every repo worktree back to its source repo with
/// `git worktree remove`, then delete the directory.
///
/// A worktree git refuses to give up (a locked one, or one whose directory
/// was moved, which git does not recognise at its new path) keeps the space:
/// the run continues through the other repos, the space directory is not
/// deleted, and the error names every directory it kept with git's own
/// reason, so a retry once the cause is fixed only revisits what is left.
/// The worktrees git did remove before the refusal are gone, as git removed
/// them. The first line of that error stands alone as a summary, which is
/// all the TUI's one-line status shows. A directory holding a repository of
/// its own, which this app never creates, is kept the same way: deleting it
/// would take its history with it.
///
/// git runs with its output captured (`spawn::output`), never inherited: the
/// parent's stdout is the JSON-RPC stream under MCP and the terminal under
/// the TUI, and git's stderr is the reason in the report.
pub fn remove_workspace(ws_dir: &Path, name: &str, force: bool) -> Result<()> {
    checked_space_name(name)?;
    let ws_path = ws_dir.join(name);
    if !ws_path.exists() {
        anyhow::bail!("workspace '{}' not found", name);
    }

    // Symlinks are not followed: `file_type` reports the link itself, so a
    // symlinked entry is left where master left it, neither classified nor
    // handed to git. Following one would let a link inside the space aim
    // `git worktree remove` at a worktree outside it.
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&ws_path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    // `read_dir` has no defined order, and both the report and which repo a
    // run reaches first would inherit it.
    dirs.sort();

    // Classified in one pass, acted on in the next. Removing a worktree
    // destroys its admin directory, and a second directory pointing at the
    // same one (a worktree copied beside its original) would classify as a
    // worktree before that and as something else after it: an orphan, which
    // was deleted, before ticket 36, and a directory git has no record of,
    // kept with a different reason, since. What a directory is has to be
    // read from the space as it was found.
    let mut classified: Vec<(PathBuf, SpaceEntry)> = dirs
        .into_iter()
        .map(|dir| {
            let kind = classify_space_entry(&dir);
            (dir, kind)
        })
        .collect();

    // Two worktrees in the space that name one admin directory are a worktree
    // and its copy. Two passes are not enough on their own: removing the
    // original still destroys that admin directory and leaves the copy with
    // nothing registered under it, which git can no longer read (a later
    // removal keeps it as `Unregistered`; before ticket 36 it deleted every
    // such copy as an orphan). So neither is handed to git. Both are kept and
    // reported as a pair, and the space is left exactly as it was found,
    // which a retry then finds again.
    //
    // The key is the canonical admin path, computed once per entry: a relative
    // gitdir reaches the same admin directory through different text from
    // each half of a pair. Members are compared by index, not by name.
    let keys: Vec<Option<PathBuf>> = classified
        .iter()
        .map(|(_, kind)| match kind {
            SpaceEntry::Worktree { admin } => {
                Some(std::fs::canonicalize(admin).unwrap_or_else(|_| admin.clone()))
            }
            _ => None,
        })
        .collect();
    let mut groups: std::collections::HashMap<&PathBuf, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        if let Some(key) = key {
            groups.entry(key).or_default().push(index);
        }
    }
    let shared: Vec<Vec<usize>> = groups
        .into_values()
        .filter(|members| members.len() > 1)
        .collect();
    let shared_groups = shared.len();
    let shared_count: usize = shared.iter().map(Vec::len).sum();
    for members in &shared {
        let admin = match &classified[members[0]].1 {
            SpaceEntry::Worktree { admin } => admin.clone(),
            _ => continue,
        };
        let names: Vec<String> = members.iter().map(|&i| name_of(&classified[i].0)).collect();
        let recorded: Vec<RecordedAt> = members
            .iter()
            .map(|&i| recorded_at(&admin, &classified[i].0))
            .collect();
        let original = recorded.iter().position(|r| matches!(r, RecordedAt::Here));
        for (me, &index) in members.iter().enumerate() {
            classified[index].1 = SpaceEntry::Shared {
                reason: pair_reason(me, original, &names, &recorded[me]),
            };
        }
    }

    let mut removed: Vec<String> = Vec::new();
    let mut orphaned: Vec<String> = Vec::new();
    let mut kept: Vec<(String, String)> = Vec::new();
    for (dir, kind) in classified {
        let repo = name_of(&dir);
        match kind {
            SpaceEntry::Shared { reason } => kept.push((repo, reason)),
            // Not a repository of any kind: ordinary content of the space,
            // which goes when the space does, as it always has.
            SpaceEntry::Plain => {}
            // Nothing registered points at this directory any more, and its
            // source repo is gone from where the `.git` file says it is:
            // deleted or moved (`absent_admin`; a record that is gone while
            // the repo is there is `Unregistered`, kept). Git has nothing to
            // unregister, so with `force` the directory goes with the space
            // rather than making the space unremovable.
            //
            // Every caller in the app passes `force` (`cli/remove.rs`, which
            // refuses the command without `--force`, `mcp/mod.rs` and the TUI
            // delete dialog), so the unforced arm is the library surface
            // only. It is here because there is no git left in this case to
            // say whether the directory holds uncommitted work, and silently
            // destroying it is the one thing `force` is supposed to gate.
            SpaceEntry::Orphan => {
                if force {
                    orphaned.push(repo);
                } else {
                    kept.push((
                        repo,
                        "its source repository is gone, so git cannot say whether the \
                         worktree holds uncommitted work; remove the space with force, \
                         or delete the directory by hand"
                            .to_string(),
                    ));
                }
            }
            SpaceEntry::Repository => kept.push((
                repo,
                "holds a git repository of its own, which space did not create; \
                 move it aside or delete it by hand, then remove the space again"
                    .to_string(),
            )),
            SpaceEntry::Unresolved(reason) => kept.push((repo, reason)),
            // No repair hint: for a copy it would hand the original's name to
            // the copy, and there is no record left to repair anyway. The
            // path stays off the first line, which may become the summary.
            SpaceEntry::Unregistered { admin } => kept.push((
                repo,
                format!(
                    "git has no record of it any more, though its source repo is still \
                     there, so what it holds cannot be told\nits .git file names {:?}, \
                     which does not exist; a copy of a worktree looks like this once the \
                     original is removed (a space duplicated with `cp -R`, for instance), \
                     and so does a worktree whose record was pruned or whose source repo \
                     was cloned again; keep what you need from it, delete it by hand, then \
                     remove the space again",
                    admin
                ),
            )),
            SpaceEntry::Unreadable(why) => kept.push((
                repo,
                format!(
                    "{}, so what this directory is cannot be told; look at it, \
                     then move it aside or delete it by hand, and remove the \
                     space again",
                    why
                ),
            )),
            SpaceEntry::Worktree { admin } => match unregister_worktree(&dir, force, &admin) {
                Ok(()) => removed.push(repo),
                Err(reason) => kept.push((repo, reason)),
            },
        }
    }

    if !kept.is_empty() {
        anyhow::bail!(removal_report(
            name,
            &removed,
            &orphaned,
            &kept,
            (shared_count, shared_groups),
        ));
    }

    std::fs::remove_dir_all(&ws_path)
        .with_context(|| format!("removing workspace directory {}", ws_path.display()))?;
    Ok(())
}

/// What a directory inside a space is, as far as removing it goes.
enum SpaceEntry {
    /// A linked worktree whose admin directory (`<source>/.git/worktrees/<id>`)
    /// is still there, so git can unregister it.
    Worktree { admin: PathBuf },
    /// A `.git` file whose target has gone, and whose source repo has gone
    /// with it (`absent_admin`).
    Orphan,
    /// A `.git` file whose target has gone while its source repo is still
    /// there: git has no record of this directory any more. A copy of a
    /// worktree whose original was removed looks like this, and nothing on
    /// disk ties it to that original, so it is kept whatever `force` says.
    Unregistered { admin: PathBuf },
    /// A repository in its own right rather than a worktree: a clone dropped
    /// in the space by hand, a bare repo, or a submodule checkout, whose
    /// `.git` file points into `<host>/.git/modules/`.
    Repository,
    /// Something claims to be a repository and cannot be read. Saying what it
    /// is would be a guess, and the guess that deletes is the wrong one.
    Unreadable(String),
    /// A worktree that names the same admin directory as another directory in
    /// the space: a worktree and its copy. Neither is handed to git; the
    /// reason is decided for the whole group at once (`pair_reason`).
    Shared { reason: String },
    /// A relative gitdir that does not resolve: a moved space or a deleted
    /// source repo, which nothing on disk tells apart. Kept, with a reason
    /// that already says what to do in each case.
    Unresolved(String),
    /// Ordinary content: no repository here.
    Plain,
}

/// Which of those a directory is, decided before anything is spawned or
/// deleted, from git's own layout on disk.
///
/// Deliberately not from libgit2, which `placement_of` also stopped asking
/// (it reads this same classification). This decides whether to delete
/// a directory, and libgit2 refuses any repository format extension its
/// version does not know. `git init --ref-format=reftable` (git 2.45 and
/// later) writes `extensions.refstorage`, and `worktree.useRelativePaths`
/// (git 2.48 and later) writes `extensions.relativeWorktrees` into the source
/// repo, so asking libgit2 made a supported repository either "not a
/// repository", which deletes it, or "unreadable", which makes the space
/// unremovable. The layout this reads instead is git's own and does not move
/// with a format extension.
fn classify_space_entry(dir: &Path) -> SpaceEntry {
    let gitfile = dir.join(".git");
    if gitfile.is_file() {
        let (admin, relative) = match worktree_admin_dir(dir) {
            Ok(gitdir) => (gitdir.path, gitdir.relative),
            Err(why) => return SpaceEntry::Unreadable(why),
        };
        // Only a directory that is provably not there is an orphan, because
        // an orphan is deleted. Any other answer, a permission error on the
        // way to it or a name that resolves to something else, is a question
        // this cannot answer, and the answer that deletes is the wrong guess.
        return match std::fs::symlink_metadata(&admin) {
            // A relative gitdir breaks when either side moves, so for one a
            // `NotFound` cannot tell "the source repo is gone" from "this
            // space was moved". An absolute gitdir survives a move of the
            // worktree, which is why only its `NotFound` is read as an orphan.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && relative => {
                // The path stays off the first line, which may become the
                // summary the TUI shows.
                SpaceEntry::Unresolved(format!(
                    "its .git file names a relative gitdir that no longer leads anywhere, \
                     which moving the space or its source repo leaves behind as well as \
                     deleting the source repo, so it is kept\nit points at {:?}, which does \
                     not exist; if the space or the source repo was moved, run `git worktree \
                     repair {:?}` from the source repo; if the source repo is gone, delete \
                     this directory by hand; then remove the space again",
                    admin, dir
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => absent_admin(admin),
            Err(e) => SpaceEntry::Unreadable(format!(
                "its .git file names {:?}, which cannot be read ({})",
                admin, e
            )),
            Ok(meta) if !meta.is_dir() => SpaceEntry::Unreadable(format!(
                "its .git file names {:?}, which is not a directory",
                admin
            )),
            // A linked worktree's admin directory holds `commondir` and
            // `gitdir` and keeps its objects in the repo it belongs to. A
            // submodule checkout's `.git` file points instead at
            // `<host>/.git/modules/<name>`, a repository with its own
            // `objects` and `config` and no `commondir`. That is the trap
            // `placement_of` relies on, read from the directory itself.
            //
            // Asked with the error kept, as the admin directory itself was:
            // `is_file` reads a permission error as "not there", and an admin
            // directory that exists but cannot be searched would then be
            // called a repository of its own, with advice to delete a live
            // worktree by hand.
            Ok(_) => match std::fs::symlink_metadata(admin.join("commondir")) {
                Ok(meta) if meta.is_file() => SpaceEntry::Worktree { admin },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => SpaceEntry::Repository,
                Err(e) => SpaceEntry::Unreadable(format!(
                    "its .git file names {:?}, which cannot be read ({})",
                    admin, e
                )),
                Ok(_) => SpaceEntry::Unreadable(format!(
                    "its .git file names {:?}, whose commondir is not a file",
                    admin
                )),
            },
        };
    }
    // No `.git` file: a clone has a `.git` directory, and a bare repo has no
    // `.git` at all, its files sitting at the top level.
    if gitfile.is_dir() || is_repository_dir(dir) {
        return SpaceEntry::Repository;
    }
    SpaceEntry::Plain
}

/// What a directory is when the absolute gitdir its `.git` file names is not
/// there. An admin directory that is gone proves only that git no longer has
/// a record of this directory, and a copy of a worktree is in exactly that
/// state once its original is removed, by this app or by hand (a space
/// duplicated with `cp -R`, ticket 36), as is a copy edited by hand to an
/// admin id git never registered. What makes a directory an orphan, which is
/// deleted, is that its source repo is gone too, so that is what is checked.
///
/// git writes a linked worktree's gitdir as `<common>/worktrees/<id>`, from
/// the real path of the common dir: absolute, every component a plain name,
/// no line break in any of them. Only that shape says where the source repo
/// is (`common_dir_in_gits_shape`), and only a `NotFound` for `<common>`
/// itself proves it is gone. `<common>/worktrees` is no evidence: git deletes
/// it with the repo's last worktree. A path in any other shape was written by
/// hand, and where its repo would be cannot be read from it: a `..` walks
/// through a `worktrees` directory that may have gone, so the kernel reports
/// `NotFound` for a `<common>` that is there, and a gitfile's second line,
/// which git reads as part of the path, can end in `worktrees/<id>` under a
/// made-up `<common>` (the code review of PR #61 found such a copy deleted).
/// Both are kept.
///
/// A `<common>` that is there is said to be the source repo only when it is
/// a repository; anything else there is kept with what it is. After the
/// admin directory itself answered `NotFound`, every directory on its path
/// down to the missing one was searchable, so `<common>` answers `Ok` or
/// `NotFound` but for a race; any other error keeps, as everywhere a
/// deletion is decided. Paths stay off each reason's first line, which may
/// become the summary the TUI shows.
fn absent_admin(admin: PathBuf) -> SpaceEntry {
    let Some(common) = common_dir_in_gits_shape(&admin) else {
        return SpaceEntry::Unreadable(format!(
            "its .git file names a path that does not exist, in a form git does not write \
             for a worktree\nthe path is {:?}",
            admin
        ));
    };
    match std::fs::symlink_metadata(common) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SpaceEntry::Orphan,
        Ok(_) if is_repository_dir(common) => SpaceEntry::Unregistered { admin },
        Ok(_) => SpaceEntry::Unreadable(format!(
            "its .git file names a worktree of a directory that is not a git repository\n\
             the path is {:?}",
            admin
        )),
        Err(e) => SpaceEntry::Unreadable(format!(
            "its .git file names a worktree of a directory that cannot be read ({})\n\
             the path is {:?}",
            e, admin
        )),
    }
}

/// `<common>` in an admin path of the form git writes for a linked worktree,
/// `<common>/worktrees/<id>`, or `None` when the path is not of that form: an
/// allow-list (the root, then plain names with no `\n` or `\r`), since what
/// a hand edit can put in a gitfile is not a list anyone can finish.
fn common_dir_in_gits_shape(admin: &Path) -> Option<&Path> {
    let plain = admin.components().all(|c| match c {
        std::path::Component::RootDir => true,
        std::path::Component::Normal(name) => !name
            .as_encoded_bytes()
            .iter()
            .any(|b| matches!(b, b'\n' | b'\r')),
        _ => false,
    });
    let worktrees = admin.parent()?;
    if !plain || worktrees.file_name() != Some("worktrees".as_ref()) {
        return None;
    }
    worktrees.parent()
}

/// Whether `dir` is itself a repository, by what every repository has
/// whatever its format or ref backend: `git init`, `git init --bare` and
/// `git init --ref-format=reftable` all produce `objects`, `config` and
/// `HEAD`. Either signal is enough, so a repository that has lost one of
/// them is still recognised. Erring towards "yes" keeps a directory and
/// reports it; erring towards "no" deletes it, so this side of the line is
/// the safe one.
fn is_repository_dir(dir: &Path) -> bool {
    dir.join("objects").is_dir() || (dir.join("config").is_file() && dir.join("HEAD").is_file())
}

/// The admin directory a linked worktree's `.git` file names.
///
/// Read by git's rule (`setup.c`, `read_gitfile_gently`): the prefix
/// `gitdir: ` is exact and at the very start, and the path is everything
/// after it with only trailing `\n` and `\r` removed. Anything else is part
/// of the path, interior newlines and trailing blanks included, because a
/// directory name may end in a blank: git made a worktree of a repo named
/// with a trailing non-breaking space, and reads its `.git` file. Checked
/// against git 2.50.1 with `git rev-parse --git-dir`, which accepts the
/// canonical form, no trailing newline, CRLF and a blank second line, and
/// rejects a leading space, two spaces after the colon, trailing spaces that
/// are not part of the path and a trailing comment line.
///
/// When git's reading names nothing that exists, that is not yet proof the
/// source repo is gone, which is what deletes. The near misses a hand edit
/// or a lax writer produces are tried first: the path trimmed at both ends,
/// the first line alone, and the first line trimmed. If any of those names
/// something real, the file is not what git reads and which was meant cannot
/// be told, so it is reported and kept. Only when every reading names
/// nothing is git's path returned, for the caller to find absent. An earlier
/// version trimmed all trailing whitespace instead, which looked safe and was
/// not: it read a real path ending in a blank as a path that does not exist.
///
/// git writes a relative path when the user sets `worktree.useRelativePaths`
/// (git 2.48 and later) and reads it relative to the worktree, which is what
/// this does: resolving it against the process's own working directory
/// instead finds nothing.
///
/// Asking the git binary instead, the third option after libgit2 and this,
/// was rejected: `git rev-parse` walks up out of the directory it is given,
/// so a plain directory inside a space that sits anywhere under a repository
/// answers with that repository's git dir, and the call would have to be
/// fenced with `GIT_CEILING_DIRECTORIES` and its answer compared with what
/// was expected anyway. It also costs a spawn per directory.
fn worktree_admin_dir(dir: &Path) -> std::result::Result<Gitdir, String> {
    let content = std::fs::read_to_string(dir.join(".git"))
        .map_err(|e| format!("its .git file cannot be read ({})", e))?;
    let rest = content
        .strip_prefix("gitdir: ")
        .ok_or_else(|| "its .git file does not name a gitdir".to_string())?;
    let as_git_reads_it = rest.trim_end_matches(['\n', '\r']);
    let relative = !Path::new(as_git_reads_it).is_absolute();
    let resolved = resolve_against(dir, as_git_reads_it);
    if resolved.exists() {
        return Ok(Gitdir {
            path: resolved,
            relative,
        });
    }
    let first_line = as_git_reads_it
        .split('\n')
        .next()
        .unwrap_or(as_git_reads_it)
        .trim_end_matches('\r');
    let near_misses = [as_git_reads_it.trim(), first_line, first_line.trim()];
    if near_misses
        .iter()
        .any(|miss| *miss != as_git_reads_it && resolve_against(dir, miss).exists())
    {
        return Err(
            "its .git file names a live admin directory in a form git does not read \
             (extra lines, or blanks around the path)"
                .to_string(),
        );
    }
    Ok(Gitdir {
        path: resolved,
        relative,
    })
}

/// What a linked worktree's `.git` file names, and whether it named it by a
/// relative path, which decides how much a `NotFound` for it proves.
struct Gitdir {
    path: PathBuf,
    relative: bool,
}

fn resolve_against(dir: &Path, target: &str) -> PathBuf {
    let target = Path::new(target);
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        dir.join(target)
    }
}

/// Where the source repo's record of this worktree says it lives, compared
/// with where this directory is. The admin directory's `gitdir` file holds
/// the path of the worktree's own `.git` file, so this answers without
/// reading git's sentence.
enum RecordedAt {
    /// The record names this directory.
    Here,
    /// The record cannot be read or resolved, which is not evidence of
    /// anything: git's reason stands as git gave it.
    Unknown,
    /// The record names a place that is not there at all. That is what moving
    /// or renaming the space leaves behind, and `git worktree repair` fixes.
    Gone(PathBuf),
    /// The record names another directory that exists. That is not a move:
    /// the worktree git knows is still there, and this is a copy of it, for
    /// instance a space duplicated with `cp -R`. `git worktree repair` here
    /// would hand the original's registration to the copy, and removing the
    /// copy would then delete the admin directory the original still uses.
    Elsewhere(PathBuf),
}

fn recorded_at(admin: &Path, dir: &Path) -> RecordedAt {
    let recorded = match std::fs::read_to_string(admin.join("gitdir")) {
        Ok(recorded) => PathBuf::from(recorded.trim_end_matches(['\n', '\r'])),
        Err(_) => return RecordedAt::Unknown,
    };
    let recorded = if recorded.is_absolute() {
        recorded
    } else {
        admin.join(recorded)
    };
    let here = match std::fs::canonicalize(dir) {
        Ok(here) => here,
        Err(_) => return RecordedAt::Unknown,
    };
    let recorded_dir = recorded.parent().unwrap_or(&recorded).to_path_buf();
    match std::fs::canonicalize(&recorded_dir) {
        Ok(there) if there == here => RecordedAt::Here,
        Ok(there) => RecordedAt::Elsewhere(there),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RecordedAt::Gone(recorded_dir),
        Err(_) => RecordedAt::Unknown,
    }
}

/// Hand one worktree back to its source repo, reporting git's reason when it
/// refuses. git runs inside the worktree itself, so git resolves the source
/// repo: that covers a relative `gitdir:` and a bare source repo, which
/// tracing the `.git` file back by hand does not.
///
/// `--force` is never doubled for a lock, with one exception: a worktree
/// git never finished (`half_built`), still locked from its add with the
/// checkout's `index.lock` and no `index`. That lock is not the user's,
/// the tree holds nothing git handed over, and `force` has already
/// consented to losing whatever the tree holds, so a forced removal passes
/// `--force --force` for that worktree alone. It is what lets a user who
/// lost power mid-create remove the space and create it again with no git
/// command; the create side refuses to adopt such a worktree
/// (`placement_of`) and names this as the way out.
fn unregister_worktree(dir: &Path, force: bool, admin: &Path) -> std::result::Result<(), String> {
    let target = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let target = target.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    let overriding = force && half_built(admin);
    if force {
        args.push("--force");
    }
    if overriding {
        args.push("--force");
    }
    // Belt and braces: what git gets is the canonicalised path, so a repo
    // directory named `-x` is already inside an absolute path. The separator
    // is what still holds if the canonicalisation above ever falls back to
    // the path as given. `git worktree add` in this module passes it too.
    args.push("--");
    args.push(&target);

    match spawn::output(Command::new("git").args(&args).current_dir(dir)) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let reason = stderr.trim();
            let reason = if reason.is_empty() {
                "git reported no error output"
            } else {
                reason
            };
            // Both remedies are read from the admin directory rather than
            // from git's wording, which changes with git's version and the
            // user's language. A locked worktree has a `locked` file; a
            // worktree whose directory was moved has an admin `gitdir` file
            // still naming the old path, which is what git refuses on.
            let first = reason.lines().next().unwrap_or(reason);
            // A half-built worktree was already given the second force, so
            // the unlock hint would be wrong for it; git's reason stands
            // (seen: `Directory not empty` while the killed add's checkout
            // child was still writing, after which the admin directory is
            // gone and the next removal keeps what is left as `Unregistered`,
            // or deletes it as plain content if its `.git` file went too).
            Err(if admin.join("locked").exists() && !overriding {
                // git's sentence, then space's own way out. git's remaining
                // lines are dropped here, and only here: they end in
                // `remove -f -f`, which space does not offer and will not,
                // since the lock is the user's. Two instructions for one
                // problem, one of them unreachable, is worse than one.
                format!(
                    "{}\nspace does not override a lock: run `git worktree unlock {:?}`, \
                     then remove the space again",
                    first, dir
                )
            } else {
                match recorded_at(admin, dir) {
                    RecordedAt::Gone(_) => format!(
                        "{}\nits source repo still points at where it used to be: run \
                         `git worktree repair {:?}`, then remove the space again",
                        first, dir
                    ),
                    // No repair hint: see `RecordedAt::Elsewhere`.
                    RecordedAt::Elsewhere(there) => format!(
                        "it is a copy of a worktree git still knows, and this is not\nthat \
                         worktree is at {:?}; keep what you need from this copy, then \
                         delete it by hand and remove the space again (removing the space \
                         again as it is keeps this copy, also once that worktree is gone, \
                         while its source repo is there)\n{}",
                        there, first
                    ),
                    RecordedAt::Here | RecordedAt::Unknown => reason.to_string(),
                }
            })
        }
        Err(e) => Err(format!("could not run git: {}", e)),
    }
}

/// The error a kept space reports. The first line stands alone as a summary,
/// since the TUI's one-line status shows only that, and a status row at the
/// supported 80 columns is clipped without an ellipsis: it leads with the
/// directories that were kept and the space they are in, which is what the
/// user has to act on, and ends with the first reason, introduced so that
/// nothing in it can read as part of git's own sentence. The lines under it
/// carry every reason in full, indented, the count against every repository
/// the space held (directories holding no repository are not counted, and go
/// with the space), what was removed before the run reached the rest, and
/// the orphans, which are still there and go with the space once nothing is
/// kept.
///
/// Directory names print with `{:?}`, the convention `checked_space_name`
/// sets in this module, so a name carrying a newline or an escape sequence
/// cannot break the summary line apart or reach a terminal as control
/// characters.
fn removal_report(
    name: &str,
    removed: &[String],
    orphaned: &[String],
    kept: &[(String, String)],
    (shared, groups): (usize, usize),
) -> String {
    let total = removed.len() + orphaned.len() + kept.len();
    let names: Vec<String> = kept.iter().map(|(repo, _)| format!("{:?}", repo)).collect();
    // `kept.first()` rather than `kept[0]`: the one call site only reports
    // when something was kept, and this does not depend on it staying so.
    let first_reason = kept
        .first()
        .and_then(|(_, reason)| reason.lines().next())
        .unwrap_or_default();
    // Two names, then a count: a space with many kept repos must not push
    // the reason off the end of a status row.
    let listed = match names.len() {
        0..=2 => names.join(", "),
        n => format!("{} and {} more", names[..2].join(", "), n - 2),
    };
    // A kept pair is the one thing in a report whose consequence reaches past
    // this run, so its count leads the line. After the names it sat at column
    // 79 of an 80-column status row once the names were ordinary ones.
    let pairs = match (shared, groups) {
        (0, _) => String::new(),
        (n, 1) => format!("{} share one worktree; ", n),
        (n, g) => format!("{} share {} worktrees; ", n, g),
    };
    let mut report = format!(
        "{}could not remove {} from space '{}'; first reason: {}",
        pairs, listed, name, first_reason
    );
    report.push_str(&format!(
        "\n  {} of {} repos in the space were kept",
        kept.len(),
        total
    ));
    for (repo, reason) in kept {
        let reason = reason
            .lines()
            .collect::<Vec<_>>()
            .join("\n      ")
            .to_string();
        report.push_str(&format!("\n  {:?}: {}", repo, reason));
    }
    if !removed.is_empty() {
        report.push_str(&format!("\n  removed: {}", quoted(removed)));
    }
    // An orphan is deleted only with the space, and a report means the space
    // was kept, so an orphan listed here is still on disk.
    if !orphaned.is_empty() {
        report.push_str(&format!(
            "\n  no source repository left to unregister, so they go with the space \
             once nothing is kept: {}",
            quoted(orphaned)
        ));
    }
    report
}

/// What one member of a group sharing an admin directory is told. Which
/// member is the original is not assumed: it is the one at the place the
/// admin directory's record names, if any is. With none there, the space was
/// moved (the record names nowhere) or every member is a copy of a worktree
/// outside the space (the record names that), and all of them are told the
/// same thing. Paths stay off the first line, which may become the summary
/// the TUI shows, and print with `{:?}`, as names do.
fn pair_reason(
    me: usize,
    original: Option<usize>,
    names: &[String],
    recorded: &RecordedAt,
) -> String {
    let others: Vec<String> = names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != me)
        .map(|(_, n)| n.clone())
        .collect();
    let listed = quoted(&others);
    let one = others.len() == 1;
    match original {
        Some(o) if o == me => {
            let (copies, share, none, them) = if one {
                ("a copy of it", "shares", "neither", "the copy")
            } else {
                ("copies of it", "share", "none of them", "the copies")
            };
            format!(
                "{} in this space ({}) {} its admin directory, so {} is removed\n\
                 removing it would leave {} with nothing registered, which git could no \
                 longer read; keep what you need from {}, delete {} by hand, then remove the \
                 space again",
                copies, listed, share, none, them, them, them
            )
        }
        Some(o) => format!(
            "it is a copy of {:?} in this space, sharing its admin directory, and git does \
             not know it\nkeep what you need from it, delete it by hand, then remove the \
             space again",
            names[o]
        ),
        None => match recorded {
            RecordedAt::Gone(was) => format!(
                "it and {} share one admin directory whose worktree is no longer where git \
                 recorded it, so none of them is removed\ngit recorded it at {:?}: run \
                 `git worktree repair <path>` from the source repo for the one that is the \
                 real worktree, keep what you need from the others, delete them by hand, \
                 then remove the space again",
                listed, was
            ),
            RecordedAt::Elsewhere(there) => format!(
                "it and {} are copies of one worktree outside this space and share its \
                 admin directory, so none of them is removed\nthe worktree git knows is at \
                 {:?}; keep what you need from these copies, delete them by hand, then remove \
                 the space again",
                listed, there
            ),
            RecordedAt::Here | RecordedAt::Unknown => format!(
                "it and {} share one admin directory, and which of them git knows cannot be \
                 read, so none of them is removed\nkeep what you need, delete all but the \
                 real worktree by hand, then remove the space again",
                listed
            ),
        },
    }
}

fn name_of(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("{:?}", n))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // fixtures start git directly (ADR 0002)
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

    /// A one-commit repo on `main` at `<parent>/<name>`.
    fn plain_repo(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&path)
            .output()
            .unwrap();
        git_setup(&path);
        git(&["commit", "--allow-empty", "-m", "init"], &path);
        path
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
            FetchOutcome::Failed {
                exit_code, stderr, ..
            } => {
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
        cmd.args(["-c", "trap '' TERM; sleep 0.3; echo late >&2; exit 0"]);

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
            elapsed < UNATTENDED_KILL_GRACE,
            "the grace must end when the child exits, took {:?}",
            elapsed
        );
    }

    /// The same child that never exits is still a timeout, ended by the
    /// SIGKILL after the grace (it ignores SIGTERM). This pins the contract;
    /// the regression guard for a helper outliving a leader that did exit on
    /// SIGTERM is `sync_repo_timeout_kills_helper_that_ignores_sigterm`.
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
            elapsed < UNATTENDED_KILL_GRACE + Duration::from_secs(2),
            "the child must be killed once the grace ends, took {:?}",
            elapsed
        );
    }

    /// `run_git_unattended` pins `LC_ALL=C` on the child, as the three other
    /// stderr-parsing helpers do, because the sync report's DETAIL column
    /// and the Creating log's fetch note strip git's English prefix, which
    /// git translates. The test process carries
    /// `LC_ALL=de_DE.UTF-8` while the fetch runs, so the child's `C` can only
    /// have come from the helper, not from an unset inheritance.
    ///
    /// Two assertions on one fetch. The upload-pack the fetch runs is a
    /// script that prints its own `LC_ALL` to stderr and exits 1, and that
    /// line is the proof on every git: the environment the helper set is what
    /// git handed its child. git's own `Could not read from remote
    /// repository.` after it is the English sentence the readers parse; on a
    /// gettext git it is what the override keeps English, on Apple git
    /// (built without gettext) it is English either way, so it is pinned but
    /// proves nothing about the override there.
    ///
    /// The locale goes on the test process by re-running this test binary on
    /// this one test with `LC_ALL=de_DE.UTF-8` and a marker in its
    /// environment: a `set_var` in the multi-threaded test process would
    /// leak the locale into every other test's git spawn. With the marker
    /// set, the test runs the fetch and asserts; without it, it runs the
    /// child and fails with the child's output if the child failed or ran
    /// anything other than this one test. That child is started through the
    /// spawn gate: it lives for a whole test run, far longer than a fixture
    /// `git` call, so a pipe it inherited from another thread's spawn would
    /// hold that thread's reader for as long (`core::spawn`).
    #[test]
    fn run_git_unattended_pins_lc_all_to_c_for_the_child() {
        const MARKER: &str = "SPACE_TEST_LC_ALL_INNER";
        if std::env::var_os(MARKER).is_none() {
            // libtest names a test by its path inside the crate, so the crate
            // segment goes; the lib and the bin target both compile this
            // module and both are named `space`, so the rest is the same
            // name in either binary. A renamed target fails loudly below.
            let (_, in_crate) = module_path!().split_once("::").unwrap();
            let test_name = format!(
                "{}::run_git_unattended_pins_lc_all_to_c_for_the_child",
                in_crate
            );
            let out = crate::core::spawn::output(
                Cmd::new(std::env::current_exe().unwrap())
                    .args(["--exact", &test_name, "--test-threads=1", "--nocapture"])
                    .env("LC_ALL", "de_DE.UTF-8")
                    .env(MARKER, "1"),
            )
            .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success(),
                "the fetch under LC_ALL=de_DE.UTF-8 failed:\n{}{}",
                stdout,
                String::from_utf8_lossy(&out.stderr)
            );
            // A filter that matches nothing exits green with zero tests run,
            // so the child's success counts only when it ran this one test.
            assert!(
                stdout.contains("test result: ok. 1 passed;"),
                "the child run must have run exactly this test, got\n{}",
                stdout
            );
            return;
        }
        assert_eq!(
            std::env::var("LC_ALL").as_deref(),
            Ok("de_DE.UTF-8"),
            "the inner run must carry the locale the override has to beat"
        );

        let tmp = tempfile::tempdir().unwrap();
        Cmd::new("git")
            .args(["init", "--bare", "-b", "main", "origin.git"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let local = plain_repo(tmp.path(), "local");
        let origin_url = format!("file://{}", tmp.path().join("origin.git").display());
        git(&["remote", "add", "origin", &origin_url], &local);
        let script = tmp.path().join("upload-pack-prints-lc-all.sh");
        std::fs::write(&script, "echo \"LC_ALL=[$LC_ALL]\" >&2\nexit 1\n").unwrap();
        // git hands the value to `sh -c`, so the path is quoted against a
        // TMPDIR with a space in it.
        git(
            &[
                "config",
                "remote.origin.uploadpack",
                &format!("/bin/sh '{}'", script.display()),
            ],
            &local,
        );

        let outcome = run_git_unattended(
            &["fetch", "--quiet", "origin"],
            &local,
            Duration::from_secs(20),
        );
        let Unattended::Exited { status, stderr } = outcome else {
            panic!("expected the fetch to exit on its own, got {:?}", outcome);
        };
        assert_eq!(
            status.code(),
            Some(128),
            "git's exit for a failed remote\n{}",
            stderr
        );
        assert!(
            stderr.contains("LC_ALL=[C]"),
            "git's child must see LC_ALL=C, got\n{}",
            stderr
        );
        assert!(
            stderr.contains("Could not read from remote repository."),
            "git's own line must be the English one, got\n{}",
            stderr
        );
    }

    /// An unattended run starts its child behind the spawn gate, so the child
    /// cannot inherit a pipe another thread made and has not yet marked
    /// close-on-exec. If it could, its own `spawn` could block for another
    /// child's lifetime before the limit even started (ticket 19).
    #[test]
    fn an_unattended_run_cannot_inherit_a_pipe_made_under_the_gate() {
        let held = crate::core::spawn::tests::a_child_started_in_the_gap_holds_the_pipe(|child| {
            run_unattended(child, Duration::from_secs(20));
        });
        assert!(
            !held,
            "a child started by `run_unattended` kept the pipe open"
        );
    }

    /// A git that cannot be started is a `Failed` with no exit code whose
    /// stderr carries `SPAWN_FAILURE_PREFIX`, which the report renders as
    /// `git did not start`. The repo directory does not exist, so spawn fails
    /// on the working directory before git is ever looked up.
    #[test]
    fn sync_repo_reports_unstartable_git_with_spawn_failure_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("no-such-repo");

        let result = sync_repo(&missing);

        match &result.fetch {
            FetchOutcome::Failed {
                exit_code, stderr, ..
            } => {
                assert_eq!(*exit_code, None, "a git that never ran has no exit code");
                assert!(
                    stderr.starts_with(SPAWN_FAILURE_PREFIX),
                    "stderr must start with {:?}, got: {:?}",
                    SPAWN_FAILURE_PREFIX,
                    stderr
                );
            }
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        }
        assert!(
            !result.fetch.is_slow(),
            "a git that never started cost nothing, so the creation fetches \
             this repo itself: {:?}",
            result.fetch
        );
        assert!(
            result.forwarded.is_empty() && result.skipped.is_empty(),
            "no branch work happens when git did not start"
        );
    }

    /// A fetch that failed carries how long it took, which is what the
    /// pre-create skip rule reads. Two fetches fail without touching the
    /// network: a slow one through a `file://` origin whose upload-pack
    /// holds it open until the test releases it, and a fast one in a repo
    /// with no remote at all, run start to finish inside that hold. Files
    /// order the two, not sleeps, so every assertion compares one
    /// measurement with another and none says how fast this machine is. A
    /// fixed 1s ceiling here was reported failing at 2.3s with other tests
    /// running (ticket 18): load stretches every spawn, and a child spawned
    /// on another thread at the same moment can stretch this run's recorded
    /// time (ticket 19). The one bound left is the 20s fetch limit, a backstop
    /// for a hold that never ends. The comparisons pin `is_slow_with` rather
    /// than `SLOW_FETCH_THRESHOLD` itself: a test that waited five seconds
    /// to check the constant would cost more than the delay this rule
    /// removes.
    #[test]
    fn a_failed_fetch_records_how_long_it_took() {
        let (tmp, local) = make_behind_repo();
        // The upload-pack is the shared hold (`spawn::hold`): it holds until
        // the test releases it and records that it saw the release, giving up
        // without that record once the test can no longer release it or
        // after its cap. git runs in its own session, so a killed test binary
        // leaves nothing else to stop it, and the 20 s fetch limit below dies
        // with the test process; the pid guard is what ends it then. The cap
        // sits well above that limit, so the limit fires first and says so.
        // A released upload-pack still fails the fetch: that is the outcome
        // under test.
        let hold = super::spawn::hold::Hold::new(tmp.path(), "upload-pack");
        let script = tmp.path().join("held-failing-upload-pack.sh");
        hold.write_script(&script, "exit 1");
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

        // A repo with no remote at all: git fails before any upload-pack runs.
        let fast_tmp = tempfile::tempdir().unwrap();
        let fast_repo = plain_repo(fast_tmp.path(), "no-remote");

        let slow_run = std::thread::spawn(move || {
            sync_repo_with_timeout(&local, Duration::from_secs(20)).fetch
        });
        hold.wait_holding(
            Duration::from_secs(60),
            "the slow fetch's upload-pack never started holding",
            || {
                slow_run
                    .is_finished()
                    .then(|| "the slow fetch ended before its upload-pack held it".to_string())
            },
        );
        let window_started = Instant::now();
        let fast = sync_repo_with_timeout(&fast_repo, Duration::from_secs(20)).fetch;
        let window = window_started.elapsed();
        hold.release();
        let slow = slow_run.join().unwrap();
        // Everything below rests on the slow fetch still being held when the
        // window closed, so check that from the upload-pack's side rather
        // than by timing: it saw `release`, which is written after the window.
        assert!(
            hold.saw_release(),
            "the slow fetch's upload-pack must hold until the release (a \
             TimedOut here means the hold outlasted the fetch limit): {:?}",
            slow
        );

        let slow_elapsed = match &slow {
            FetchOutcome::Failed { elapsed, .. } => *elapsed,
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        };
        let fast_elapsed = match &fast {
            FetchOutcome::Failed { elapsed, .. } => *elapsed,
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        };
        // Measured, not stamped. The slow fetch was held open across the
        // whole window, so it recorded more than the window; the fast one
        // ran inside it, so it recorded no more. Zero fails the first, and a
        // constant cannot be both above the window and at or below it.
        assert!(
            slow_elapsed > window,
            "the slow fetch was held open for the {:?} the fast one took, so \
             it must record more, got {:?}",
            window,
            slow_elapsed
        );
        assert!(
            fast_elapsed <= window,
            "the fast fetch cannot record more than its call took ({:?}), got {:?}",
            window,
            fast_elapsed
        );

        // The comparison on a real outcome, at the elapsed it recorded and a
        // nanosecond either side of it: the rule is at or above. The slow
        // elapsed is above the window, so taking a nanosecond off cannot
        // underflow.
        let nanosecond = Duration::from_nanos(1);
        assert!(
            slow.is_slow_with(slow_elapsed - nanosecond),
            "a failure is slow against a threshold under what it took: {:?}",
            slow
        );
        assert!(
            slow.is_slow_with(slow_elapsed),
            "a failure that took exactly the threshold is slow: {:?}",
            slow
        );
        assert!(
            !slow.is_slow_with(slow_elapsed + nanosecond),
            "a failure is not slow against a threshold over what it took: {:?}",
            slow
        );
        // The rule is how long it took, not that it failed: at a threshold
        // the slow failure meets, the fast one is still fetched again.
        assert!(
            !fast.is_slow_with(slow_elapsed),
            "a fast failure is fetched again at a threshold the slow one meets: \
             {:?}",
            fast
        );
    }

    /// Every unattended run passes through `join_bounded` once its child has
    /// exited, so a wait there lands in every recorded `elapsed`: it must
    /// return as soon as the reader has finished, not when the limit runs
    /// out. The limit is far beyond anything a finished thread needs, so the
    /// assertion fails only when the whole limit ran. Before ticket 18 only a
    /// 1s ceiling on a real git spawn caught that.
    #[test]
    fn join_bounded_returns_once_the_reader_has_finished() {
        let reader = std::thread::spawn(|| {});
        while !reader.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        let limit = Duration::from_secs(10);
        let started = Instant::now();
        join_bounded(&reader, limit);
        let took = started.elapsed();
        assert!(
            took < limit,
            "a finished reader must not be waited on, took {:?}",
            took
        );
    }

    /// A timeout is slow whatever its limit was: it spent the whole limit by
    /// definition. The tests that produce one use limits of a few hundred
    /// milliseconds, well under `SLOW_FETCH_THRESHOLD`, and they must still
    /// be skipped.
    #[test]
    fn timed_out_is_slow_at_any_limit() {
        let timed_out = FetchOutcome::TimedOut {
            after: Duration::from_millis(200),
            stderr: String::new(),
        };

        assert!(timed_out.is_slow(), "a timeout is slow: {:?}", timed_out);
        assert!(
            timed_out.is_slow_with(Duration::from_secs(3600)),
            "a timeout is slow against any threshold, not only a small one"
        );
        assert!(
            !FetchOutcome::Ok.is_slow(),
            "a fetch that worked is never slow"
        );
    }

    /// The threshold has to leave room on both sides. Below the fetch limit,
    /// or a timeout would be the only thing it caught, which is the defect it
    /// exists to remove. Above every fast refusal measured on this machine
    /// (0.45s for a refused https prompt, 0.77s for a host key, 1.6s for a
    /// passphrase key), so those are still fetched again on the worker, where
    /// git's fresh refusal reaches the Creating log.
    #[test]
    fn slow_threshold_sits_inside_the_fetch_limit() {
        assert!(
            SLOW_FETCH_THRESHOLD < UNATTENDED_FETCH_TIMEOUT,
            "a threshold at or above the limit catches only timeouts"
        );
        assert!(
            SLOW_FETCH_THRESHOLD >= Duration::from_secs(2),
            "every fast refusal measured is under 2s and must still be retried"
        );
    }

    /// The rule is "at or above": a failure that took exactly the threshold
    /// is slow, one a nanosecond under it is not. No real fetch lands on the
    /// boundary, but the comparison is the rule and a `>` would pass every
    /// other test here.
    #[test]
    fn a_failure_at_exactly_the_threshold_is_slow() {
        let threshold = Duration::from_millis(500);
        let at = FetchOutcome::Failed {
            exit_code: Some(128),
            stderr: String::new(),
            elapsed: threshold,
        };
        let under = FetchOutcome::Failed {
            exit_code: Some(128),
            stderr: String::new(),
            elapsed: threshold - Duration::from_nanos(1),
        };
        assert!(
            at.is_slow_with(threshold),
            "a failure that took exactly the threshold is slow"
        );
        assert!(
            !under.is_slow_with(threshold),
            "a failure a nanosecond under the threshold is not"
        );
    }

    /// The cancelled outcome through its real path, not a literal written in
    /// a test. `sync_repo_cancellable` returns before any git call when the
    /// flag is already set, and what it returns must read as "nothing ran":
    /// zero elapsed and not slow, so the creation still fetches the repo. A
    /// non-zero value here would make a cancelled sync look like a slow
    /// remote and skip the fetch, building from refs of unknown age, the
    /// misclassification an earlier round of this ticket found once already.
    #[test]
    fn a_cancelled_sync_costs_nothing_and_is_not_slow() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome =
            sync_repo_cancellable(tmp.path(), Duration::from_secs(20), &AtomicBool::new(true));
        match &outcome.fetch {
            FetchOutcome::Failed {
                exit_code,
                stderr,
                elapsed,
            } => {
                assert_eq!(*exit_code, None);
                assert_eq!(stderr, "sync cancelled");
                assert_eq!(
                    *elapsed,
                    Duration::ZERO,
                    "nothing ran, so the outcome must cost nothing"
                );
            }
            other => panic!("expected FetchOutcome::Failed, got {:?}", other),
        }
        assert!(
            !outcome.fetch.is_slow(),
            "a cancelled sync must not be skipped as slow: {:?}",
            outcome.fetch
        );
        assert!(outcome.forwarded.is_empty() && outcome.skipped.is_empty());
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

    /// What a fetch against a remote that should refuse a credential prompt
    /// tells the test, once the outcome is read for what it can prove.
    #[derive(Debug)]
    enum PromptProbe {
        /// git reached the server, got its 401, and refused the prompt with
        /// its own text under exit code 128: the shape the two network tests
        /// pin.
        Refused,
        /// git never got an answer from the server, so its prompt logic never
        /// ran. The test cannot tell the fixed code from the bug here: a
        /// transport failure reads the same whether or not prompts are
        /// disabled, and both shapes exit 128. The reason is git's own line,
        /// or the test's limit, for the skip message.
        Unreached(String),
        /// Anything else, with the text to fail on: the server answered with
        /// an HTTP status line (the probe URL rotted, or an outage; a 401 is
        /// consumed by git's credential path and never reaches that text),
        /// the refusal text arrived under another exit code, git did not
        /// start or was signalled, or the fetch succeeded.
        Unexpected(String),
    }

    /// Classify a prompt probe's fetch. Pass is the refusal text with exit
    /// 128. Skip is `TimedOut`, or `Failed` carrying git's transport prefix
    /// (`unable to access`) without an HTTP status (`The requested URL
    /// returned error`): every resolve, connect, TLS and reset failure lands
    /// there, and none of them ever reached the prompt. Everything else fails.
    fn prompt_probe(fetch: &FetchOutcome) -> PromptProbe {
        match fetch {
            FetchOutcome::TimedOut { after, .. } => {
                PromptProbe::Unreached(format!("timed out after {}s", after.as_secs()))
            }
            FetchOutcome::Failed {
                exit_code: Some(128),
                stderr,
                ..
            } if stderr.contains("terminal prompts disabled") => PromptProbe::Refused,
            FetchOutcome::Failed { stderr, .. }
                if stderr.contains("unable to access")
                    && !stderr.contains("The requested URL returned error") =>
            {
                PromptProbe::Unreached(stderr.trim_end().to_string())
            }
            other => PromptProbe::Unexpected(format!(
                "expected git's refused-prompt text under exit code 128, got {:?}",
                other
            )),
        }
    }

    /// The refusal text with git's exit code is the one pass.
    #[test]
    fn prompt_probe_refusal_text_with_exit_128_is_refused() {
        let fetch = FetchOutcome::Failed {
            exit_code: Some(128),
            stderr: "fatal: could not read Username for 'https://github.com': terminal prompts disabled\n".to_string(),
            elapsed: Duration::from_millis(400),
        };
        assert!(matches!(prompt_probe(&fetch), PromptProbe::Refused));
    }

    /// Every transport failure git reports as `unable to access` is a skip
    /// with git's own line as the reason: the server never answered, so the
    /// prompt logic never ran.
    #[test]
    fn prompt_probe_transport_failures_are_unreached() {
        let lines = [
            "fatal: unable to access 'https://github.com/x/repo.git/': Failed to connect to github.com port 443 after 3 ms: Couldn't connect to server\n",
            "fatal: unable to access 'https://github.com/x/repo.git/': Could not resolve host: github.com\n",
            "fatal: unable to access 'https://github.com/x/repo.git/': LibreSSL SSL_connect: SSL_ERROR_SYSCALL in connection to github.com:443\n",
            "fatal: unable to access 'https://github.com/x/repo.git/': Recv failure: Connection reset by peer\n",
        ];
        for line in lines {
            let fetch = FetchOutcome::Failed {
                exit_code: Some(128),
                stderr: line.to_string(),
                elapsed: Duration::from_millis(5),
            };
            match prompt_probe(&fetch) {
                PromptProbe::Unreached(reason) => assert_eq!(reason, line.trim_end()),
                other => panic!("{:?} must be Unreached, got {:?}", line, other),
            }
        }
    }

    /// The test's own limit expiring is a skip too: the bound is shorter than
    /// the 75 s the macOS kernel gives a connect before giving up
    /// (`net.inet.tcp.keepinit`, which git's `http.connectTimeout` does not
    /// shorten on this build), so a black-hole host lands here.
    #[test]
    fn prompt_probe_timed_out_is_unreached() {
        let fetch = FetchOutcome::TimedOut {
            after: Duration::from_secs(20),
            stderr: String::new(),
        };
        match prompt_probe(&fetch) {
            PromptProbe::Unreached(reason) => assert_eq!(reason, "timed out after 20s"),
            other => panic!("expected Unreached, got {:?}", other),
        }
    }

    /// The sync test's verdict handling, kept out of the test so it can be
    /// pinned without the network: `Refused` passes (returns true),
    /// `Unreached` hands the skip line to `skip` and returns false, and
    /// `Unexpected` panics with its reason.
    fn refused_or_skip(verdict: PromptProbe, skip: &mut dyn FnMut(String)) -> bool {
        match verdict {
            PromptProbe::Refused => true,
            PromptProbe::Unreached(reason) => {
                skip(format!("skipping: github.com was not reached: {}", reason));
                false
            }
            PromptProbe::Unexpected(reason) => panic!("{}", reason),
        }
    }

    /// The create test's verdict handling: `Unexpected` panics first, so its
    /// sharper reason is never masked by a creation failure; then
    /// `assert_created` runs for a refused and an unreached fetch alike,
    /// because a failed fetch of either kind must not fail the creation;
    /// only then is the refusal check skipped, with a line that says the
    /// creation half was still asserted. Returns true when the refusal was
    /// observed.
    fn created_then_refused_or_skip(
        verdict: PromptProbe,
        assert_created: impl FnOnce(),
        skip: &mut dyn FnMut(String),
    ) -> bool {
        if let PromptProbe::Unexpected(reason) = &verdict {
            panic!("{}", reason);
        }
        assert_created();
        match verdict {
            PromptProbe::Refused => true,
            PromptProbe::Unreached(reason) => {
                skip(format!(
                    "skipping the refusal check (the worktree was still created from local refs): github.com was not reached: {}",
                    reason
                ));
                false
            }
            PromptProbe::Unexpected(_) => unreachable!("handled above"),
        }
    }

    #[test]
    fn refused_or_skip_passes_on_refused_and_prints_nothing() {
        let mut lines = Vec::new();
        assert!(refused_or_skip(PromptProbe::Refused, &mut |l| lines.push(l)));
        assert!(lines.is_empty(), "{:?}", lines);
    }

    #[test]
    fn refused_or_skip_prints_the_reason_on_unreached() {
        let mut lines = Vec::new();
        let refused = refused_or_skip(
            PromptProbe::Unreached("timed out after 20s".to_string()),
            &mut |l| lines.push(l),
        );
        assert!(!refused);
        assert_eq!(
            lines,
            ["skipping: github.com was not reached: timed out after 20s"]
        );
    }

    #[test]
    #[should_panic(expected = "got Failed { exit_code: Some(1)")]
    fn refused_or_skip_panics_on_unexpected() {
        refused_or_skip(
            PromptProbe::Unexpected("got Failed { exit_code: Some(1) ..".to_string()),
            &mut |_| {},
        );
    }

    #[test]
    fn created_then_refused_or_skip_asserts_creation_then_passes_on_refused() {
        let ran = std::cell::Cell::new(false);
        let mut lines = Vec::new();
        let refused =
            created_then_refused_or_skip(PromptProbe::Refused, || ran.set(true), &mut |l| {
                lines.push(l)
            });
        assert!(refused);
        assert!(ran.get(), "the creation assertions must run on a pass");
        assert!(lines.is_empty(), "{:?}", lines);
    }

    /// The order is the point: the creation assertions run before the skip
    /// is decided, so a skip never retires them.
    #[test]
    fn created_then_refused_or_skip_asserts_creation_before_skipping() {
        let ran = std::cell::Cell::new(false);
        let mut lines = Vec::new();
        let refused = created_then_refused_or_skip(
            PromptProbe::Unreached(
                "fatal: unable to access 'u': Could not resolve host: h".to_string(),
            ),
            || ran.set(true),
            &mut |l| {
                assert!(
                    ran.get(),
                    "the skip line must come after the creation assertions"
                );
                lines.push(l)
            },
        );
        assert!(!refused);
        assert!(ran.get(), "the creation assertions must run on a skip");
        assert_eq!(
            lines,
            ["skipping the refusal check (the worktree was still created from local refs): github.com was not reached: fatal: unable to access 'u': Could not resolve host: h"]
        );
    }

    /// `Unexpected` panics before the creation assertions, so its reason is
    /// what the failure shows, not whatever the creation half would say.
    #[test]
    fn created_then_refused_or_skip_panics_on_unexpected_before_asserting_creation() {
        let ran = std::cell::Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            created_then_refused_or_skip(
                PromptProbe::Unexpected("the sharper reason".to_string()),
                || ran.set(true),
                &mut |_| {},
            )
        }));
        let payload = result.expect_err("Unexpected must panic");
        let text = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert_eq!(text, "the sharper reason");
        assert!(
            !ran.get(),
            "the creation assertions must not run before the panic"
        );
    }

    /// An HTTP status means the server was reached, so the refusal path was
    /// reachable and did not refuse: that is a failure for a person to read,
    /// whether the probe URL rotted or the code changed. git words a 404 as
    /// `repository '<url>' not found` (no `unable to access`, so the
    /// catch-all takes it) and every other status as `unable to access
    /// '<url>': The requested URL returned error: <status>`.
    #[test]
    fn prompt_probe_http_status_is_unexpected() {
        let lines = [
            ("returned error: 403", "fatal: unable to access 'https://github.com/x/repo.git/': The requested URL returned error: 403\n"),
            ("not found", "fatal: repository 'https://github.com/x/repo.git/' not found\n"),
        ];
        for (needle, line) in lines {
            let fetch = FetchOutcome::Failed {
                exit_code: Some(128),
                stderr: line.to_string(),
                elapsed: Duration::from_millis(300),
            };
            match prompt_probe(&fetch) {
                PromptProbe::Unexpected(reason) => {
                    assert!(reason.contains(needle), "{}", reason)
                }
                other => panic!("{:?} must be Unexpected, got {:?}", line, other),
            }
        }
    }

    /// The refusal text under any other exit code, a spawn failure, a signal
    /// death and a clean fetch all fail: none of them is the pinned shape.
    #[test]
    fn prompt_probe_other_shapes_are_unexpected() {
        let refusal =
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled\n";
        let shapes = [
            FetchOutcome::Failed {
                exit_code: Some(1),
                stderr: refusal.to_string(),
                elapsed: Duration::ZERO,
            },
            FetchOutcome::Failed {
                exit_code: None,
                stderr: format!("{}: No such file or directory", SPAWN_FAILURE_PREFIX),
                elapsed: Duration::ZERO,
            },
            FetchOutcome::Failed {
                exit_code: None,
                stderr: String::new(),
                elapsed: Duration::from_millis(10),
            },
            FetchOutcome::Ok,
        ];
        for fetch in shapes {
            assert!(
                matches!(prompt_probe(&fetch), PromptProbe::Unexpected(_)),
                "{:?} must be Unexpected",
                fetch
            );
        }
    }

    /// A remote that would prompt for credentials fails at once with git's
    /// own "terminal prompts disabled" text instead of hanging. Needs the
    /// network.
    ///
    /// Pass: `FetchOutcome::Failed` with exit code 128 and the refusal text.
    /// Skip, with the reason printed: an askpass helper is configured (it
    /// would answer the prompt); `github.com:443` does not accept a TCP
    /// connect within 3 s; or the fetch itself never got an answer (a
    /// transport failure, or the 20 s limit). The network can change between
    /// the 3 s precheck and the fetch, and a connect failure cannot tell
    /// "prompt refused" from "never got there": git consults
    /// `GIT_TERMINAL_PROMPT` only after the server's 401, so a run that never
    /// got an answer prints the same text and exit code with or without the
    /// fix. Fail: anything else, including any `The requested URL returned
    /// error: <status>` line or git's own `repository '<url>' not found` for
    /// a 404, since a status means github answered and the refusal did not
    /// happen (git consumes its 401 in the credential path, so that status
    /// never reaches this text).
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

        refused_or_skip(prompt_probe(&result.fetch), &mut |line| {
            eprintln!("{}", line)
        });
    }

    /// The pre-create fetch runs under the unattended-run policy, so a remote
    /// that would prompt for credentials fails at once with git's own
    /// "terminal prompts disabled" text instead of prompting on the raw-mode
    /// terminal behind the alternate screen. A failed fetch is not an error:
    /// the worktree is still created from local refs. Needs the network.
    ///
    /// Pass: `Some(FetchOutcome::Failed)` with exit code 128 and the refusal
    /// text, then the worktree exists and its branch starts at the local tip.
    /// Skip, with the reason printed: an askpass helper is configured (it
    /// would answer the prompt); `github.com:443` does not accept a TCP
    /// connect within 3 s; or the fetch itself never got an answer (a
    /// transport failure, or the 20 s limit). The network can change between
    /// the 3 s precheck and the fetch, and a connect failure cannot tell
    /// "prompt refused" from "never got there": git consults
    /// `GIT_TERMINAL_PROMPT` only after the server's 401, so a run that never
    /// got an answer prints the same text and exit code with or without the
    /// fix. Fail: anything else, including any `The requested URL returned
    /// error: <status>` line or git's own `repository '<url>' not found` for
    /// a 404, since a status means github answered and the refusal did not
    /// happen (git consumes its 401 in the credential path, so that status
    /// never reaches this text). The creation half holds for
    /// a refused and an unreached fetch alike, so it is asserted before the
    /// skip: only the refusal check is skipped, and the skip message says so.
    #[test]
    fn create_worktree_refused_https_prompt_still_creates_from_local_refs() {
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
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo)
            .output()
            .unwrap();
        git_setup(&repo);
        git(&["commit", "--allow-empty", "-m", "init"], &repo);
        git(
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/space-wayfinder-probe-nonexistent/repo.git",
            ],
            &repo,
        );
        // An empty value resets the helper list, so a stored credential on the
        // machine cannot answer the 401 before the prompt logic runs.
        git(&["config", "credential.helper", ""], &repo);

        let base_tip = get_sha(&repo, "main");
        let ws_dir = tmp.path().join("workspaces");
        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::NewBranch("feature".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );

        let fetch = attempt
            .fetch
            .as_ref()
            .expect("PreCreateFetch::Run must record a fetch outcome");
        created_then_refused_or_skip(
            prompt_probe(fetch),
            || {
                // Refused or unreached, the fetch failed, and a failed fetch
                // must not fail the creation: the worktree exists and, with
                // no `origin/main` to prefer, the branch starts at the local
                // base tip.
                let path = attempt
                    .created
                    .as_ref()
                    .expect("a failed fetch must not fail the creation");
                assert!(
                    path.exists(),
                    "the worktree must still be created: {}",
                    path.display()
                );
                assert_eq!(
                    get_sha(&repo, "feature"),
                    base_tip,
                    "the new branch must start at the local base tip"
                );
            },
            &mut |line| eprintln!("{}", line),
        );
    }

    /// A remote that never answers must not block the creation for git's own
    /// network timeout: the fetch is stopped at the limit and the worktree is
    /// created from local refs. The remote is a `file://` origin whose
    /// upload-pack is a script that sleeps, so nothing touches the network.
    #[test]
    fn create_worktree_timed_out_fetch_still_creates_from_local_refs() {
        let (tmp, local) = make_behind_repo();
        let script = tmp.path().join("slow-upload-pack.sh");
        std::fs::write(&script, "exec sleep 30\n").unwrap();
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
        let ws_dir = tmp.path().join("workspaces");
        let started = Instant::now();
        let attempt = create_worktree_with_fetch(
            &local,
            &ws_dir,
            "ws",
            &BranchStrategy::NewBranch("feature".to_string()),
            PreCreateFetch::Run(limit),
        );
        let elapsed = started.elapsed();

        match &attempt.fetch {
            Some(FetchOutcome::TimedOut { after, .. }) => assert_eq!(*after, limit),
            other => panic!("expected Some(FetchOutcome::TimedOut), got {:?}", other),
        }
        assert!(
            elapsed < Duration::from_secs(10),
            "creation must return shortly after the limit, took {:?}",
            elapsed
        );
        let path = attempt
            .created
            .expect("a timed-out fetch must not fail the creation");
        assert!(
            path.exists(),
            "the worktree must still be created: {}",
            path.display()
        );
    }

    /// `PreCreateFetch::Skip` runs no fetch at all. The origin is a path that
    /// does not exist, so a fetch would fail instantly and loudly; `Run` on
    /// the same repo is the contrast that proves the skip did something.
    #[test]
    fn create_worktree_skip_runs_no_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let dead_url = format!("file://{}", tmp.path().join("no-such-origin.git").display());
        git(&["remote", "add", "origin", &dead_url], &repo);

        let ws_dir = tmp.path().join("workspaces");
        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "skipped",
            &BranchStrategy::NewBranch("feature".to_string()),
            PreCreateFetch::Skip,
        );
        assert!(
            attempt.fetch.is_none(),
            "Skip must not run a fetch, got {:?}",
            attempt.fetch
        );
        assert!(
            attempt.created.unwrap().exists(),
            "the worktree must be created"
        );

        // Contrast: the same repo with `Run` does fetch, and it fails.
        let ran = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "fetched",
            &BranchStrategy::NewBranch("other".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        assert!(
            matches!(ran.fetch, Some(FetchOutcome::Failed { .. })),
            "Run against a dead origin must report the failure, got {:?}",
            ran.fetch
        );
        assert!(
            ran.created.unwrap().exists(),
            "the worktree must still be created"
        );
    }

    /// A repo whose `origin` is a bare repo holding `main` and `feat`, both
    /// already known locally as `origin/main` and `origin/feat` from one
    /// ungated fetch during setup. `remote.origin.uploadpack` is gated after
    /// that fetch, so a fetch that runs later leaves `marker` behind: git
    /// runs the gate on the far side of a `file://` fetch, and the marker
    /// exists iff a fetch reached the remote. That is the proof this file's
    /// strategy-skip tests rest on, because `attempt.fetch` is what the code
    /// chose to report, not what git did (PR #34's lesson). The repo's own
    /// `main` is a separate history from the origin's, so a local branch
    /// made from it never shares a tip with `origin/feat`.
    fn gated_repo(parent: &Path, name: &str) -> (PathBuf, PathBuf) {
        let repo = plain_repo(parent, name);
        let marker = gate_origin(parent, name, &repo);
        (repo, marker)
    }

    /// The origin half of `gated_repo`, for a repo made some other way.
    fn gate_origin(parent: &Path, name: &str, repo: &Path) -> PathBuf {
        let origin = parent.join(format!("{}-origin.git", name));
        let seed = plain_repo(parent, &format!("{}-seed", name));
        // A second commit, so the origin's history is not byte-identical to
        // the repo's own (two empty `init` commits in the same second are).
        git(&["commit", "--allow-empty", "-m", "origin-only"], &seed);
        git(&["branch", "feat"], &seed);
        git(&["init", "-q", "--bare", origin.to_str().unwrap()], parent);
        git(
            &["push", "-q", origin.to_str().unwrap(), "main", "feat"],
            &seed,
        );

        git(
            &[
                "remote",
                "add",
                "origin",
                &format!("file://{}", origin.display()),
            ],
            repo,
        );
        git(&["fetch", "-q", "origin"], repo);

        let marker = parent.join(format!("FETCHED-{}", name));
        let script = parent.join(format!("gate-{}.sh", name));
        std::fs::write(
            &script,
            format!(
                "touch \"{}\"\nexec git upload-pack \"$@\"\n",
                marker.display()
            ),
        )
        .unwrap();
        git(
            &[
                "config",
                "remote.origin.uploadpack",
                &format!("/bin/sh {}", script.display()),
            ],
            repo,
        );
        marker
    }

    fn head_is_detached(wt: &Path) -> bool {
        git2::Repository::open(wt).unwrap().head_detached().unwrap()
    }

    /// A detached HEAD is added from the source repo's own `HEAD` name, a
    /// local ref, so the pre-create fetch cannot change what it checks out
    /// and does not run. The contrast repo, same fixture and a strategy
    /// that reads `origin/*`, must leave its marker: if the gate were broken
    /// the first half would pass for the wrong reason.
    #[test]
    fn a_detached_head_create_runs_no_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let (detached, detached_marker) = gated_repo(tmp.path(), "detached");
        let (new_branch, new_branch_marker) = gated_repo(tmp.path(), "new-branch");
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &detached,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt
            .created
            .expect("the detached worktree must be created");
        assert!(
            attempt.fetch.is_none(),
            "a detached HEAD reads no remote ref, so no fetch is reported, got {:?}",
            attempt.fetch
        );
        assert!(
            !detached_marker.exists(),
            "a detached HEAD reads no remote ref, so no fetch may reach origin"
        );
        assert!(head_is_detached(&wt), "the strategy is still applied");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&detached, "main"),
            "detached at the source repo's own HEAD"
        );

        let attempt = create_worktree_with_fetch(
            &new_branch,
            &ws_dir,
            "ws",
            &BranchStrategy::NewBranch("topic".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        attempt
            .created
            .expect("the contrast worktree must be created");
        assert_eq!(
            attempt.fetch,
            Some(FetchOutcome::Ok),
            "a new branch reads origin/<base>, so it fetches"
        );
        assert!(
            new_branch_marker.exists(),
            "the contrast proves the gate: a strategy that reads origin/* reaches the remote"
        );
    }

    // Ticket 24: a tag that shares a name with a branch, or with a ref the
    // app builds (`origin/<name>`), must never decide what a worktree is
    // checked out at. Every strategy resolves its refs fully qualified
    // except the plain existing-branch arm, which git decides. The tags in
    // these tests sit at a commit no assertion expects, and each fixture
    // asserts that difference so a same-second collision cannot make a
    // test pass vacuously.

    /// A commit no strategy should land on: a fresh parentless commit that
    /// no branch points at, so it is neither the local `main` tip, nor
    /// `origin/main`, nor `origin/feat` (origin's first commit would be:
    /// two empty `init` commits in the same second are byte-identical).
    fn decoy_sha(repo: &Path) -> String {
        let out = Cmd::new("git")
            .args(["commit-tree", "HEAD^{tree}", "-m", "decoy"])
            .current_dir(repo)
            .output()
            .unwrap();
        let decoy = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            decoy.len(),
            40,
            "fixture: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        for tip in [
            "refs/heads/main",
            "refs/remotes/origin/main",
            "refs/remotes/origin/feat",
        ] {
            assert_ne!(
                decoy,
                get_sha(repo, tip),
                "fixture: decoy must differ from {tip}"
            );
        }
        decoy
    }

    /// The configured upstream of a local branch, read by its qualified
    /// name so a same-named tag cannot make the question ambiguous.
    fn upstream_of(wt: &Path, branch: &str) -> String {
        let out = Cmd::new("git")
            .args([
                "for-each-ref",
                "--format=%(upstream)",
                &format!("refs/heads/{}", branch),
            ])
            .current_dir(wt)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn attempt(repo: &Path, ws_dir: &Path, strategy: BranchStrategy) -> PathBuf {
        create_worktree_with_fetch(repo, ws_dir, "ws", &strategy, PreCreateFetch::Skip)
            .created
            .expect("the worktree must be created")
    }

    /// T1: a New-branch name that is only a tag creates the branch from the
    /// base; the probe reads `refs/heads/<name>`, never the bare name.
    #[test]
    fn new_branch_ignores_a_tag_of_that_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "topic", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("topic".into()),
        );

        assert!(!head_is_detached(&wt), "a tag named topic is not a branch");
        assert_eq!(git::current_branch(&wt).unwrap(), "topic");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/main")
        );
    }

    /// T2: a tag beside a remote-only branch of the same name, plus a tag
    /// named like the remote ref itself: the remote branch is tracked.
    #[test]
    fn new_branch_tracks_the_remote_branch_beside_a_tag_of_that_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "feat", &decoy], &repo);
        git(&["tag", "origin/feat", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("feat".into()),
        );

        assert!(!head_is_detached(&wt));
        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/feat")
        );
        assert_eq!(
            upstream_of(&wt, "feat"),
            "refs/remotes/origin/feat",
            "the local branch tracks origin/feat"
        );
    }

    /// T2b: a tag named `origin/x` with no branch `x` anywhere is not a
    /// remote branch; `x` is created from the base.
    #[test]
    fn new_branch_is_not_fooled_by_a_tag_named_like_a_remote_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "origin/x", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("x".into()),
        );

        assert_eq!(git::current_branch(&wt).unwrap(), "x");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/main")
        );
    }

    /// T3: tags named like the base and like `origin/<base>` do not shadow
    /// the start point; the branch starts at `origin/main` and tracks it,
    /// as it does with no tags (git sets upstream from a remote-tracking
    /// start point).
    #[test]
    fn new_branch_starts_at_the_base_branch_not_a_tag_of_its_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "main", &decoy], &repo);
        git(&["tag", "origin/main", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("fresh".into()),
        );

        assert_eq!(git::current_branch(&wt).unwrap(), "fresh");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/main")
        );
        assert_eq!(
            upstream_of(&wt, "fresh"),
            "refs/remotes/origin/main",
            "the local branch tracks origin/main"
        );
    }

    /// T4: with no `origin/<base>` the start point is the local base branch,
    /// `refs/heads/main`, not the tag of that name.
    #[test]
    fn new_branch_falls_back_to_the_local_base_branch_not_its_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        git(&["commit", "--allow-empty", "-m", "second"], &repo);
        let decoy = get_sha(&repo, "refs/heads/main~1");
        git(&["tag", "main", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("fresh".into()),
        );

        assert_eq!(git::current_branch(&wt).unwrap(), "fresh");
        assert_eq!(get_sha(&wt, "HEAD"), get_sha(&repo, "refs/heads/main"));
        assert_ne!(get_sha(&wt, "HEAD"), decoy);
    }

    /// T5: a detached worktree starts at the base branch's tip, not at the
    /// tag of the same name.
    #[test]
    fn detached_head_starts_at_the_base_branch_not_a_tag_of_its_name() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        git(&["commit", "--allow-empty", "-m", "second"], &repo);
        let decoy = get_sha(&repo, "refs/heads/main~1");
        git(&["tag", "main", &decoy], &repo);

        let wt = attempt(&repo, &tmp.path().join("ws"), BranchStrategy::DetachedHead);

        assert!(head_is_detached(&wt));
        assert_eq!(get_sha(&wt, "HEAD"), get_sha(&repo, "refs/heads/main"));
        assert_ne!(get_sha(&wt, "HEAD"), decoy);
    }

    /// T6: a source repo that is itself detached has no base branch; the
    /// base is `HEAD` and is not qualified (`refs/heads/HEAD` is no ref).
    #[test]
    fn detached_head_from_a_detached_source_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        git(&["commit", "--allow-empty", "-m", "second"], &repo);
        git(&["checkout", "--detach", "refs/heads/main~1"], &repo);
        let source_head = get_sha(&repo, "HEAD");
        assert_ne!(source_head, get_sha(&repo, "refs/heads/main"), "fixture");

        let wt = attempt(&repo, &tmp.path().join("ws"), BranchStrategy::DetachedHead);

        assert!(head_is_detached(&wt));
        assert_eq!(get_sha(&wt, "HEAD"), source_head);
    }

    /// T6b: a source repo whose HEAD is symbolic to something that is not a
    /// local branch (here a remote-tracking ref, as some tooling leaves a
    /// clone) has no base branch either; the base is `HEAD`, not
    /// `refs/heads/origin/main`, which does not exist.
    #[test]
    fn detached_head_from_a_source_whose_head_is_not_a_local_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        git(&["symbolic-ref", "HEAD", "refs/remotes/origin/main"], &repo);
        let source_head = get_sha(&repo, "HEAD");
        assert_eq!(
            source_head,
            get_sha(&repo, "refs/remotes/origin/main"),
            "fixture"
        );
        assert_ne!(source_head, get_sha(&repo, "refs/heads/main"), "fixture");

        let wt = attempt(&repo, &tmp.path().join("ws"), BranchStrategy::DetachedHead);

        assert!(head_is_detached(&wt));
        assert_eq!(get_sha(&wt, "HEAD"), source_head);
    }

    /// T10: a tag whose own name is `refs/heads/x` (git allows it; a remote
    /// can publish one). `rev-parse --verify refs/heads/x` falls back to
    /// `refs/tags/refs/heads/x` when no branch `x` exists (git 2.50.1), so a
    /// qualified name is not enough on its own: the probe has to be an exact
    /// lookup (`show-ref --verify`), or the add of the bare `x` fails with
    /// `invalid reference`.
    #[test]
    fn new_branch_ignores_a_tag_named_like_a_qualified_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "refs/heads/x", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("x".into()),
        );

        assert_eq!(git::current_branch(&wt).unwrap(), "x");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/main")
        );
    }

    /// T6d: the New-branch strategy on a plainly detached source (T6's
    /// shape: no symref at all) does work, unlike T6c: the base is `HEAD`,
    /// `refs/remotes/origin/HEAD` is absent in this fixture, so the branch
    /// starts at the commit HEAD points at, with no upstream. Pins the half
    /// of the distinction the GUIDE's Detached bullet implies.
    #[test]
    fn new_branch_from_a_detached_source_repo_starts_at_its_head() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        git(&["commit", "--allow-empty", "-m", "second"], &repo);
        git(&["checkout", "--detach", "refs/heads/main~1"], &repo);
        let source_head = get_sha(&repo, "HEAD");
        assert_ne!(source_head, get_sha(&repo, "refs/heads/main"), "fixture");
        assert!(
            !ref_exists(&repo, "refs/remotes/origin/HEAD"),
            "fixture: no origin/HEAD, so the start point is HEAD itself"
        );

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("a".into()),
        );

        assert!(!head_is_detached(&wt));
        assert_eq!(git::current_branch(&wt).unwrap(), "a");
        assert_eq!(get_sha(&wt, "HEAD"), source_head);
        assert_eq!(
            upstream_of(&wt, "a"),
            "",
            "a bare HEAD start point sets no upstream"
        );
    }

    /// T6c: the New-branch strategy on the T6b source (HEAD symbolic to a
    /// remote-tracking ref) is refused by git itself, whatever the start
    /// point: `git worktree add -b` dies with `HEAD not found below
    /// refs/heads!` there (git 2.50.1, probed with every bare and qualified
    /// start point, including master's bare `origin/main`). So this case is
    /// neither fixed nor changed by ticket 24; the test pins that the error
    /// reaching the caller is git's own sentence, so a later change that
    /// makes the strategy silently detach or start elsewhere is noticed. It
    /// passes on master too, and says so.
    #[test]
    fn new_branch_from_a_source_whose_head_is_not_a_local_branch_is_refused_by_git() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        git(&["symbolic-ref", "HEAD", "refs/remotes/origin/feat"], &repo);

        let err = create_worktree_with_fetch(
            &repo,
            &tmp.path().join("ws"),
            "ws",
            &BranchStrategy::NewBranch("a".into()),
            PreCreateFetch::Skip,
        )
        .created
        .expect_err("git refuses to create a branch in such a repo");
        assert!(
            err.to_string().contains("HEAD not found below refs/heads!"),
            "git's own refusal, got {:?}",
            err.to_string()
        );
    }

    /// T7: `origin/feat` is read as `refs/remotes/origin/feat`, so a tag
    /// named `origin/feat` neither shadows it nor makes it ambiguous.
    #[test]
    fn existing_origin_branch_ignores_a_tag_named_like_the_remote_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        let decoy = decoy_sha(&repo);
        git(&["tag", "origin/feat", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::ExistingBranch("origin/feat".into()),
        );

        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "refs/remotes/origin/feat")
        );
        assert_eq!(
            upstream_of(&wt, "feat"),
            "refs/remotes/origin/feat",
            "the local branch tracks origin/feat"
        );
    }

    /// T8: a local branch beside a tag of the same name is checked out as a
    /// branch. The commit-ish slot of this form must stay the bare name:
    /// `git worktree add <wt> refs/heads/feat` succeeds but detaches
    /// (git 2.50.1), because git keys the branch form on the bare name.
    #[test]
    fn new_branch_checks_out_the_local_branch_beside_a_tag_of_its_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "refs/heads/main"], &repo);
        let decoy = get_sha(&repo, "refs/remotes/origin/main");
        assert_ne!(decoy, get_sha(&repo, "refs/heads/feat"), "fixture");
        git(&["tag", "feat", &decoy], &repo);

        let wt = attempt(
            &repo,
            &tmp.path().join("ws"),
            BranchStrategy::NewBranch("feat".into()),
        );

        assert!(
            !head_is_detached(&wt),
            "the local branch, not the tag, and as a branch"
        );
        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(get_sha(&wt, "HEAD"), get_sha(&repo, "refs/heads/feat"));
    }

    /// An `origin/`-prefixed name is added with `--track` from that very
    /// ref, so the fetch stays.
    #[test]
    fn an_origin_existing_branch_still_fetches() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("origin/feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert_eq!(attempt.fetch, Some(FetchOutcome::Ok));
        assert!(
            marker.exists(),
            "origin/feat is the ref the add reads, so the fetch must reach origin"
        );
        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(
            get_sha(&wt, "feat@{upstream}"),
            get_sha(&repo, "origin/feat"),
            "the local branch tracks origin/feat"
        );
    }

    /// A plain name that is a local branch is checked out at that branch's
    /// local tip and `origin/<name>` is never read, so the fetch is skipped.
    /// The local `feat` is deliberately a different history from
    /// `origin/feat`, so the tip assertion can tell the two apart.
    #[test]
    fn a_local_existing_branch_runs_no_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "main"], &repo);
        assert_ne!(
            get_sha(&repo, "feat"),
            get_sha(&repo, "origin/feat"),
            "fixture: local feat and origin/feat must differ"
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert!(
            attempt.fetch.is_none(),
            "a local branch reads no remote ref, got {:?}",
            attempt.fetch
        );
        assert!(
            !marker.exists(),
            "a local branch reads no remote ref, so no fetch may reach origin"
        );
        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "feat"),
            "checked out at the local tip, not origin's"
        );
    }

    /// The case the ticket's premise missed. A plain name with no local
    /// branch is not "no remote ref": git's `worktree add` resolves it to
    /// `origin/<name>` when exactly one remote has it, and adds with
    /// `--track -b` (git 2.50.1, reproduced during grilling). The fetch can
    /// change what gets checked out, so it stays. This also pins the DWIM
    /// itself: a git that stops doing it shows up here, not in a space.
    #[test]
    fn an_existing_branch_found_only_on_origin_still_fetches() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        assert!(
            git2::Repository::open(&repo)
                .unwrap()
                .find_branch("feat", git2::BranchType::Local)
                .is_err(),
            "fixture: feat must not exist locally"
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert_eq!(attempt.fetch, Some(FetchOutcome::Ok));
        assert!(
            marker.exists(),
            "with no local feat the add reads origin/feat, so the fetch must reach origin"
        );
        assert_eq!(git::current_branch(&wt).unwrap(), "feat");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            get_sha(&repo, "origin/feat"),
            "git resolved the plain name to origin/feat"
        );
        assert_eq!(
            get_sha(&wt, "feat@{upstream}"),
            get_sha(&repo, "origin/feat"),
            "and set it up to track origin/feat"
        );
    }

    /// The skip rests on `git fetch origin` leaving `refs/heads/*` alone,
    /// which is a property of the default refspec, not of fetch. A repo
    /// whose fetch refspec writes into `refs/heads/*` has its local
    /// branches moved by the fetch, so a local branch is not the fixed
    /// point the rule assumes and the fetch stays. Proof:
    /// the local `feat` starts on the repo's own history and the worktree
    /// comes out at the origin's tip, which only a fetch could have put
    /// there. The refspec names `feat` alone: a `refs/heads/*` wildcard
    /// would also cover the checked-out `main`, and git then refuses the
    /// whole fetch before moving anything (git 2.50.1, `refusing to fetch
    /// into branch`), which would prove nothing here.
    #[test]
    fn a_fetch_that_writes_local_branches_is_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "main"], &repo);
        let origin_tip = get_sha(&repo, "origin/feat");
        assert_ne!(get_sha(&repo, "feat"), origin_tip, "fixture");
        git(
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/feat:refs/heads/feat",
            ],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert!(
            attempt.fetch.is_some(),
            "a fetch that can move local branches must run"
        );
        assert!(marker.exists(), "and must reach the remote");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            origin_tip,
            "the fetch moved local feat to the origin's tip before the add"
        );
    }

    /// A fetch refspec with no destination (`refs/heads/feat` on its own)
    /// fetches into `FETCH_HEAD` only and writes no local branch, so the
    /// skip stands. It is here because git accepts that form and git2's
    /// refspec accessor does not: `Refspec::dst` unwraps a null destination
    /// (git2 0.19, `refspec.rs:37`), so a rule that read refspecs through
    /// it took the Creating worker down on such a repo.
    #[test]
    fn a_fetch_refspec_without_a_destination_still_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(
            &["config", "--add", "remote.origin.fetch", "refs/heads/feat"],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        attempt.created.expect("the worktree must be created");
        assert!(attempt.fetch.is_none(), "got {:?}", attempt.fetch);
        assert!(
            !marker.exists(),
            "a destination-less refspec writes no local branch, so nothing to fetch for"
        );
    }

    /// A fetch destination that is not fully qualified (`+refs/heads/feat:feat`)
    /// is a local branch to git: the fetch creates or moves `refs/heads/feat`
    /// (git 2.50.1, reproduced). So it counts as writing local branches and
    /// the fetch stays, even though the destination does not spell
    /// `refs/heads/`.
    #[test]
    fn an_unqualified_fetch_destination_is_a_local_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "main"], &repo);
        let origin_tip = get_sha(&repo, "origin/feat");
        assert_ne!(get_sha(&repo, "feat"), origin_tip, "fixture");
        git(
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/feat:feat",
            ],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert!(attempt.fetch.is_some(), "the fetch must run");
        assert!(marker.exists(), "and reach the remote");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            origin_tip,
            "the fetch moved local feat before the add"
        );
    }

    /// The realistic mirror setup in a non-bare repo: a wildcard into
    /// `refs/heads/*` plus a negative refspec that exempts the checked-out
    /// branch, so git no longer refuses the fetch and every other local
    /// branch moves. The negative entry has no destination and must neither
    /// hide the wildcard beside it nor break the rule.
    #[test]
    fn a_negative_refspec_beside_a_wildcard_keeps_the_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "main"], &repo);
        let origin_tip = get_sha(&repo, "origin/feat");
        assert_ne!(get_sha(&repo, "feat"), origin_tip, "fixture");
        git(
            &[
                "config",
                "--add",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
            &repo,
        );
        git(
            &["config", "--add", "remote.origin.fetch", "^refs/heads/main"],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert_eq!(
            attempt.fetch,
            Some(FetchOutcome::Ok),
            "the fetch must run and succeed"
        );
        assert!(marker.exists(), "and reach the remote");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            origin_tip,
            "the fetch moved local feat before the add"
        );
    }

    /// A repo with no `origin` at all has nothing for a fetch to move, so a
    /// detached HEAD is added without one. Before the skip this fetched and
    /// failed; the line that reported that failure was the only trace.
    #[test]
    fn a_repo_without_origin_runs_no_fetch_for_a_detached_head() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert!(attempt.fetch.is_none(), "got {:?}", attempt.fetch);
        assert!(head_is_detached(&wt));
    }

    /// The one path where a skipped fetch and a refused add coincide: an
    /// `ExistingBranch` of a present local branch that is checked out in
    /// another worktree. The fetch is skipped (local branch, default
    /// refspec) and the add is refused by git, and the attempt reports both
    /// as they are, `fetch: None` beside the refusal, so a caller reading
    /// them together is not led to blame stale refs for a checkout clash.
    #[test]
    fn a_refused_add_of_a_local_branch_reports_no_fetch_beside_the_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "feat", "main"], &repo);
        let ws_dir = tmp.path().join("workspaces");

        let first = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "first",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        first.created.expect("the first worktree must be created");

        let second = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "second",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let err = second
            .created
            .expect_err("feat is checked out in the first worktree");
        assert!(
            refuses_because_checked_out(&err.to_string()),
            "expected git's checked-out refusal, got: {}",
            err
        );
        assert!(second.fetch.is_none(), "got {:?}", second.fetch);
        assert!(!marker.exists(), "neither attempt reads a remote ref");
    }

    /// The mirror destination `refs/*` (what `git clone --mirror` and
    /// `git remote add --mirror=fetch` write) covers `refs/heads/*` without
    /// spelling it, so a fetch through it moves local branches too. Here the
    /// checked-out branch is one origin does not have, so git does not refuse
    /// the fetch (git 2.50.1, reproduced: local `feat` moved, exit 0).
    #[test]
    fn a_wildcard_refs_destination_keeps_the_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["checkout", "-q", "-b", "local-only"], &repo);
        git(&["branch", "feat", "main"], &repo);
        let origin_tip = get_sha(&repo, "origin/feat");
        assert_ne!(get_sha(&repo, "feat"), origin_tip, "fixture");
        git(
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/*:refs/*",
            ],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert!(attempt.fetch.is_some(), "the fetch must run");
        assert!(marker.exists(), "and reach the remote");
        assert_eq!(
            get_sha(&wt, "HEAD"),
            origin_tip,
            "the fetch moved local feat before the add"
        );
    }

    /// Every arm of the refspec classifier, pinned directly, so the arms
    /// the marker fixtures do not reach (an empty destination, `src:`,
    /// which git reads as "do not store"; the negative forms) have a
    /// killing assertion too. False rows are the skip side and are the
    /// ones that matter; true rows only cost a fetch.
    #[test]
    fn refspec_classifier_table() {
        let cases = [
            ("+refs/heads/*:refs/remotes/origin/*", false),
            ("refs/heads/feat", false),
            ("refs/heads/feat:", false),
            ("^refs/heads/main", false),
            ("+refs/heads/*:refs/remotes/*", false),
            ("+refs/tags/*:refs/tags/*", false),
            ("+refs/heads/*:refs/heads/*", true),
            ("+refs/heads/feat:refs/heads/feat", true),
            ("+refs/heads/feat:feat", true),
            ("+refs/*:refs/*", true),
            ("+refs/heads/*:refs/h*", true),
            ("+refs/heads/*:*", true),
            ("  +refs/heads/*:refs/heads/*  ", true),
        ];
        for (spec, expected) in cases {
            assert_eq!(
                refspec_writes_under(spec, "refs/heads/"),
                expected,
                "refspec {:?}",
                spec
            );
        }
        // The same classifier asked about another remote's namespace
        // (ticket 25): an unqualified destination is a local branch, never
        // a remote-tracking ref, and a wildcard counts only when its
        // literal part can reach the prefix.
        let upstream_cases = [
            ("+refs/heads/*:refs/remotes/upstream/*", true),
            ("+refs/heads/main:refs/remotes/upstream/feat", true),
            ("+refs/heads/*:refs/remotes/*", true),
            ("+refs/heads/*:refs/remotes/up*", true),
            ("+refs/*:refs/*", true),
            ("+refs/heads/*:refs/remotes/upstream/rel-*", true),
            ("+refs/heads/*:refs/remotes/origin/*", false),
            ("+refs/heads/*:refs/heads/*", false),
            ("+refs/heads/feat:feat", false),
            ("+refs/heads/*:*", false),
            ("+refs/heads/*:refs/remotes/upstreamer/*", false),
            ("refs/heads/feat", false),
        ];
        for (spec, expected) in upstream_cases {
            assert_eq!(
                refspec_writes_under(spec, "refs/remotes/upstream/"),
                expected,
                "refspec {:?} under refs/remotes/upstream/",
                spec
            );
        }
    }

    /// The config read's fail-safe, on the one path a repo can reach: a
    /// `remote.origin.fetch` value git2 cannot decode as UTF-8. The entry
    /// here writes under `refs/remotes/`, so git would not move a local
    /// branch through it; the fetch runs because the value cannot be read,
    /// which is the invariant, and a detached HEAD is the strategy that
    /// would otherwise skip.
    #[test]
    fn a_fetch_refspec_git2_cannot_decode_takes_the_fetching_side() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        let config = repo.join(".git").join("config");
        let mut bytes = std::fs::read(&config).unwrap();
        bytes.extend_from_slice(
            b"[remote \"origin\"]\n\tfetch = +refs/heads/*:refs/remotes/\xff/*\n",
        );
        std::fs::write(&config, bytes).unwrap();
        let undecodable = {
            let config = git2::Repository::open(&repo).unwrap().config().unwrap();
            let mut entries = config.multivar("remote.origin.fetch", None).unwrap();
            let mut found = false;
            while let Some(Ok(entry)) = entries.next() {
                found |= entry.value().is_none();
            }
            found
        };
        assert!(
            undecodable,
            "fixture: one fetch refspec must be undecodable to git2"
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        attempt.created.expect("the worktree must be created");
        assert!(
            attempt.fetch.is_some(),
            "an unreadable refspec must not skip"
        );
        assert!(marker.exists(), "and the fetch must reach the remote");
    }

    /// The caller's fail-safe: a repo git2 cannot open is treated as reading
    /// origin. A reftable repo is one git 2.50.1 works in and git2 0.19
    /// refuses (`unsupported extension name extensions.refstorage`), so the
    /// marker can prove the fetch reached the remote. The base branch comes
    /// from git2 too and falls back to `main`, which is the branch here.
    #[test]
    fn a_repo_git2_cannot_open_takes_the_fetching_side() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(
            &["init", "-q", "-b", "main", "--ref-format=reftable"],
            &repo,
        );
        git_setup(&repo);
        git(&["commit", "--allow-empty", "-m", "init"], &repo);
        let marker = gate_origin(tmp.path(), "repo", &repo);
        assert!(
            git2::Repository::open(&repo).is_err(),
            "fixture: git2 must be unable to open a reftable repo"
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert_eq!(attempt.fetch, Some(FetchOutcome::Ok));
        assert!(marker.exists(), "the fetch must reach the remote");
        assert_eq!(get_sha(&wt, "HEAD"), get_sha(&repo, "main"));
    }

    /// The same fail-safe on the everyday way to break an open: a syntax
    /// error in `.git/config`. Shell git dies reading the same file before
    /// it reaches the remote (`fatal: bad config line`), so no marker can
    /// appear here; what this pins is that the fetch was attempted, which
    /// only a spawned `git fetch` can report.
    #[test]
    fn a_config_syntax_error_still_attempts_the_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, _marker) = gated_repo(tmp.path(), "repo");
        let config = repo.join(".git").join("config");
        let mut bytes = std::fs::read(&config).unwrap();
        bytes.extend_from_slice(b"[broken\n");
        std::fs::write(&config, bytes).unwrap();
        assert!(git2::Repository::open(&repo).is_err(), "fixture");
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        match attempt.fetch {
            Some(FetchOutcome::Failed { ref stderr, .. }) => assert!(
                stderr.contains("bad config"),
                "git fetch ran and refused the config, got: {}",
                stderr
            ),
            ref other => panic!("the fetch must be attempted, got {:?}", other),
        }
    }

    /// `origin/x` fetches because of its prefix, not because no local
    /// branch has that name. A local branch literally named `origin/feat`
    /// is legal, and with it present the local-branch rule alone would skip,
    /// so this is the test where the unconditional-fetch arm decides.
    #[test]
    fn an_origin_name_fetches_even_beside_a_local_branch_of_that_name() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["branch", "origin/feat", "main"], &repo);
        assert!(
            git2::Repository::open(&repo)
                .unwrap()
                .find_branch("origin/feat", git2::BranchType::Local)
                .is_ok(),
            "fixture: a local branch named origin/feat"
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::ExistingBranch("origin/feat".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        assert_eq!(attempt.fetch, Some(FetchOutcome::Ok));
        assert!(marker.exists(), "the fetch must reach the remote");
    }

    /// A detached HEAD skips only after the refspec check: a fetch refspec
    /// that writes `refs/heads/*` can move the very branch the detached
    /// worktree is added at, so the fetch stays. The checked-out branch is
    /// one origin does not have, so git does not refuse the fetch.
    #[test]
    fn a_detached_head_fetches_when_the_refspec_writes_local_branches() {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, marker) = gated_repo(tmp.path(), "repo");
        git(&["checkout", "-q", "-b", "local-only"], &repo);
        git(
            &[
                "config",
                "--replace-all",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
            &repo,
        );
        let ws_dir = tmp.path().join("workspaces");

        let attempt = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "ws",
            &BranchStrategy::DetachedHead,
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let wt = attempt.created.expect("the worktree must be created");
        assert_eq!(attempt.fetch, Some(FetchOutcome::Ok));
        assert!(marker.exists(), "the fetch must reach the remote");
        assert!(head_is_detached(&wt));
    }

    /// The fetch outcome survives an add that then refused. Stale refs are a
    /// likely reason for such a refusal, so the caller must still get the
    /// fetch line that explains it. Offline: the origin is a path that does
    /// not exist, and the add is refused because the branch is already
    /// checked out in the first worktree.
    #[test]
    fn create_worktree_reports_the_fetch_even_when_the_add_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        Cmd::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo)
            .output()
            .unwrap();
        git_setup(&repo);
        git(&["commit", "--allow-empty", "-m", "init"], &repo);
        let dead_url = format!("file://{}", tmp.path().join("no-such-origin.git").display());
        git(&["remote", "add", "origin", &dead_url], &repo);

        let ws_dir = tmp.path().join("workspaces");
        let first = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "first",
            &BranchStrategy::NewBranch("feature".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        assert!(first.created.is_ok(), "the first worktree must be created");

        // `NewBranch` of a name that already exists locally adds that branch
        // and is refused the same way, and it still fetches (it probes
        // `origin/*`); an `ExistingBranch` of a present local branch would
        // skip the fetch this test is about.
        let second = create_worktree_with_fetch(
            &repo,
            &ws_dir,
            "second",
            &BranchStrategy::NewBranch("feature".to_string()),
            PreCreateFetch::Run(Duration::from_secs(20)),
        );
        let err = second
            .created
            .expect_err("a branch checked out elsewhere must refuse the add");
        assert!(
            err.to_string().contains("already"),
            "expected git's refusal, got: {}",
            err
        );
        assert!(
            matches!(second.fetch, Some(FetchOutcome::Failed { .. })),
            "the fetch outcome must survive a refused add, got {:?}",
            second.fetch
        );
    }

    /// The admin directory a worktree's `.git` file names, read the way the
    /// fixtures need it: git's `gitdir: ` line, resolved against the worktree
    /// when relative.
    fn admin_dir_of(wt: &Path) -> PathBuf {
        let content = std::fs::read_to_string(wt.join(".git")).unwrap();
        let target = content
            .strip_prefix("gitdir: ")
            .unwrap()
            .trim_end_matches(['\n', '\r']);
        let target = Path::new(target);
        if target.is_absolute() {
            target.to_path_buf()
        } else {
            wt.join(target)
        }
    }

    /// A worktree of `repo` at `spaces/<ws>/repo`, made by the production
    /// path on a branch of its own.
    fn worktree_in(repo: &Path, spaces: &Path, ws: &str) -> PathBuf {
        create_worktree_with_fetch(
            repo,
            spaces,
            ws,
            &BranchStrategy::NewBranch(format!("topic-{}", ws)),
            PreCreateFetch::Skip,
        )
        .created
        .expect("the fixture's worktree must be created")
    }

    /// The predicate the Creating worker skips on. A worktree made by the
    /// production path counts whatever branch it carries, which is the case
    /// the retry after a checked-out bounce turns on: the repos already in
    /// the space were created under the strategy the user has just replaced.
    #[test]
    fn placement_of_adopts_a_worktree_of_the_source_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let wt = worktree_in(&repo, &tmp.path().join("spaces"), "ws-a");

        assert_eq!(
            git::current_branch(&wt).unwrap(),
            "topic-ws-a",
            "the fixture is on a branch the source repo is not on, so a \
             branch-blind predicate is what is being asserted"
        );
        assert_eq!(
            placement_of(&wt, &repo),
            Placement::Adopted,
            "a worktree of the source repo is already created in this space"
        );
    }

    /// The three shapes that must NOT be skipped. Each one still reaches
    /// `git worktree add` and fails with `already exists`, which is the
    /// truthful row for a path the space does not own.
    ///
    /// The clone is the case that decides the definition: `<path>/.git
    /// exists`, which `workspace_detail` and `remove_workspace` use for "a
    /// repo in a space", accepts it.
    #[test]
    fn placement_of_attempts_a_plain_directory_a_clone_and_another_repos_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let other = plain_repo(tmp.path(), "other");
        let spaces = tmp.path().join("spaces");

        let plain = spaces.join("ws-a").join("repo");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("README.md"), "not a repo\n").unwrap();
        assert_eq!(
            placement_of(&plain, &repo),
            Placement::Attempt,
            "a plain directory is not a worktree of anything"
        );

        let clone = spaces.join("ws-b").join("repo");
        std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
        git(
            &["clone", repo.to_str().unwrap(), clone.to_str().unwrap()],
            tmp.path(),
        );
        assert!(
            clone.join(".git").is_dir(),
            "the fixture must be a real clone, which `.git exists` would accept"
        );
        assert_eq!(
            placement_of(&clone, &repo),
            Placement::Attempt,
            "a clone of the source repo is not a worktree of it"
        );

        let foreign = spaces.join("ws-c").join("repo");
        std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        git(
            &["worktree", "add", "-b", "topic", foreign.to_str().unwrap()],
            &other,
        );
        assert_eq!(
            placement_of(&foreign, &other),
            Placement::Adopted,
            "the fixture must be a worktree of the OTHER repo"
        );
        assert_eq!(
            placement_of(&foreign, &repo),
            Placement::Attempt,
            "a worktree of a different repo must not count as this repo's"
        );
    }

    /// The one shape only the commondir check keeps out. A submodule's
    /// gitdir is `<source>/.git/modules/<name>/`, a repository of its own
    /// with no `commondir`, so the layout reader calls it a repository
    /// rather than a worktree. Without that, a submodule checkout sitting
    /// where the space wants one would be reported as already created and
    /// silently skipped.
    ///
    /// `protocol.file.allow=always` is needed from git 2.38.1: the file
    /// transport is refused for submodules by default (CVE-2022-39253).
    #[test]
    fn placement_of_attempts_a_submodule_whose_gitdir_lives_under_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let source = plain_repo(tmp.path(), "source");
        let other = plain_repo(tmp.path(), "other");

        git(
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                other.to_str().unwrap(),
                "sub",
            ],
            &source,
        );

        let sub = source.join("sub");
        assert!(
            sub.join(".git").is_file(),
            "the fixture must be a gitlink FILE, which is what points at the \
             gitdir under the source"
        );
        assert!(
            source.join(".git").join("modules").join("sub").is_dir(),
            "the fixture's gitdir must live under the source repo's .git"
        );
        assert_eq!(
            placement_of(&sub, &source),
            Placement::Attempt,
            "a submodule is not a worktree of its superproject: its place in \
             a space must still be attempted, not skipped"
        );
    }

    /// A `kill -9` of `git worktree add` alone leaves its checkout child
    /// running, and the child finishes: the tree is whole, the index
    /// written, and only the `initializing` lock stays (probed on git 2.50.1,
    /// 24,000 of 24,000 files). That tree is adopted as the complete tree it
    /// is; the lock meets ticket 27's unlock hint at removal. The content of
    /// `locked` is not what decides, which is also what keeps a user's lock
    /// reasoned `initializing` out of the second force.
    #[test]
    fn placement_of_adopts_a_finished_tree_under_a_stale_initializing_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let wt = worktree_in(&repo, &tmp.path().join("spaces"), "ws-a");
        let admin = admin_dir_of(&wt);
        assert!(
            admin.join("index").is_file(),
            "fixture: a finished add has an index"
        );
        std::fs::write(admin.join("locked"), "initializing\n").unwrap();

        assert_eq!(
            placement_of(&wt, &repo),
            Placement::Adopted,
            "a complete tree is adopted whatever its lock says"
        );
    }

    /// The state a killed checkout leaves: the lock git took for the add,
    /// the lock the checkout took on the index, and no index, because
    /// `reset --hard` writes every file first and commits the index last.
    /// Probed three times on git 2.50.1 with parent and child killed
    /// together: `locked`, `index.lock`, no `index`, every time. Built by
    /// hand from a finished add, since a kill in the suite would race the
    /// child. The lock's content is git's marker here and is not read.
    #[test]
    fn placement_of_refuses_a_locked_worktree_whose_checkout_never_wrote_its_index() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let wt = worktree_in(&repo, &tmp.path().join("spaces"), "ws-a");
        let admin = admin_dir_of(&wt);
        std::fs::write(admin.join("locked"), "initializing\n").unwrap();
        std::fs::remove_file(admin.join("index")).unwrap();
        std::fs::write(admin.join("index.lock"), "").unwrap();

        match placement_of(&wt, &repo) {
            Placement::HalfBuilt(why) => {
                assert!(
                    why.contains("never finished") && why.contains("index.lock"),
                    "the reason says what it is, got {:?}",
                    why
                );
                assert!(
                    why.contains(&format!("{:?}", wt)) && why.contains("git worktree remove -f -f"),
                    "the reason names the path and the way out, got {:?}",
                    why
                );
            }
            other => panic!(
                "a checkout that never wrote its index must be refused, got {:?}",
                other
            ),
        }
    }

    /// A lock the user placed is not a half-built worktree, whatever its
    /// reason: `git worktree lock` writes an empty file with no reason and
    /// the reason otherwise, and the tree under it is whole (it has an
    /// index). A `--no-checkout` worktree the user then locked has no index
    /// but no `index.lock` either, since no checkout ever ran: it is adopted
    /// too, because refusing it would let the remove side delete files the
    /// user put there by hand. Each of the three signs is required on its
    /// own: a stale `index.lock` beside a surviving `index` under a user's
    /// lock is a killed index writer, not an add; a stale `index.lock` with
    /// no index in an unlocked worktree is not an add either.
    #[test]
    fn placement_of_adopts_a_worktree_the_user_locked() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let spaces = tmp.path().join("spaces");

        let no_reason = worktree_in(&repo, &spaces, "ws-a");
        git(&["worktree", "lock", no_reason.to_str().unwrap()], &repo);
        assert_eq!(
            std::fs::read_to_string(admin_dir_of(&no_reason).join("locked")).unwrap(),
            "",
            "fixture: a lock with no reason is an empty file"
        );
        assert_eq!(placement_of(&no_reason, &repo), Placement::Adopted);

        let reason = worktree_in(&repo, &spaces, "ws-b");
        git(
            &[
                "worktree",
                "lock",
                "--reason",
                "on a usb stick",
                reason.to_str().unwrap(),
            ],
            &repo,
        );
        assert_eq!(placement_of(&reason, &repo), Placement::Adopted);

        // git's own marker word, typed by the user, on a finished tree.
        let marker_word = worktree_in(&repo, &spaces, "ws-d");
        git(
            &[
                "worktree",
                "lock",
                "--reason",
                "initializing",
                marker_word.to_str().unwrap(),
            ],
            &repo,
        );
        assert_eq!(
            placement_of(&marker_word, &repo),
            Placement::Adopted,
            "a finished tree the user locked with git's own word is theirs"
        );

        // A user's lock on a finished tree whose index writer died: `index`
        // survives because git writes it lock-then-rename, and the stale
        // `index.lock` beside it must not read as a checkout in progress.
        let stale_lock = worktree_in(&repo, &spaces, "ws-f");
        git(
            &[
                "worktree",
                "lock",
                "--reason",
                "initializing",
                stale_lock.to_str().unwrap(),
            ],
            &repo,
        );
        let stale_admin = admin_dir_of(&stale_lock);
        std::fs::write(stale_admin.join("index.lock"), "").unwrap();
        assert!(
            stale_admin.join("index").is_file(),
            "fixture: the index survived"
        );
        assert_eq!(
            placement_of(&stale_lock, &repo),
            Placement::Adopted,
            "a finished tree with a stale index.lock under a user lock is theirs"
        );

        // No lock at all: the add finished (git unlinks the lock last), so
        // a missing index with a stale index.lock is a worktree the user
        // made without a checkout and whose index writer died. Theirs.
        let unlocked = worktree_in(&repo, &spaces, "ws-e");
        let unlocked_admin = admin_dir_of(&unlocked);
        std::fs::remove_file(unlocked_admin.join("index")).unwrap();
        std::fs::write(unlocked_admin.join("index.lock"), "").unwrap();
        assert_eq!(
            placement_of(&unlocked, &repo),
            Placement::Adopted,
            "without the add's lock there is no add in progress to refuse"
        );

        let no_checkout = spaces.join("ws-c").join("repo");
        std::fs::create_dir_all(no_checkout.parent().unwrap()).unwrap();
        git(
            &[
                "worktree",
                "add",
                "--no-checkout",
                "--lock",
                "-b",
                "topic-ws-c",
                no_checkout.to_str().unwrap(),
            ],
            &repo,
        );
        let admin = admin_dir_of(&no_checkout);
        assert!(
            !admin.join("index").exists() && !admin.join("index.lock").exists(),
            "fixture: no checkout ran, so neither the index nor its lock exists"
        );
        std::fs::write(no_checkout.join("by-hand.txt"), "mine\n").unwrap();
        assert_eq!(
            placement_of(&no_checkout, &repo),
            Placement::Adopted,
            "a --no-checkout worktree the user locked is theirs, not half-built"
        );
    }

    /// `worktree.useRelativePaths` (git 2.48 and later) writes a relative
    /// gitdir into the worktree's `.git` file and `extensions.relativeWorktrees`
    /// into the source repo, which the libgit2 this crate links cannot open.
    /// The old predicate answered "not a worktree" and the retry failed on
    /// `already exists`. The config is set for real, not the file hand-written
    /// (the ticket 27 lesson).
    #[test]
    fn placement_of_adopts_a_relative_paths_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        git(&["config", "worktree.useRelativePaths", "true"], &repo);
        let wt = worktree_in(&repo, &tmp.path().join("spaces"), "ws-a");

        let gitfile = std::fs::read_to_string(wt.join(".git")).unwrap();
        assert!(
            gitfile.starts_with("gitdir: ..") || gitfile.starts_with("gitdir: ./"),
            "fixture: git must have written a relative gitdir, got {:?}",
            gitfile
        );
        assert!(
            git2::Repository::open(&wt).is_err(),
            "fixture: git2 must be unable to open it, or the test proves nothing"
        );
        assert_eq!(
            placement_of(&wt, &repo),
            Placement::Adopted,
            "a worktree with a relative gitdir is a worktree of its repo"
        );
    }

    /// A reftable source repo (`git init --ref-format=reftable`, git 2.45 and
    /// later, `extensions.refstorage`) is the other format libgit2 refuses.
    /// Where the local git is older, an unknown extension stands in, as in
    /// the ticket 27 tests: git2 refuses either.
    #[test]
    fn placement_of_adopts_a_worktree_of_a_reftable_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reftable = Cmd::new("git")
            .args(["init", "-q", "-b", "main", "--ref-format=reftable"])
            .current_dir(&repo)
            .output()
            .unwrap();
        if !reftable.status.success() {
            Cmd::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&repo)
                .output()
                .unwrap();
            git(&["config", "core.repositoryformatversion", "1"], &repo);
            git(&["config", "extensions.spaceUnknown", "true"], &repo);
        }
        git_setup(&repo);
        git(&["commit", "--allow-empty", "-m", "init"], &repo);
        assert!(
            git2::Repository::open(&repo).is_err(),
            "fixture: git2 must refuse the source repo, or the test proves nothing"
        );
        let wt = worktree_in(&repo, &tmp.path().join("spaces"), "ws-a");

        assert_eq!(
            placement_of(&wt, &repo),
            Placement::Adopted,
            "a worktree of a repo in a format libgit2 refuses is still its worktree"
        );
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

    /// Whether `dir` (a clone or a linked worktree) has a merge in progress:
    /// the `MERGE_HEAD` file in its git dir, the way git and `pull_repo` test
    /// it, so a linked worktree's private git dir is honoured and a ref named
    /// `MERGE_HEAD` does not count.
    fn merge_head_present(dir: &Path) -> bool {
        let out = Cmd::new("git")
            .args(["rev-parse", "--git-path", "MERGE_HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git rev-parse --git-path failed");
        dir.join(String::from_utf8_lossy(&out.stdout).trim())
            .exists()
    }

    /// Diverge `local` from origin so that a merge conflicts on `base.txt` and
    /// updates `other.txt` cleanly (origin alone changes it). The clean update
    /// is what a failed abort trips over in the dirty-file shape.
    fn conflicting_diverge(tmp: &Path, local: &Path) {
        std::fs::write(local.join("other.txt"), "other\n").unwrap();
        git(&["add", "."], local);
        git(&["commit", "-m", "add-other"], local);
        git(&["push", "origin", "main"], local);
        advance_origin(tmp, |helper| {
            std::fs::write(helper.join("base.txt"), "helper-version\n").unwrap();
            std::fs::write(helper.join("other.txt"), "helper-other\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "helper-edit"], helper);
        });
        std::fs::write(local.join("base.txt"), "local-version\n").unwrap();
        git(&["add", "."], local);
        git(&["commit", "-m", "local-edit"], local);
    }

    /// Install a `post-index-change` hook in `local` that runs `action` (a
    /// `/bin/sh` snippet; cwd is the worktree, `$D` the absolute git dir)
    /// exactly once: at the index write that checks out a conflicted merge,
    /// which is the last thing `git merge` does before `pull_repo` runs
    /// `git merge --abort` with nothing in between.
    ///
    /// Why it fires once and never during the abort: git runs the hook after
    /// every index write, with `$1 = 1` only when entries changed. A conflicted
    /// merge writes the index twice, a refresh (`$1 = 0`) and the checkout
    /// (`$1 = 1`), both before MERGE_HEAD is written; the abort writes it once,
    /// with MERGE_HEAD present, after its reset has already succeeded. Gating on
    /// `$1 = 1` and no MERGE_HEAD therefore matches the checkout write alone, so
    /// the abort meets whatever `action` left behind (probed on git 2.50.1,
    /// ticket 28). `core.hooksPath` is set in the local config to an absolute
    /// directory, because a global `core.hooksPath` would otherwise skip
    /// `.git/hooks` silently and the test would pass on the unfixed code.
    fn arm_conflicted_merge_hook(local: &Path, action: &str) {
        use std::os::unix::fs::PermissionsExt;
        let hooks = local.join(".git").join("ticket-28-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("post-index-change");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nD=$(git rev-parse --absolute-git-dir)\n\
                 if [ \"$1\" = 1 ] && [ ! -e \"$D/MERGE_HEAD\" ]; then\n{action}\nfi\nexit 0\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
            local,
        );
    }

    /// The abort after a conflicted merge fails because a file the merge
    /// updated cleanly was changed underneath it (git: "not uptodate"). The
    /// repo stays mid-merge and the report must say so, with git's reason.
    #[test]
    fn pull_repo_reports_a_failed_abort_and_leaves_the_merge_in_place() {
        let (tmp, local) = origin_and_local();
        conflicting_diverge(tmp.path(), &local);
        arm_conflicted_merge_hook(&local, "printf edited > other.txt");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::AbortFailed),
            "a failed abort must report AbortFailed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!result.success(), "AbortFailed is not a success");
        assert!(
            result.message.contains("not uptodate"),
            "the report must carry git's reason, got: {}",
            result.message
        );
        assert!(
            result.message.contains("MERGE_HEAD"),
            "the report must say the repo is left mid-merge, got: {}",
            result.message
        );
        assert!(
            merge_head_present(&local),
            "a failed abort leaves MERGE_HEAD in place"
        );
        assert!(
            std::fs::read_to_string(local.join("base.txt"))
                .unwrap()
                .contains("<<<<<<<"),
            "the conflict markers are still there for the user to resolve"
        );
    }

    /// The abort fails because an `index.lock` appeared after the merge
    /// conflicted. git's first stderr line names the lock; the report carries
    /// that line and nothing more of git's seven-line advice, so the three
    /// lines fit the Running overlay at 80x24.
    #[test]
    fn pull_repo_reports_a_failed_abort_when_the_index_is_locked() {
        let (tmp, local) = origin_and_local();
        conflicting_diverge(tmp.path(), &local);
        arm_conflicted_merge_hook(&local, ": > \"$D/index.lock\"");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::AbortFailed),
            "a locked abort must report AbortFailed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            result.message.contains("index.lock"),
            "the report must carry git's first stderr line, got: {}",
            result.message
        );
        assert_eq!(
            result.message.lines().count(),
            3,
            "what happened, git's first line, what to do: {}",
            result.message
        );
        assert!(
            merge_head_present(&local),
            "a failed abort leaves MERGE_HEAD in place"
        );
    }

    /// An unstaged local edit stops the merge from starting at all. Nothing is
    /// mid-merge, so nothing is aborted: the report is a plain failure with
    /// git's own text, the diverged twin of the blocked fast-forward test.
    #[test]
    fn pull_repo_does_not_abort_a_merge_that_never_started() {
        let (tmp, local) = origin_and_local();
        conflicting_diverge(tmp.path(), &local);
        std::fs::write(local.join("base.txt"), "dirty-local\n").unwrap();

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Failed),
            "a merge that never started must report Failed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            result
                .message
                .starts_with("Merge of origin/main into main failed:"),
            "got: {}",
            result.message
        );
        assert!(
            result.message.contains("would be overwritten"),
            "the report must carry git's reason, got: {}",
            result.message
        );
        assert!(
            !result.message.contains("restored"),
            "nothing was aborted, so nothing was restored: {}",
            result.message
        );
        assert!(!merge_head_present(&local), "no merge was started");
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            "dirty-local\n",
            "the local edit must survive untouched"
        );
    }

    /// The user's own merge is mid-conflict when pull runs. Pull must refuse
    /// before touching anything: on the unfixed code the merge fails to start
    /// and the blanket abort throws the user's merge away, then reports a
    /// clean restore.
    #[test]
    fn pull_repo_leaves_a_pre_existing_merge_alone() {
        let (tmp, local) = origin_and_local();
        conflicting_diverge(tmp.path(), &local);
        git(&["checkout", "-b", "side", "HEAD~1"], &local);
        std::fs::write(local.join("base.txt"), "side-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "side-edit"], &local);
        git(&["checkout", "main"], &local);
        let merge = Cmd::new("git")
            .args(["merge", "--no-edit", "side"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(!merge.status.success(), "the fixture merge must conflict");
        assert!(
            merge_head_present(&local),
            "the fixture leaves a merge in progress"
        );
        let markers_before = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(markers_before.contains("<<<<<<<"));

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Failed),
            "a pre-existing merge must be refused as Failed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            result.message.contains("MERGE_HEAD") && result.message.contains("git merge --abort"),
            "the refusal must name the state and the commands, got: {}",
            result.message
        );
        assert!(
            merge_head_present(&local),
            "the user's merge must still be in progress"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            markers_before,
            "the user's conflict must be untouched"
        );
    }

    /// A ref named `MERGE_HEAD` (here a tag the fetch brings in) must not read
    /// as a merge in progress: git decides that by the file in the git dir,
    /// and so does `pull_repo`. A ref-name lookup (`rev-parse --verify`)
    /// would refuse every pull of a repo whose origin carries such a tag. The
    /// first pull fetches the tag and merges; it is the second pull, with the
    /// tag now in the local refs when the pre-fetch check runs, that tells the
    /// two implementations apart (the coordinator's review of PR #54 found the
    /// one-pull version green on the ref lookup).
    #[test]
    fn pull_repo_ignores_a_ref_named_merge_head() {
        let (tmp, local) = origin_and_local();
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("remote.txt"), "remote\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "remote-edit"], helper);
            git(&["tag", "MERGE_HEAD"], helper);
            git(&["push", "origin", "MERGE_HEAD"], helper);
        });
        std::fs::write(local.join("local.txt"), "local\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "local-edit"], &local);

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Merged),
            "a tag named MERGE_HEAD is not a merge in progress, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(!merge_head_present(&local), "the merge was committed");
        assert!(
            get_sha(&local, "refs/tags/MERGE_HEAD") == get_sha(&local, "origin/main"),
            "the fetch brought the tag in"
        );

        let again = pull_repo(&local);

        // The merge commit puts main ahead of origin, so a second pull is a
        // no-op success; a ref lookup would refuse it as mid-merge instead.
        assert!(
            again.success(),
            "with the tag in the local refs the repo is still not mid-merge, got {:?}: {}",
            again.outcome,
            again.message
        );
    }

    /// A behind-only branch with the user's merge in progress: refused by the
    /// check before the fetch, and git's `merge --ff-only` would refuse too
    /// (probed on git 2.50.1, exit 128), so the user's merge is left alone.
    #[test]
    fn pull_repo_fast_forward_arm_leaves_a_pre_existing_merge_alone() {
        let (tmp, local) = origin_and_local();
        let base_sha = get_sha(&local, "main");
        // origin: base -> B (edits base.txt) -> C (adds remote.txt). Local
        // main moves to B only, so it is strictly behind (0 ahead, 1 behind).
        advance_origin(tmp.path(), |helper| {
            std::fs::write(helper.join("base.txt"), "b-version\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "b-edit"], helper);
            std::fs::write(helper.join("remote.txt"), "remote\n").unwrap();
            git(&["add", "."], helper);
            git(&["commit", "-m", "c-edit"], helper);
        });
        git(&["fetch", "origin"], &local);
        git(&["merge", "--ff-only", "origin/main~1"], &local);
        // A side branch from the base edits base.txt too, so merging it into
        // main (at B) conflicts without moving main.
        git(&["checkout", "-b", "side", &base_sha], &local);
        std::fs::write(local.join("base.txt"), "side-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "side-edit"], &local);
        git(&["checkout", "main"], &local);
        let merge = Cmd::new("git")
            .args(["merge", "--no-edit", "side"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(!merge.status.success(), "the fixture merge must conflict");
        assert!(
            merge_head_present(&local),
            "the fixture leaves a merge in progress"
        );
        assert_eq!(
            git::ahead_behind(&local).unwrap(),
            (0, 1),
            "main is behind only"
        );
        let markers_before = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(markers_before.contains("<<<<<<<"));
        let sha_before = get_sha(&local, "main");

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Failed),
            "git refuses the fast-forward, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            merge_head_present(&local),
            "the user's merge is still in progress"
        );
        assert_eq!(sha_before, get_sha(&local, "main"), "main did not move");
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            markers_before,
            "the user's conflict is untouched"
        );
    }

    /// The refusal does not depend on the branch state: a repo that is up to
    /// date with origin but mid-merge is refused too, not told "nothing to
    /// pull" while conflict markers sit in the tree (found by the independent
    /// reviewer of PR #54 on the version that checked only the diverged arm).
    #[test]
    fn pull_repo_refuses_a_pre_existing_merge_when_up_to_date() {
        let (_tmp, local) = origin_and_local();
        let base_sha = get_sha(&local, "main");
        std::fs::write(local.join("base.txt"), "main-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "main-edit"], &local);
        git(&["push", "origin", "main"], &local);
        git(&["checkout", "-b", "side", &base_sha], &local);
        std::fs::write(local.join("base.txt"), "side-version\n").unwrap();
        git(&["add", "."], &local);
        git(&["commit", "-m", "side-edit"], &local);
        git(&["checkout", "main"], &local);
        let merge = Cmd::new("git")
            .args(["merge", "--no-edit", "side"])
            .current_dir(&local)
            .output()
            .unwrap();
        assert!(!merge.status.success(), "the fixture merge must conflict");
        assert!(
            merge_head_present(&local),
            "the fixture leaves a merge in progress"
        );
        assert_eq!(
            git::ahead_behind(&local).unwrap(),
            (0, 0),
            "main is up to date"
        );
        let markers_before = std::fs::read_to_string(local.join("base.txt")).unwrap();
        assert!(markers_before.contains("<<<<<<<"));

        let result = pull_repo(&local);

        assert!(
            matches!(result.outcome, PullOutcome::Failed),
            "a pre-existing merge must be refused as Failed, got {:?}: {}",
            result.outcome,
            result.message
        );
        assert!(
            result.message.contains("mid-merge") && result.message.contains("git merge --continue"),
            "the refusal must name the state and the commands, got: {}",
            result.message
        );
        assert!(
            merge_head_present(&local),
            "the user's merge is still in progress"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("base.txt")).unwrap(),
            markers_before,
            "the user's conflict is untouched"
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

    /// Ticket 13. One row per clause of the creation rule, and the names the
    /// rule must keep accepting (interior spaces, dots, non-ASCII).
    #[test]
    fn validate_space_name_accepts_ordinary_names_and_rejects_each_clause() {
        for ok in [
            "ws",
            "my space",
            "a\u{a0}b",
            "\u{e9}",
            "v1..v2",
            "a.b",
            "feature-x",
            "x-",
        ] {
            assert!(
                validate_space_name(ok).is_ok(),
                "{:?} is a valid space name",
                ok
            );
        }
        let rejected = [
            ("", "Space name cannot be empty"),
            (" x", "Space name cannot start or end with whitespace"),
            ("x ", "Space name cannot start or end with whitespace"),
            ("a/b", "Space name cannot contain '/' or '\\'"),
            ("/etc/x", "Space name cannot contain '/' or '\\'"),
            ("a\\b", "Space name cannot contain '/' or '\\'"),
            ("-x", "Space name cannot start with '-' or '.'"),
            (".", "Space name cannot start with '-' or '.'"),
            ("..", "Space name cannot start with '-' or '.'"),
            (".hidden", "Space name cannot start with '-' or '.'"),
            (
                "a\nb",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\0b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\x7fb",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{85}b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{2028}b",
                "Space name cannot contain control or formatting characters",
            ),
            // Cf: a bidi override that renders the name reversed, a
            // zero-width space (the lookup-guard table has it inside "..",
            // which here the leading-dot clause answers first), an isolate
            // control.
            (
                "safe\u{202e}elif.exe",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{200b}b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{2066}b",
                "Space name cannot contain control or formatting characters",
            ),
            // The joiner the docs name as refused, and the last code point
            // of the table's last range, so a truncated range is caught.
            (
                "a\u{200d}b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{e007f}b",
                "Space name cannot contain control or formatting characters",
            ),
        ];
        for (name, rule) in rejected {
            let err = validate_space_name(name).expect_err(&format!("{:?} must be rejected", name));
            assert_eq!(err.to_string(), rule, "wrong rule for {:?}", name);
        }
    }

    /// Ticket 13. The lookup guard is looser than the creation rule on
    /// purpose: a hand-made `-scratch` or `.old` space must stay addressable.
    #[test]
    fn require_plain_component_rejects_dot_dot_and_separators() {
        for ok in ["ws", "-scratch", ".old", "a b", "x."] {
            assert!(
                require_plain_component(ok).is_ok(),
                "{:?} is one plain component",
                ok
            );
        }
        let rejected = [
            ("", "Space name cannot be empty"),
            (".", "Space name cannot be '.' or '..'"),
            ("..", "Space name cannot be '.' or '..'"),
            ("a/b", "Space name cannot contain '/' or '\\'"),
            ("/", "Space name cannot contain '/' or '\\'"),
            ("a\\b", "Space name cannot contain '/' or '\\'"),
            (
                "a\tb",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\0b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{9b}b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{2029}b",
                "Space name cannot contain control or formatting characters",
            ),
            (
                ".\u{200b}.",
                "Space name cannot contain control or formatting characters",
            ),
            (
                "a\u{200e}b",
                "Space name cannot contain control or formatting characters",
            ),
        ];
        for (name, rule) in rejected {
            let err =
                require_plain_component(name).expect_err(&format!("{:?} must be rejected", name));
            assert_eq!(err.to_string(), rule, "wrong rule for {:?}", name);
        }
        let err = checked_space_name("..").unwrap_err().to_string();
        assert_eq!(
            err, "invalid space name \"..\": Space name cannot be '.' or '..'",
            "the core wrapper names the offending name"
        );
    }

    /// Ticket 13. The check is git's verdict, not a mirror of it: the rows
    /// are names whose status only git's rule settles.
    #[test]
    fn check_branch_name_agrees_with_git() {
        for ok in ["feature/x", "@", "\u{e9}", "x.y", "a./b"] {
            assert!(
                check_branch_name(ok).is_ok(),
                "git accepts {:?} as a branch name",
                ok
            );
        }
        for bad in [
            "-foo", "-", "HEAD", "a..b", "foo.lock", "a b", ".a", "a.", "",
        ] {
            assert!(
                check_branch_name(bad).is_err(),
                "git rejects {:?} as a branch name",
                bad
            );
        }
        assert_eq!(
            check_branch_name("-foo").unwrap_err().to_string(),
            "'-foo' is not a valid branch name",
            "the error is git's sentence with 'fatal: ' stripped"
        );
    }

    /// Ticket 13. The guard and the entry checks must see the name git will
    /// put after `-b`, which for a remote-tracking form is the stripped one.
    /// Ticket 25: the prefix is any configured remote, `origin` always, the
    /// longest remote when one remote's name is a prefix of another's.
    #[test]
    fn branch_slot_name_is_the_stripped_local_name() {
        let existing = |name: &str| BranchStrategy::ExistingBranch(name.to_string());
        let remotes = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(
            branch_slot_name(&BranchStrategy::NewBranch("-x".to_string()), &[], None),
            Some("-x")
        );
        assert_eq!(
            branch_slot_name(&existing("origin/-M"), &[], None),
            Some("-M"),
            "origin/-M passes git's check as a whole, but -M is what reaches -b"
        );
        assert_eq!(
            branch_slot_name(&existing("origin/-M"), &remotes(&["upstream"]), None),
            Some("-M"),
            "origin counts whether or not it is configured"
        );
        assert_eq!(
            branch_slot_name(&existing("feature/x"), &[], None),
            Some("feature/x")
        );
        assert_eq!(
            branch_slot_name(&BranchStrategy::DetachedHead, &[], None),
            None
        );
        assert_eq!(
            branch_slot_name(&existing("origin/origin/-x"), &[], None),
            Some("origin/-x"),
            "one prefix is stripped, as add_worktree strips one, so the slot \
             name does not begin with a dash and needs no refusal"
        );
        assert_eq!(
            branch_slot_name(&existing("upstream/-M"), &[], None),
            Some("upstream/-M"),
            "a prefix that names no remote is part of the branch name"
        );
        assert_eq!(
            branch_slot_name(&existing("upstream/-M"), &remotes(&["upstream"]), None),
            Some("-M"),
            "a configured remote's prefix is stripped like origin's"
        );
        assert_eq!(
            branch_slot_name(&existing("a/b/-M"), &remotes(&["a", "a/b"]), None),
            Some("-M"),
            "the longest configured remote wins"
        );
        assert_eq!(
            branch_slot_name(&existing("a/b/-M"), &remotes(&["a"]), None),
            Some("b/-M"),
            "and a shorter one when the longer is not configured"
        );
        assert_eq!(
            branch_slot_name(&existing("upstream/"), &remotes(&["upstream"]), None),
            Some("upstream/"),
            "an empty local part is not a split"
        );
    }

    /// Ticket 13. The two `--track` forms' `--` protects their path slot,
    /// which is testable the same way as the plain form: a relative
    /// worktree path beginning with `-` against a branch that exists on
    /// the remote and not locally (new branch) or is named by its remote
    /// shorthand (existing branch).
    #[test]
    fn a_dash_path_is_a_path_in_both_track_forms() {
        for (label, strategy) in [
            (
                "new branch that exists on the remote",
                BranchStrategy::NewBranch("feat".to_string()),
            ),
            (
                "existing branch by remote shorthand",
                BranchStrategy::ExistingBranch("origin/feat".to_string()),
            ),
        ] {
            let (_tmp, local) = origin_and_local();
            git(&["push", "origin", "main:feat"], &local);
            git(&["fetch", "origin"], &local);
            let created = add_worktree(
                &local,
                Path::new("-dashout"),
                "main".to_string(),
                &strategy,
                existing_split(&local, &strategy, &[]),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "{}: with -- before the path, -dashout is a path: {}",
                    label, e
                )
            });
            assert_eq!(created, Path::new("-dashout"), "{}", label);
            assert!(
                local.join("-dashout").join(".git").exists(),
                "{}: the worktree was created at the dash-named relative path",
                label
            );
            // Which argv form ran is pinned by the upstream it left: the
            // `--track` forms set it to origin/feat, while the new-branch
            // form off the base would set origin/main. Without this the
            // new-branch arm could silently drift to the base form and
            // still pass. The plain form is not distinguished: git's DWIM
            // treats a branch that exists only as one remote-tracking ref
            // as `--track -b`, so it leaves origin/feat too, and its `--`
            // is the same property this test pins.
            let upstream = Cmd::new("git")
                .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
                .current_dir(local.join("-dashout"))
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&upstream.stdout).trim(),
                "origin/feat",
                "{}: the --track form ran, not another form with --",
                label
            );
        }
    }

    /// Ticket 13. The one `git worktree add` form whose commit-ish cannot
    /// carry a dash (the local branch already exists, so the name passed the
    /// guard) still has a path slot, and that is what its `--` protects: a
    /// relative worktree path beginning with `-` is a path, not an option.
    #[test]
    fn a_dash_path_is_a_path_when_the_local_branch_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        git(&["branch", "topic", "main"], &repo);

        let created = add_worktree(
            &repo,
            Path::new("-dashdir"),
            "main".to_string(),
            &BranchStrategy::NewBranch("topic".to_string()),
            None,
        )
        .expect("with -- before the path, -dashdir is a path");
        assert_eq!(created, Path::new("-dashdir"));
        assert!(
            repo.join("-dashdir").join(".git").exists(),
            "the worktree was created at the dash-named relative path"
        );
    }

    /// Ticket 13. The `--` in every `git worktree add` form is only provable
    /// by putting a dash in the commit-ish slot and reading git's answer, and
    /// `create_worktree_cancellable` refuses such a branch before git runs,
    /// so this calls `add_worktree` directly. Without `--` git says
    /// `unknown switch 'o'` followed by its usage text. Since ticket 24 the
    /// base reaches git as `refs/heads/-foo`, so only the plain
    /// existing-branch form still puts a leading dash in that slot; the two
    /// base forms pin that git sees a reference, whatever its spelling.
    #[test]
    fn a_dash_commit_ish_is_reported_as_an_invalid_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = plain_repo(tmp.path(), "repo");
        let spaces = tmp.path().join("spaces");
        std::fs::create_dir_all(spaces.join("ws")).unwrap();
        let wt_path = spaces.join("ws").join("repo");

        let cases: [(&str, String, BranchStrategy, &str); 3] = [
            (
                "existing branch",
                "main".to_string(),
                BranchStrategy::ExistingBranch("-foo".to_string()),
                "invalid reference: -foo",
            ),
            (
                "detached at base",
                "-foo".to_string(),
                BranchStrategy::DetachedHead,
                "invalid reference: refs/heads/-foo",
            ),
            (
                "new branch off base",
                "-foo".to_string(),
                BranchStrategy::NewBranch("topic".to_string()),
                "invalid reference: refs/heads/-foo",
            ),
        ];
        for (label, base, strategy, expected) in cases {
            let err = add_worktree(&repo, &wt_path, base, &strategy, None)
                .expect_err("a dash commit-ish cannot resolve");
            let text = err.to_string();
            assert!(
                text.contains(expected),
                "{}: git must see -foo as a reference, not an option, got {:?}",
                label,
                text
            );
            assert!(
                !text.contains("unknown switch"),
                "{}: an option parse means the -- is missing, got {:?}",
                label,
                text
            );
        }
    }
}
