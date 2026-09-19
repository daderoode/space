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
//! The gate is a static, so there is one per compiled copy of this module, and
//! `main.rs` compiles its own copy of `core` rather than using the library's.
//! A process therefore has a single gate only while the binary reaches the
//! library through nothing but `space::logging`, which never spawns;
//! `the_binary_reaches_the_library_only_through_logging` holds it to that.

use std::io;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Mutex, MutexGuard};

static GATE: Mutex<()> = Mutex::new(());

fn enter() -> MutexGuard<'static, ()> {
    // std's fork path panics inside `spawn` when its exec-error pipe fails,
    // which poisons the gate. The gate guards no data, so later spawns go on.
    GATE.lock().unwrap_or_else(|e| e.into_inner())
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

/// `Command::status` with only the spawn behind the gate: the wait for the
/// child runs outside it. Streams the caller left unset are inherited, as
/// `Command::status` does.
pub fn status(cmd: &mut Command) -> io::Result<ExitStatus> {
    spawn(cmd)?.wait()
}

/// Hold the gate for as long as the guard lives, so a test can make a pipe the
/// way std does and show that nothing started through this module inherits it.
#[cfg(test)]
pub(crate) fn hold() -> MutexGuard<'static, ()> {
    enter()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// How long the pipe is left without close-on-exec while `start` runs. An
    /// ungated start creates its child well inside it (under a millisecond
    /// here), and a gated one cannot create it at all until the gap has
    /// closed. Every gated spawn in the test binary waits out this gap, so it
    /// stays short.
    const GAP: Duration = Duration::from_millis(200);
    /// A backstop, not a measurement: with nothing holding the pipe,
    /// end-of-file arrives as soon as the test closes its own write end.
    const HOLD_LIMIT: Duration = Duration::from_secs(5);

    /// A child that records that it started, then waits for the test to
    /// release it. It gives up on its own once the test cannot release it any
    /// more: the directory goes when the test's TempDir is dropped, and the pid
    /// goes when the test binary exits.
    fn waiting_child(dir: &Path, started: &Path, release: &Path) -> Command {
        let mut child = Command::new("/bin/sh");
        child.arg("-c").arg(format!(
            ": > '{started}'\n\
             while [ ! -e '{release}' ] && [ -d '{dir}' ] && kill -0 {pid} 2>/dev/null\n\
             do sleep 0.01; done\n",
            started = started.display(),
            release = release.display(),
            dir = dir.display(),
            pid = std::process::id()
        ));
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
        let tmp = tempfile::tempdir().unwrap();
        let started = tmp.path().join("started");
        let release = tmp.path().join("release");
        let child = waiting_child(tmp.path(), &started, &release);

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
        let gap_ends = Instant::now() + GAP;
        while !started.exists() && Instant::now() < gap_ends {
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
        let polled = unsafe { libc::poll(&mut ready, 1, HOLD_LIMIT.as_millis() as libc::c_int) };
        assert!(polled >= 0, "poll: {}", io::Error::last_os_error());
        let held = polled == 0;

        std::fs::write(&release, "").unwrap();
        let mut rest = Vec::new();
        File::from(read_end).read_to_end(&mut rest).unwrap();
        starter.join().unwrap();
        held
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

    #[test]
    fn a_child_started_through_status_cannot_inherit_a_pipe_made_under_the_gate() {
        let held = a_child_started_in_the_gap_holds_the_pipe(|mut child| {
            status(&mut child).unwrap();
        });
        assert!(!held, "a child started through `status` kept the pipe open");
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
        assert!(GATE.is_poisoned(), "the panic must have poisoned the gate");

        let spawned = output(&mut Command::new("/usr/bin/true"));
        GATE.clear_poison();
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
        let started = tmp.path().join("started");
        let release = tmp.path().join("release");
        let mut first = waiting_child(tmp.path(), &started, &release);
        let first = std::thread::spawn(move || output(&mut first).unwrap());
        while !started.exists() {
            assert!(
                !first.is_finished(),
                "the first child ended before it started"
            );
            std::thread::sleep(Duration::from_millis(2));
        }

        let second = std::thread::spawn(|| output(&mut Command::new("/usr/bin/true")).unwrap());
        let backstop = Instant::now() + HOLD_LIMIT;
        while !second.is_finished() && Instant::now() < backstop {
            std::thread::sleep(Duration::from_millis(2));
        }
        let second_returned = second.is_finished();
        std::fs::write(&release, "").unwrap();
        first.join().unwrap();
        second.join().unwrap();
        assert!(
            second_returned,
            "a second `output` could not return while the first child was still running"
        );
    }

    /// See the module doc: the binary compiles its own `core`, so a spawn it
    /// reached through the library would run behind a second gate. Code in a
    /// module the library also compiles cannot name the library by its crate
    /// name, since the library cannot name itself, so only the binary's own
    /// modules can. Those are the ones `main.rs` declares and `lib.rs` does not.
    /// In them, only `space::logging`, which never spawns, may be named.
    #[test]
    fn the_binary_reaches_the_library_only_through_logging() {
        use std::path::PathBuf;

        fn declared_modules(file: &Path) -> Vec<String> {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    let line = line.strip_prefix("pub ").unwrap_or(line);
                    let name = line.strip_prefix("mod ")?.strip_suffix(';')?;
                    Some(name.to_string())
                })
                .collect()
        }
        fn rust_files(path: &Path, out: &mut Vec<PathBuf>) {
            if path.is_dir() {
                for entry in std::fs::read_dir(path).unwrap() {
                    rust_files(&entry.unwrap().path(), out);
                }
            } else if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
                out.push(path.to_path_buf());
            }
        }

        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let library = declared_modules(&src.join("lib.rs"));
        let binary_only: Vec<String> = declared_modules(&src.join("main.rs"))
            .into_iter()
            .filter(|m| !library.contains(m))
            .collect();
        assert!(
            !binary_only.is_empty(),
            "found no module only the binary declares"
        );
        let mut files = vec![src.join("main.rs")];
        for module in &binary_only {
            rust_files(&src.join(format!("{module}.rs")), &mut files);
            rust_files(&src.join(module), &mut files);
        }

        let mut offending = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            for (number, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for (at, _) in code.match_indices("space::") {
                    let before = code[..at].chars().next_back();
                    let names_the_crate = !before.is_some_and(|c| c.is_alphanumeric() || c == '_');
                    if names_the_crate && !code[at..].starts_with("space::logging") {
                        offending.push(format!("{}:{}", file.display(), number + 1));
                    }
                }
            }
        }
        assert!(
            offending.is_empty(),
            "the binary reaches the library other than through space::logging at {:?}; \
             a spawn reached that way runs behind the library's gate, not the binary's",
            offending
        );
    }
}
