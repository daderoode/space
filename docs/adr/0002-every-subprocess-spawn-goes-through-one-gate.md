# Every subprocess spawn goes through one process-wide gate

On macOS, std makes the pipes behind `Stdio::piped()`, and on its fork path the
pipe it reads exec errors from, with `pipe()` and only then sets close-on-exec on
each end (Rust 1.96.0, `library/std/src/sys/pipe/unix.rs`). Linux uses
`pipe2(O_CLOEXEC)` and has no such gap. A process started by another thread
inside that gap inherits the write end and keeps it for its whole life. The
inheriting spawn can take either path: `fork` copies every descriptor, and
Apple's `posix_spawn` keeps every one without close-on-exec, because std never
sets `POSIX_SPAWN_CLOEXEC_DEFAULT`. Two things follow:

- When the inherited end is the exec-error pipe, `Command::spawn` blocks until
  the unrelated child exits. `pre_exec` forces that path, and every unattended
  run uses `pre_exec(setsid)`, so an unattended run could outlive its
  wall-clock limit by the life of any concurrent child, since the limit starts
  only after `spawn` returns.
- When it is a stdio pipe, a stderr reader or `Command::output` waits for
  end-of-file just as long, and nothing bounds an `output` wait.

A two-thread probe with no app code showed both. A `pre_exec` victim's
`spawn()` blocked 4 times and its stderr was held 5 times in 128,874 calls,
each time for the 0.30 s life of a concurrent `sleep 0.3`.

So one `Mutex`, `space::SPAWN_GATE` in `lib.rs`, is the process's spawn gate.
The `spawn`, `output` and `status` in `src/core/spawn.rs` hold it across
`Command::spawn` and never across a wait, and every production spawn goes
through them. With no two spawns overlapping, no process is created
while another spawn's pipe is still inheritable. `clippy.toml` disallows the raw
`Command::spawn`, `Command::output` and `Command::status`, so a production call
site that skips the gate shows up as one more clippy warning.

## Considered options

- **Drop `pre_exec` so std can use `posix_spawn`.** This is the obvious fix,
  and it fails twice over.
  - It removes only the blocked `spawn()`. Stdio pipes have the gap on either
    path: with `process_group(0)` the same probe still saw stderr held 6 times
    in 173,407 calls.
  - It gives up the controlling-terminal half of the unattended-run policy. On
    Apple, std reaches `posix_spawn` only without a new session (`setsid` is
    unstable, and off linux-gnu std falls back to fork for it anyway).
    `process_group(0)` leaves the child in the app's session: under a pty, a
    child reading `/dev/tty` was stopped by SIGTTIN and would have waited out
    the whole limit. With `setsid`, the open fails at once with `Device not
    configured`. Without a terminal, as in most test harnesses, both kinds
    fail at once, so only a pty shows the difference.
- **Call `libc::posix_spawn` directly, with `POSIX_SPAWN_CLOEXEC_DEFAULT` and
  `POSIX_SPAWN_SETSID`.** A child started that way inherits only the
  descriptors it is handed, whoever made the rest, and still gets a new
  session. So it would also cover spawns the lint cannot see. It was not taken
  for three reasons:
  - std cannot build a `std::process::Child` from a pid it did not start. The
    waiting, polling, killing and stderr capture in `run_unattended`, and in
    every `output` caller, would all have to be rebuilt over raw pids.
  - The argv, environment, working directory and file actions would need
    unsafe FFI.
  - It would only work on macOS, because `POSIX_SPAWN_CLOEXEC_DEFAULT` is an
    Apple extension, and Linux has no gap to close.
- **Start the limit before `spawn`.** A blocked `Command::spawn` cannot be
  interrupted, so the limit would still be enforced only once `spawn`
  returned, and `output` has no limit to move.
- **Accept and document.** The TUI does reach the case: a git op left with Esc
  keeps spawning after its network fetch. That can hold an unattended fetch's
  `spawn` before its limit starts, or freeze the UI thread on an `output`. And
  the gate is cheap.

## Consequences

- **The gate is held briefly.** It is held from before std makes the pipes
  until the fork (`posix_spawn`) or the child's exec (the fork path). Under the
  probe's load, three threads spawning back to back while four more kept 32
  children alive, the longest wait for it was 38 ms. Waits run outside it,
  which `waits_happen_outside_the_gate` pins.
- **The limit guarantee holds again.** An unattended run's `spawn` now waits
  only on other threads' spawns, never on a concurrent child, so CONTEXT.md's
  **Unattended run** ("the limit is what guarantees the end") stays true
  without an edit.
- **`output` sets its own stdio.** It sets stdin null and stdout and stderr
  piped whatever the caller set, because a `Command` cannot report what was
  set. Callers set no stdio of their own.
- **There is one gate per process by construction.** The gate is
  `space::SPAWN_GATE`, a static in `lib.rs`. `main.rs` compiles its own copy of
  `core` rather than using the library's, so one process can hold two copies
  of `spawn.rs`.
  - Both copies lock the gate through the crate name. The library aliases
    itself to that name (`extern crate self as space`), so the name resolves
    to the library from either copy, and both lock the same static.
  - The model tests hold the gate by that name, so an entry point that locked
    anything else would fail them.
  - Two more tests lock it by name from outside `spawn.rs`, one in `lib.rs` and
    one in `main.rs`. Each checks that `core::spawn` cannot start a child
    meanwhile, so a copy that went back to a static of its own would fail in
    the library or in the binary.
- **Test fixtures start git directly.** They allow the lint at their module or
  crate root, so the gate does not order them against anything. The effect
  runs both ways:
  - A fixture's child can inherit a gated spawn's pipe. Fixtures start only
    short git commands, so that child lives only milliseconds, and that is
    all it can add to a gated run. Every long-lived child in the suite starts
    through the gate. The one exception is the control test's child, which
    starts ungated but only while that test holds the gate, when no gated
    spawn can be in its gap.
  - A child started while a fixture's own pipe is in its gap keeps that pipe,
    and the fixture waits for the child's life. For a gated child that can be
    seconds: up to its test's limit plus the kill grace, or until its test
    releases it. For the control's ungated child it is about half a second.
    This slows the fixture, not the gated call; no test times a fixture.
- **The lint cannot see spawns made in a dependency**, or through `libc`
  directly. There are none today: git2 is built without ssh or https, and rmcp
  without its child-process transport.
