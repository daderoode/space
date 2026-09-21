//! Keeps every test process off the invoking user's git configuration
//! (ticket 43). A global `remote.pushDefault` flipped a push-routing test,
//! a failing global hook broke every fixture commit, and an Apple git
//! system file's `init.defaultBranch = main` broke a fixture guard wherever
//! the user's global file does not override it. CI's clean home hid all of it.
//!
//! Compiled into every integration test binary through `mod common` (every
//! `tests/*.rs` declares it, and `every_test_binary_declares_mod_common`
//! checks that), and into the lib and bin unit-test binaries through a
//! `#[cfg(test)]` include in `src/lib.rs` and `src/main.rs`.
#![allow(clippy::disallowed_methods)] // the guards start git and this binary directly (ADR 0002)

use git2::opts::{get_search_path, set_search_path};
use git2::ConfigLevel;
use std::process::Command;

/// libgit2's config levels that live outside the repository.
const LEVELS: [ConfigLevel; 3] = [ConfigLevel::System, ConfigLevel::Global, ConfigLevel::XDG];

/// Variables that would still lead git away from the fixture's own
/// repository and config once HOME has moved:
///
/// - `XDG_CONFIG_HOME`: the XDG config, ignore and attributes files, for git
///   and libgit2.
/// - What `git rev-parse --local-env-vars` lists (git 2.50), the variables
///   git itself clears when it moves to another repository: `GIT_CONFIG`
///   (the file plain `git config` reads and writes), `GIT_CONFIG_PARAMETERS`
///   and `GIT_CONFIG_COUNT` (values set with `-c`, and their environment
///   form), and those naming a repository, its index or its objects. A git
///   hook exports `GIT_DIR` and `GIT_INDEX_FILE`, so `cargo test` run from
///   one would otherwise send fixture git into the repository being
///   committed to.
/// - The environment forms of `init.templateDir`,
///   `init.defaultObjectFormat` and `init.defaultRefFormat`, which decide
///   what a fixture's `git init` makes (hooks, or a repository libgit2
///   cannot open).
///
/// `GIT_CONFIG_SYSTEM` stays: `GIT_CONFIG_NOSYSTEM` skips the system level
/// whatever it names.
const REMOVED: [&str; 19] = [
    "XDG_CONFIG_HOME",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_TEMPLATE_DIR",
    "GIT_DEFAULT_HASH",
    "GIT_DEFAULT_REF_FORMAT",
];

/// Points git and libgit2 away from every config file outside the
/// repository, for this process and every process it starts.
///
/// - `HOME=/dev/null`: git and libgit2 find no `~/.gitconfig`, no
///   `~/.config/git/config`, and no global ignore or attributes file (git
///   reads `~/.config/git/ignore` whatever `GIT_CONFIG_GLOBAL` says), and a
///   spawned `space` binary's libgit2 finds no global file either. Nothing
///   can be created under it, so no run leaves anything behind for the
///   next; git treats the `ENOTDIR` it gets as a missing file and prints
///   nothing.
/// - `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`: git children
///   read no global or system file even when the invoking environment
///   names one, and skip Apple git's own system file.
/// - libgit2 reads none of those variables for a repository opened with
///   `Repository::open`, and guesses its search path from HOME once, when
///   it first initialises. Setting each level's path to nothing reaches it
///   whenever that happened, and reaches `/etc/gitconfig`, which no
///   variable does.
///
/// Not reached: a spawned `space` binary's own libgit2 still reads
/// `/etc/gitconfig`, a path libgit2 hard-codes (root-owned, absent on the
/// machines this runs on).
extern "C" fn isolate() {
    // SAFETY: this runs before `main`, from the binary's initializer list,
    // so no other thread exists to read the environment or libgit2's search
    // path while they change.
    unsafe {
        std::env::set_var("HOME", "/dev/null");
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        for var in REMOVED {
            std::env::remove_var(var);
        }
        for level in LEVELS {
            if let Err(e) = set_search_path(level, "") {
                // A panic cannot unwind out of an initializer. Say why, then
                // stop: running on the user's config is what this prevents.
                eprintln!(
                    "tests/common/git_isolation.rs: libgit2 refused to clear its {:?} search path: {}",
                    level, e
                );
                std::process::abort();
            }
        }
    }
}

/// `isolate`'s entry in the binary's initializer list, which the loader runs
/// before `main`: the technique of the `ctor` crate, without the dependency.
/// `#[used]` keeps the entry through optimisation, and the linkers keep the
/// section itself.
#[used]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__mod_init_func")]
#[cfg_attr(target_os = "linux", link_section = ".init_array")]
static ISOLATE: extern "C" fn() = isolate;

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
compile_error!("tests/common/git_isolation.rs registers its initializer on Apple and Linux only");

/// Names the variables `a_hostile_invoking_environment_is_cleared_before_main`
/// sets in the child it starts, so the child's guard checks each one is gone
/// against a list that is not `REMOVED`.
const HOSTILE_VARS: &str = "SPACE_TEST_HOSTILE_VARS";

