// `core::spawn` locks `::space::SPAWN_GATE`. The binary compiles its own copy
// of `core` rather than using this library's. In the binary, Cargo makes the
// name `space` mean this library; this alias makes it mean this library here
// too, so both copies of `core::spawn` lock the one static below.
extern crate self as space;

pub mod core;
pub mod logging;
pub mod mcp;
pub mod shell;
pub mod tui;

/// The one spawn gate in the process; see `core::spawn`. It lives in the one
/// file only the library compiles, so both copies of `core::spawn` lock this
/// same static.
#[doc(hidden)]
pub static SPAWN_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    /// `core::spawn` locks this static and not one of its own: held by name
    /// here, in the one file only the library compiles, it keeps a spawn from
    /// starting.
    #[test]
    fn core_spawn_waits_on_the_library_gate() {
        let gate = crate::SPAWN_GATE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            !crate::core::spawn::tests::a_spawn_starts_while(gate),
            "a spawn started while SPAWN_GATE was held"
        );
    }
}
