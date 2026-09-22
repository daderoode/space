// Lets code compiled into this library name it `space`, as the binary and the
// integration tests do. `core::spawn` locks `::space::SPAWN_GATE`, and
// `tests/common/git_isolation.rs`, included below in this library's unit
// tests, starts its child through `space::core::spawn`.
extern crate self as space;

pub mod core;
pub mod logging;
pub mod mcp;
pub mod shell;
pub mod tui;

/// The one spawn gate in the process; see `core::spawn`. The binary takes
/// `core` from this library rather than compiling its own copy (ticket 29),
/// so a process holds one `core::spawn`, and it locks this static by name.
#[doc(hidden)]
pub static SPAWN_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Keeps this binary's tests off the invoking user's git config (ticket 43).
#[cfg(test)]
#[path = "../tests/common/git_isolation.rs"]
mod git_isolation;

#[cfg(test)]
mod tests {
    /// `core::spawn` locks this static and not one of its own: held by name
    /// here, outside `spawn.rs`, it keeps a spawn from starting.
    #[test]
    fn core_spawn_waits_on_the_library_gate() {
        let gate = crate::SPAWN_GATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            !crate::core::spawn::tests::a_spawn_starts_while(gate),
            "a spawn started while SPAWN_GATE was held"
        );
    }
}
