//! Every subprocess the app starts goes through this module's gate.
//!
//! On macOS, std makes the pipes behind `Stdio::piped()`, and on its fork path
//! the pipe it reads exec errors from, with `pipe()` and only then sets
//! close-on-exec on each end (Rust 1.96.0, `library/std/src/sys/pipe/unix.rs`).
//! A process started by another thread inside that gap inherits the write end
//! and keeps it for its whole life, so whoever reads the other end waits for an
//! unrelated child to exit. When the inherited end is the exec-error pipe,
//! `Command::spawn` itself blocks; `pre_exec` forces that path, and every
//! unattended run uses `pre_exec`. When it is a stdio pipe, a stderr reader or
//! `Command::output` never sees end-of-file. Linux creates both pipes with
//! close-on-exec already set, so there the gate costs one uncontended lock per
//! spawn.
//!
//! The gate closes the gap by never letting two spawns overlap. It is held from
//! before std makes the pipes until `Command::spawn` returns, which is after the
//! fork (`posix_spawn`) or after the child's exec (the fork path), and never
//! across a wait. `clippy.toml` disallows the raw `Command::spawn`,
//! `Command::output` and `Command::status`, so a call site that skips the gate
//! is a clippy warning. ADR 0002 records the decision and the alternatives it
//! rejected.
//!
//! The gate is `space::SPAWN_GATE`, a static in `lib.rs`, locked through the
//! crate name. The binary takes `core` from the library rather than compiling
//! its own copy (ticket 29), so a process holds one copy of this module and one
//! gate.

use std::io;
use std::process::{Child, Command, Output, Stdio};
use std::sync::MutexGuard;

fn enter() -> MutexGuard<'static, ()> {
    // std's fork path panics inside `spawn` when its exec-error pipe fails,
    // which poisons the gate. The gate guards no data, so later spawns go on.
    ::space::SPAWN_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// `Command::spawn` behind the gate.
#[allow(clippy::disallowed_methods)] // the one place allowed to call it
pub fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let _gate = enter();
    cmd.spawn()
}

/// `Command::output` with only the spawn behind the gate: the wait for the
/// child runs outside it. stdin is null and stdout and stderr are piped, which
/// is what `Command::output` does with streams the caller left unset. This
/// sets them whatever the caller set, because a `Command` cannot report what
/// was set, so callers set no stdio of their own.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn(cmd)?.wait_with_output()
}

/// Hold the gate for as long as the guard lives, so a test can make a pipe the
/// way std does and show that nothing started through this module inherits it.
/// It locks the gate by its own name rather than through `enter`, so an entry
/// point that locked anything else would be caught.
#[cfg(test)]
pub(crate) fn hold() -> MutexGuard<'static, ()> {
    ::space::SPAWN_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The file-gated hold the integration tests share, compiled into the unit
