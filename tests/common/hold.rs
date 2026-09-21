//! One file-gated hold for every test that keeps a child process open until
//! the test releases it, and one answer to how that hold ends when the test
//! dies (ticket 31).
//!
//! The helper is a `sh` loop. It marks `holding` when it starts, waits, and
//! records in `released` that it saw `release`, so a test can prove from the
//! helper's side that the hold spanned what it needed to span. It gives up
//! without that record, exiting 1 and running no tail, once the test can no
//! longer release it:
//!
//! - `holding` is gone: the test's TempDir dropped, because it returned or
//!   panicked.
//! - the test pid is gone: the test binary was killed. This is the only guard
//!   a SIGKILL leaves, and the only one that reaches a helper inside a git
//!   started with `setsid`, which outlives the test binary on its own.
//! - the cap: an iteration count for when both fail (the pid reused within a
//!   poll, a `remove_dir_all` that errored before reaching the marker, a test
//!   hanging on a `join` because its release never came).
//!
//! Four fixtures use it: the Esc-cancel `post-checkout` hook and the gated
//! upload-pack in the integration tests, the spawn gate's waiting child and
//! the held failing upload-pack in the unit tests. Each is reached through
//! `mod common` or a `#[path]` include, so nothing here ships in the binary.
//! `sh` is fine: the app documents macOS and Linux only.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Seconds between polls, as the `sleep` argument. Every iteration also pays
/// a `sleep` process, so the cap's wall time is not the count times this.
pub const POLL: &str = "0.01";

/// Iterations before a helper gives up on its own. Well above every
/// wall-clock bound in the fixtures (5 s, 20 s and 60 s), so those fire
/// first and say what happened in their own words; the cap is only the
/// backstop that ends a hold nobody can release. Measured standalone at
/// 92 s and 107 s on this machine at load averages of about 3, not derived
/// from the count (ticket 22 measured the same loop at 98 s and 103 s).
pub const CAP: u32 = 6000;

/// The three files of one hold, and the pid and cap its script checks.
pub struct Hold {
    /// Written by the helper when it starts; its absence ends the hold.
    pub holding: PathBuf,
    /// Written by the test to end the hold.
    pub release: PathBuf,
    /// Written by the helper only if it saw `release`.
    pub released: PathBuf,
    pid: u32,
    cap: u32,
}

impl Hold {
    /// A hold whose files live in `dir` as `<prefix>-holding`,
    /// `<prefix>-release` and `<prefix>-released`, watching this test process
    /// with the shared cap.
    pub fn new(dir: &Path, prefix: &str) -> Self {
        Self {
            holding: dir.join(format!("{prefix}-holding")),
            release: dir.join(format!("{prefix}-release")),
            released: dir.join(format!("{prefix}-released")),
            pid: std::process::id(),
            cap: CAP,
        }
    }

    /// The same hold watching another pid, for proving the pid guard.
    pub fn watching_pid(mut self, pid: u32) -> Self {
        self.pid = pid;
        self
    }

    /// The same hold with another cap, for proving the cap.
    pub fn with_cap(mut self, cap: u32) -> Self {
        self.cap = cap;
        self
    }

    /// The helper as a `sh` script. `tail` runs only after a release was seen
    /// and recorded, so a helper that gave up never does the fixture's work
    /// (serve a fetch, succeed as a hook); it exits 1 instead.
    pub fn script(&self, tail: &str) -> String {
        format!(
            "#!/bin/sh\n\
             : > '{holding}'\n\
             i=0\n\
             while [ -e '{holding}' ] && [ ! -e '{release}' ] && kill -0 {pid} 2>/dev/null \\\n\
             && [ $i -lt {cap} ]\n\
             do i=$((i+1)); sleep {poll}; done\n\
             [ -e '{release}' ] || exit 1\n\
             : > '{released}'\n\
             {tail}\n",
            holding = self.holding.display(),
            release = self.release.display(),
            released = self.released.display(),
            pid = self.pid,
            cap = self.cap,
            poll = POLL,
            tail = tail,
        )
    }

    /// Write the script to `path` and make it executable, for fixtures that
    /// point git at a file (a hook, an upload-pack).
    pub fn write_script(&self, path: &Path, tail: &str) {
        std::fs::write(path, self.script(tail)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Whether the helper has started holding.
    pub fn is_holding(&self) -> bool {
        self.holding.exists()
    }

    /// Wait for the helper to start holding. `fail_fast` is polled first on
    /// every turn and its message fails the test at once, for the signature
    /// of a hold that was skipped (the worker got past the held repo, the
    /// held fetch already returned); `what` is the whole message when
    /// `deadline` passes instead, with the deadline appended.
    pub fn wait_holding(
        &self,
        deadline: Duration,
        what: &str,
        mut fail_fast: impl FnMut() -> Option<String>,
    ) {
        let until = Instant::now() + deadline;
        while !self.is_holding() {
            if let Some(message) = fail_fast() {
                panic!("{message}");
            }
            assert!(Instant::now() < until, "{what} (waited {deadline:?})");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// End the hold.
    pub fn release(&self) {
        std::fs::write(&self.release, "").unwrap();
    }

    /// Whether the helper recorded that it saw the release, which is what
    /// makes a passing run proof that the hold spanned the test's action
    /// rather than ending on its own.
    pub fn saw_release(&self) -> bool {
        self.released.exists()
    }
}