/// The initializer ran in this binary: the environment is as `isolate`
/// left it, libgit2 searches no directory outside the repository, and
/// neither git nor libgit2 reads a `remote.pushDefault` in a fresh
/// repository. Fails if the linker ever drops the initializer entry.
/// `a_hostile_invoking_environment_is_cleared_before_main` runs it again in a
/// child whose environment sets that key through every channel.
#[test]
fn git_config_outside_the_repository_is_out_of_reach() {
    assert_eq!(std::env::var("HOME").as_deref(), Ok("/dev/null"));
    assert_eq!(
        std::env::var("GIT_CONFIG_GLOBAL").as_deref(),
        Ok("/dev/null")
    );
    assert_eq!(std::env::var("GIT_CONFIG_NOSYSTEM").as_deref(), Ok("1"));
    let hostile = std::env::var(HOSTILE_VARS).unwrap_or_default();
    for var in hostile.split(',').filter(|v| !v.is_empty()) {
        assert_eq!(std::env::var_os(var), None, "{} is still set", var);
    }
    for level in [ConfigLevel::System, ConfigLevel::Global, ConfigLevel::XDG] {
        // SAFETY: nothing sets a search path after `isolate`, so this read
        // races no write.
        let path = unsafe { get_search_path(level) }.unwrap();
        assert!(
            path.as_bytes().is_empty(),
            "libgit2 still searches {:?} for {:?}",
            path,
            level
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let init = git(&["init", "-q", "-b", "main"]);
    assert!(
        init.status.success(),
        "git init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let get = git(&["config", "--get", "remote.pushDefault"]);
    assert_eq!(
        get.status.code(),
        Some(1),
        "git reads remote.pushDefault {:?} {}",
        String::from_utf8_lossy(&get.stdout),
        String::from_utf8_lossy(&get.stderr)
    );
    let config = git2::Repository::open(dir.path())
        .expect("libgit2 opens the repository git just made")
        .config()
        .unwrap();
    assert_eq!(
        config
            .get_string("remote.pushDefault")
            .map_err(|e| e.code()),
        Err(git2::ErrorCode::NotFound),
        "libgit2 reads remote.pushDefault"
    );
}

/// Starts this binary again, running only the guard above, with an
/// environment that is hostile through every channel: a home, an XDG
/// directory, a global file, a system file, `-c` parameters and the
/// `GIT_CONFIG_COUNT` form each set `remote.pushDefault`; every variable git
/// clears when it moves to another repository (`git rev-parse
/// --local-env-vars`) points somewhere else, as the ones a git hook exports
/// would; and the variables that decide what `git init` makes are set.
/// `GIT_CONFIG_NOSYSTEM` is removed so the child must set it itself. The
/// child's initializer runs before its `main`, so its guard sees only what
/// that left. The list here is the check's own, not `REMOVED`.
#[test]
fn a_hostile_invoking_environment_is_cleared_before_main() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let key = |value: &str| format!("[remote]\n\tpushDefault = {}\n", value);
    std::fs::create_dir_all(home.join("xdg/git")).unwrap();
    std::fs::create_dir_all(home.join(".config/git")).unwrap();
    std::fs::write(home.join(".gitconfig"), key("hostile-home")).unwrap();
    std::fs::write(home.join(".config/git/config"), key("hostile-home-xdg")).unwrap();
    std::fs::write(home.join("xdg/git/config"), key("hostile-xdg")).unwrap();
    std::fs::write(home.join("global"), key("hostile-global")).unwrap();
    std::fs::write(home.join("system"), key("hostile-system")).unwrap();
    let xdg = home.join("xdg").display().to_string();
    let elsewhere = home.join("elsewhere").display().to_string();

    let cleared: [(&str, &str); 19] = [
        ("XDG_CONFIG_HOME", &xdg),
        ("GIT_CONFIG", &elsewhere),
        (
            "GIT_CONFIG_PARAMETERS",
            "'remote.pushdefault'='hostile-parameters'",
        ),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &elsewhere),
        ("GIT_OBJECT_DIRECTORY", &elsewhere),
        ("GIT_DIR", &elsewhere),
        ("GIT_WORK_TREE", &elsewhere),
        ("GIT_IMPLICIT_WORK_TREE", "0"),
        ("GIT_GRAFT_FILE", &elsewhere),
        ("GIT_INDEX_FILE", &elsewhere),
        ("GIT_NO_REPLACE_OBJECTS", "1"),
        ("GIT_REPLACE_REF_BASE", "refs/hostile/"),
        ("GIT_PREFIX", "hostile/"),
        ("GIT_SHALLOW_FILE", &elsewhere),
        ("GIT_COMMON_DIR", &elsewhere),
        ("GIT_TEMPLATE_DIR", &elsewhere),
        ("GIT_DEFAULT_HASH", "sha256"),
        ("GIT_DEFAULT_REF_FORMAT", "reftable"),
    ];
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .arg("git_config_outside_the_repository_is_out_of_reach")
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", home.join("global"))
        .env("GIT_CONFIG_SYSTEM", home.join("system"))
        .env("GIT_CONFIG_KEY_0", "remote.pushDefault")
        .env("GIT_CONFIG_VALUE_0", "hostile-count")
        .env_remove("GIT_CONFIG_NOSYSTEM")
        .env(
            HOSTILE_VARS,
            cleared
                .iter()
                .map(|(v, _)| *v)
                .collect::<Vec<_>>()
                .join(","),
        );
    for (var, value) in cleared {
        child.env(var, value);
    }
    let out = child.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("test result: ok. 1 passed"),
        "the guard must run once and pass in a child with a hostile environment:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Isolation reaches an integration test binary only through `mod common`,
/// so every `tests/*.rs` declares it, the two that run no git included. A new
/// file without it would run on the invoking user's config with nothing to
/// say so.
#[test]
fn every_test_binary_declares_mod_common() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut seen = Vec::new();
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            if !text.lines().any(|l| l.trim() == "mod common;") {
                missing.push(path.display().to_string());
            }
            seen.push(path);
        }
    }
    assert!(!seen.is_empty(), "no test files under {}", dir.display());
    assert!(
        missing.is_empty(),
        "these test binaries do not declare `mod common;`, so git_isolation never runs in them: {:?}",
        missing
    );
}