/// tests from the same file so there is one copy (ticket 31). The path is
/// relative to this file's directory.
#[cfg(test)]
#[path = "../../tests/common/hold.rs"]
pub(crate) mod hold;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    /// How long the pipe is left without close-on-exec while `start` runs. An
    /// ungated start creates its child well inside it (under a millisecond
    /// here), and a gated one cannot create it at all until the gap has
    /// closed, so a gated test always runs the whole gap. Every other gated
    /// spawn in the test binary waits it out. Only the library's unit-test
    /// binary compiles these tests, and it has four such tests: the three
    /// gated model tests, plus the by-name test in `lib.rs`. Together they
    /// cost up to 0.8s of gate time. Timing bounds elsewhere in the suite
    /// leave room for that, and tightening one of them should account for it.
    /// A machine too loaded to start a child within the gap lets an ungated
    /// entry point pass one run, but never fails a gated one.
    const GAP: Duration = Duration::from_millis(200);
    /// A backstop, not a measurement: with nothing holding the pipe,
    /// end-of-file arrives as soon as the test closes its own write end.
    const HOLD_LIMIT: Duration = Duration::from_secs(5);

    /// A child that records that it started, then waits for the test to
    /// release it: the shared hold as a `sh -c` script, with no tail. Its
    /// module doc says how it gives up once the test cannot release it; its
    /// cap sits well above `HOLD_LIMIT`, so the tests' own bounds report
    /// first.
    fn waiting_child(hold: &hold::Hold) -> Command {
        let mut child = Command::new("/bin/sh");
        child.arg("-c").arg(hold.script(""));
        child
    }

    /// Make a pipe while holding the gate, the way std makes one on macOS: two
    /// descriptors, neither yet close-on-exec. Keep that gap open while `start`
    /// runs a child on another thread, until the child has started or `GAP`
    /// has passed. Then set close-on-exec, release the gate, close this side's
    /// write end, and report whether the child kept a copy of it, that is
    /// whether end-of-file waited for the child to be released.
    pub(crate) fn a_child_started_in_the_gap_holds_the_pipe(
        start: impl FnOnce(Command) + Send + 'static,
    ) -> bool {
        holds_the_pipe_after(GAP, HOLD_LIMIT, start)
    }

    fn holds_the_pipe_after(
        gap: Duration,
        eof_within: Duration,
        start: impl FnOnce(Command) + Send + 'static,
    ) -> bool {
        let tmp = tempfile::tempdir().unwrap();
        let waiting = hold::Hold::new(tmp.path(), "gap");
        let child = waiting_child(&waiting);

        let gate = hold();
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` has room for the two descriptors pipe(2) writes.
        let made = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(made, 0, "pipe: {}", io::Error::last_os_error());
        // SAFETY: pipe(2) just returned both descriptors and nothing else owns
        // them; owning them here closes them on every path, a panic included.
        let (read_end, write_end) =
            unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };

        let starter = std::thread::spawn(move || start(child));
        let gap_ends = Instant::now() + gap;
        while !waiting.is_holding() && Instant::now() < gap_ends {
            std::thread::sleep(Duration::from_millis(2));
        }
        for fd in [&read_end, &write_end] {
            // SAFETY: FIOCLEX on a descriptor this function owns.
            let set = unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIOCLEX) };
            assert_eq!(set, 0, "FIOCLEX: {}", io::Error::last_os_error());
        }
        drop(gate);
        drop(write_end);

        let mut ready = libc::pollfd {
            fd: read_end.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for a descriptor this function owns.
        let polled = unsafe { libc::poll(&mut ready, 1, eof_within.as_millis() as libc::c_int) };
        assert!(polled >= 0, "poll: {}", io::Error::last_os_error());
        let held = polled == 0;

        waiting.release();
        let mut rest = Vec::new();
        File::from(read_end).read_to_end(&mut rest).unwrap();
        starter.join().unwrap();
        held
    }

    /// Whether `output` starts a child while `gate` is held. The caller locks
    /// the gate by name from outside this module, so this checks the static
    /// `enter` really locks without going through `hold`, which would move
    /// with it. The child is released once the gate drops and must then run,
    /// so a harness that cannot see a start cannot pass.
    pub(crate) fn a_spawn_starts_while(gate: MutexGuard<'static, ()>) -> bool {
        let tmp = tempfile::tempdir().unwrap();
        let started = tmp.path().join("started");
        let mut child = Command::new("/bin/sh");
        child.arg("-c").arg(format!(": > '{}'", started.display()));
        let starter = std::thread::spawn(move || output(&mut child).unwrap());
        let gap_ends = Instant::now() + GAP;
        while !started.exists() && Instant::now() < gap_ends {
            std::thread::sleep(Duration::from_millis(2));
        }
        let started_while_held = started.exists();
        drop(gate);
        let out = starter.join().unwrap();
        assert!(
            out.status.success() && started.exists(),
            "the child must run once the gate is released: {:?}",
            out
        );
        started_while_held
    }

    #[test]
    fn a_child_started_through_spawn_cannot_inherit_a_pipe_made_under_the_gate() {
        let held = a_child_started_in_the_gap_holds_the_pipe(|mut child| {
            spawn(&mut child).unwrap().wait().unwrap();
        });
        assert!(!held, "a child started through `spawn` kept the pipe open");
    }

    #[test]
    fn a_child_started_through_output_cannot_inherit_a_pipe_made_under_the_gate() {
        let held = a_child_started_in_the_gap_holds_the_pipe(|mut child| {
            output(&mut child).unwrap();
        });
        assert!(!held, "a child started through `output` kept the pipe open");
    }

    /// The control for the three tests above: with no gate at all, a child
    /// started in the gap keeps the pipe, so the harness can see what they
    /// assert is absent. The gap may run to `HOLD_LIMIT` here because an
    /// ungated child starts, and closes the gap, within milliseconds. The
    /// child keeps the pipe until the test releases it, so half a second
    /// without end-of-file is enough to call it held.
    #[test]
    #[allow(clippy::disallowed_methods)] // the ungated spawn is the point
    fn a_child_started_without_the_gate_holds_the_pipe() {
        let held = holds_the_pipe_after(HOLD_LIMIT, Duration::from_millis(500), |mut child| {
            child.spawn().unwrap().wait().unwrap();
        });
        assert!(held, "a child started with no gate did not keep the pipe");
    }

    /// `output` gives the child a null stdin and captures stdout and stderr,
    /// whatever the caller set, as its doc says. A git child that kept the
    /// TUI's stdin would read the user's keys.
    #[test]
    fn output_sets_every_stream_whatever_the_caller_set() {
        let out = output(
            Command::new("/bin/sh")
                .args([
                    "-c",
                    "[ /dev/stdin -ef /dev/null ] && echo null || echo other; echo err >&2",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "null\n");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
    }

    /// std's fork path can panic inside `Command::spawn` while the gate is
    /// held, which poisons it. The gate guards no data, so later spawns must
    /// still get through.
    #[test]
    fn a_panic_while_the_gate_is_held_does_not_stop_later_spawns() {
        let poisoner = std::thread::spawn(|| {
            let _gate = hold();
            std::panic::resume_unwind(Box::new("poison the spawn gate"));
        });
        assert!(poisoner.join().is_err());
        assert!(
            ::space::SPAWN_GATE.is_poisoned(),
            "the panic must have poisoned the gate"
        );

        let spawned = output(&mut Command::new("/usr/bin/true"));
        ::space::SPAWN_GATE.clear_poison();
        assert!(
            spawned.is_ok_and(|out| out.status.success()),
            "a spawn after the gate was poisoned must still run"
        );
    }

    /// The gate covers the spawn and nothing after it: a child that has not
    /// exited must not stop another thread from starting one. The first child
    /// waits for a file that is written only once the second `output` has
    /// returned, so a gate held across the first wait never lets the second
    /// start. Files order the two; the only clock is a backstop.
    #[test]
    fn waits_happen_outside_the_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let waiting = hold::Hold::new(tmp.path(), "first");
        let mut first = waiting_child(&waiting);
        let first = std::thread::spawn(move || output(&mut first).unwrap());
        waiting.wait_holding(HOLD_LIMIT, "the first child did not start", || {
            first
                .is_finished()
                .then(|| "the first child ended before it started".to_string())
        });

        let second = std::thread::spawn(|| output(&mut Command::new("/usr/bin/true")).unwrap());
        let backstop = Instant::now() + HOLD_LIMIT;
        while !second.is_finished() && Instant::now() < backstop {
            std::thread::sleep(Duration::from_millis(2));
        }
        let second_returned = second.is_finished();
        waiting.release();
        first.join().unwrap();
        second.join().unwrap();
        assert!(
            second_returned,
            "a second `output` could not return while the first child was still running"
        );
    }
}
