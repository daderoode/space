use std::path::PathBuf;
use tracing_appender::rolling::Rotation;

/// Initialise file-based logging. Call once from `main` before any other work.
///
/// Returns the `WorkerGuard` that must be kept alive for the process lifetime.
/// Dropping it stops the background flush thread and buffered logs may be lost.
///
/// # Environment variables
///
/// - `SPACE_LOG=off`          -- disable logging entirely
/// - `SPACE_LOG=/path/to/dir` -- override log directory
/// - `SPACE_LOG_LEVEL=debug`  -- verbose logging (default: info)
/// - `SPACE_LOG_LEVEL=off`    -- disable logging
pub fn init() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::fmt;

    let level_str = std::env::var("SPACE_LOG_LEVEL").unwrap_or_default();
    if level_str.eq_ignore_ascii_case("off") {
        return None;
    }

    let log_dir: PathBuf = match std::env::var("SPACE_LOG") {
        Ok(v) if v.eq_ignore_ascii_case("off") => return None,
        Ok(v) => PathBuf::from(v),
        Err(_) => dirs::data_local_dir()?.join("space"),
    };
    std::fs::create_dir_all(&log_dir).ok()?;

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("space.log")
        .max_log_files(3)
        .build(&log_dir)
        .ok()?;

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let max_level = if level_str.eq_ignore_ascii_case("debug") {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    // Use try_init so a second call (e.g. in tests or if the MCP subscriber is
    // already set) fails gracefully instead of panicking.
    if fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(true)
        .with_timer(fmt::time::SystemTime)
        .with_max_level(max_level)
        .try_init()
        .is_err()
    {
        return None;
    }

    Some(guard)
}
