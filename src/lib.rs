// `core::spawn` locks `::space::SPAWN_GATE`. The binary compiles its own copy
// of `core` rather than using this library's, and the crate name resolves to
// this library from both copies; this alias makes it resolve here too.
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
