//! Keeps every test process off the invoking user's git configuration
//! (ticket 43). A global `remote.pushDefault` flipped a push-routing test,
//! a failing global hook broke every fixture commit, and an Apple git
//! system file's `init.defaultBranch = main` broke a fixture guard wherever
//! the user's global file does not override it. CI's clean home hid all of it.
//!
//! Compiled into every integration test binary through `mod common`, and
//! into the lib and bin unit-test binaries through a `#[cfg(test)]` include
//! in `src/lib.rs` and `src/main.rs`. `config_test` and `repo_test` do not
//! include `common`; they run no git. A new test binary that runs git
//! includes `common` like the rest.

use git2::opts::{get_search_path, set_search_path};
use git2::ConfigLevel;

/// libgit2's config levels that live outside the repository.
const LEVELS: [ConfigLevel; 3] = [ConfigLevel::System, ConfigLevel::Global, ConfigLevel::XDG];

/// Variables that would still lead git to config outside the repository
/// once HOME has moved: `XDG_CONFIG_HOME` (the XDG config, ignore and
/// attributes files, for git and libgit2), `GIT_CONFIG` (the file plain `git
/// config` reads and writes), and `GIT_CONFIG_PARAMETERS` and
/// `GIT_CONFIG_COUNT` (values set with `-c`, and their environment form).
/// `GIT_CONFIG_SYSTEM` stays: `GIT_CONFIG_NOSYSTEM` skips the system level
/// whatever it names.
const REMOVED: [&str; 4] = [
    "XDG_CONFIG_HOME",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
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
            set_search_path(level, "").expect("libgit2 takes an empty search path");
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

/// The initializer ran in this binary: the environment is as `isolate`
/// left it and libgit2 searches no directory outside the repository. Fails
/// if the linker ever drops the initializer entry.
#[test]
fn git_config_outside_the_repository_is_out_of_reach() {
    assert_eq!(std::env::var("HOME").as_deref(), Ok("/dev/null"));
    assert_eq!(
        std::env::var("GIT_CONFIG_GLOBAL").as_deref(),
        Ok("/dev/null")
    );
    assert_eq!(std::env::var("GIT_CONFIG_NOSYSTEM").as_deref(), Ok("1"));
    for var in REMOVED {
        assert_eq!(std::env::var_os(var), None, "{} is still set", var);
    }
    for level in LEVELS {
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
}
