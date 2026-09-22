#![allow(clippy::disallowed_methods)] // fixtures start git directly (ADR 0002)

mod common;

use assert_cmd::Command;
use common::TestEnv;
use predicates::prelude::*;
use space::core::workspace::{create_worktree, BranchStrategy};
/// Build a `space` Command wired to the test environment.
fn space(env: &TestEnv) -> Command {
    let mut cmd = Command::cargo_bin("space").unwrap();
    cmd.env("SPACE_CONFIG_DIR", &env.config_dir);
    // Disable colour codes so assertions match plain text.
    cmd.env("NO_COLOR", "1");
    cmd
}

// ---------------------------------------------------------------------------
// 1. ls – no workspaces
// ---------------------------------------------------------------------------
#[test]
fn ls_no_workspaces_succeeds() {
    let env = TestEnv::new();
    space(&env)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("No workspaces"));
}

// ---------------------------------------------------------------------------
// 2. ls – shows workspace names
// ---------------------------------------------------------------------------
#[test]
fn ls_shows_workspace_names() {
    let env = TestEnv::new();

    // Create a workspace dir with a repo subdir containing a .git marker
    let ws_path = env.workspaces_dir.join("my-feature");
    let repo_in_ws = ws_path.join("some-repo");
    std::fs::create_dir_all(&repo_in_ws).unwrap();
    // A .git *file* (not directory) is enough: list.rs checks .join(".git").exists()
    std::fs::write(repo_in_ws.join(".git"), "gitdir: /dev/null").unwrap();

    space(&env)
        .arg("ls")
        .assert()
        .success()
        .stdout(predicate::str::contains("my-feature"));
}

// ---------------------------------------------------------------------------
// 3. ls -v – shows branch info
// ---------------------------------------------------------------------------
#[test]
fn ls_verbose_shows_branch_info() {
    let env = TestEnv::new();

    let repo_path = env.create_repo("alpha");
    let ws_name = "verbose-ws";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("feat-x".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["ls", "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains(ws_name).and(predicate::str::contains("feat-x")));
}

// ---------------------------------------------------------------------------
// 4. status – existing workspace
// ---------------------------------------------------------------------------
#[test]
fn status_existing_workspace() {
    let env = TestEnv::new();

    let repo_path = env.create_repo("beta");
    let ws_name = "status-ws";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("feat-y".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["status", ws_name])
        .assert()
        .success()
        .stdout(predicate::str::contains("beta").and(predicate::str::contains("feat-y")));
}

// ---------------------------------------------------------------------------
// 5. status – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn status_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["status", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 6. repos – lists from cache
// ---------------------------------------------------------------------------
#[test]
fn repos_lists_from_cache() {
    let env = TestEnv::new();
    // Write cache pointing to a path that does NOT exist on disk.
    // The only way the name appears in output is via the cache.
    let fake_path = std::path::PathBuf::from("/nonexistent/repos/phantom-repo");
    env.write_cache(&[fake_path]);

    space(&env)
        .arg("repos")
        .assert()
        .success()
        .stdout(predicate::str::contains("phantom-repo"));
}

// ---------------------------------------------------------------------------
// 7. repos --refresh – rescans
// ---------------------------------------------------------------------------
#[test]
fn repos_refresh_rescans() {
    let env = TestEnv::new();
    env.create_repo("gamma");

    // No cache file exists initially
    let cache_path = env.config_dir.join("repos.cache");
    assert!(
        !cache_path.exists(),
        "cache should not exist before refresh"
    );

    space(&env)
        .args(["repos", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gamma"));

    // Cache file should now exist and contain the repo
    assert!(
        cache_path.exists(),
        "cache should be written after --refresh"
    );
    let content = std::fs::read_to_string(&cache_path).unwrap();
    assert!(
        content.contains("gamma"),
        "cache should contain scanned repo"
    );
}

// ---------------------------------------------------------------------------
// 8. go – existing workspace emits cd marker
// ---------------------------------------------------------------------------
#[test]
fn go_existing_emits_cd() {
    let env = TestEnv::new();
    let ws_path = env.workspaces_dir.join("jump-ws");
    std::fs::create_dir_all(&ws_path).unwrap();

    space(&env)
        .args(["go", "jump-ws"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "__SPACE_CD__:{}",
            ws_path.display()
        )));
}

// ---------------------------------------------------------------------------
// 9. go – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn go_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["go", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 10. rm --force – removes workspace
// ---------------------------------------------------------------------------
#[test]
fn rm_force_removes_workspace() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("ephemeral");
    let ws_name = "doomed";

    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        ws_name,
        &BranchStrategy::NewBranch("temp-branch".to_string()),
    )
    .unwrap();

    let ws_path = env.workspaces_dir.join(ws_name);
    assert!(ws_path.exists());

    space(&env)
        .args(["rm", "--force", ws_name])
        .assert()
        .success();

    assert!(!ws_path.exists(), "workspace dir should be deleted");

    // Verify worktree is unlinked from the main repo
    let output = std::process::Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let wt_list = String::from_utf8_lossy(&output.stdout);
    assert!(
        !wt_list.contains(ws_name),
        "worktree should be unlinked after rm --force"
    );
}

// ---------------------------------------------------------------------------
// 11. rm --force – nonexistent workspace errors
// ---------------------------------------------------------------------------
#[test]
fn rm_force_nonexistent_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["rm", "--force", "ghost"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ---------------------------------------------------------------------------
// 12. go – writes cd target to file when __SPACE_CD_FILE__ is set
// ---------------------------------------------------------------------------
#[test]
fn go_writes_cd_file_when_env_set() {
    let env = TestEnv::new();
    let ws_path = env.workspaces_dir.join("cd-ws");
    std::fs::create_dir_all(&ws_path).unwrap();

    let cd_file = env.dir.path().join("cd_target");

    space(&env)
        .args(["go", "cd-ws"])
        .env("__SPACE_CD_FILE__", &cd_file)
        .assert()
        .success();

    assert!(cd_file.exists(), "cd file should be written");
    let content = std::fs::read_to_string(&cd_file).unwrap();
    assert_eq!(
        content,
        ws_path.display().to_string(),
        "cd file should contain workspace path"
    );
}

// ---------------------------------------------------------------------------
// __complete workspaces -- lists workspace names with context
// ---------------------------------------------------------------------------
#[test]
fn complete_workspaces_lists_names() {
    let env = TestEnv::new();
    let repo_path = env.create_repo("alpha");
    create_worktree(
        &repo_path,
        &env.workspaces_dir,
        "my-feature",
        &BranchStrategy::NewBranch("feat-x".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["__complete", "workspaces"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-feature"))
        .stdout(predicate::str::contains("feat-x"));
}

// ---------------------------------------------------------------------------
// __complete workspaces -- empty when none exist
// ---------------------------------------------------------------------------
#[test]
fn complete_workspaces_empty() {
    let env = TestEnv::new();
    space(&env)
        .args(["__complete", "workspaces"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

// ---------------------------------------------------------------------------
// __complete repos -- lists from cache
// ---------------------------------------------------------------------------
#[test]
fn complete_repos_lists_cached() {
    let env = TestEnv::new();
    let repo_path = env.repos_dir.join("my-service");
    std::fs::create_dir_all(&repo_path).unwrap();
    env.write_cache(&[repo_path.clone()]);

    space(&env)
        .args(["__complete", "repos"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-service"));
}

// ---------------------------------------------------------------------------
// __complete available-repos -- filters out existing repos
// ---------------------------------------------------------------------------
#[test]
fn complete_available_repos_filters_existing() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let beta = env.create_repo("beta");
    env.write_cache(&[alpha.clone(), beta.clone()]);

    create_worktree(
        &alpha,
        &env.workspaces_dir,
        "test-ws",
        &BranchStrategy::NewBranch("feat".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["__complete", "available-repos", "test-ws"])
        .assert()
        .success()
        .stdout(predicate::str::contains("beta"))
        .stdout(predicate::str::contains("alpha").not());
}

// ---------------------------------------------------------------------------
// init zsh -- outputs wrapper function and completions
// ---------------------------------------------------------------------------
#[test]
fn init_zsh_outputs_wrapper_and_completions() {
    let env = TestEnv::new();
    let output = space(&env).args(["init", "zsh"]).output().unwrap();
    assert!(output.status.success(), "init zsh should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Contains shell wrapper
    assert!(
        stdout.contains("__SPACE_CD_FILE__"),
        "init output should contain the shell wrapper"
    );
    // Contains completion function
    assert!(
        stdout.contains("compdef _space space"),
        "init output should contain the completion registration"
    );
}

// ---------------------------------------------------------------------------
// init -- unsupported shell returns error
// ---------------------------------------------------------------------------
#[test]
fn init_unsupported_shell_errors() {
    let env = TestEnv::new();
    space(&env)
        .args(["init", "fish"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported"));
}

// ---------------------------------------------------------------------------
// 18. rm --force – ticket 27: a refused worktree removal is reported, and the
//     CLI's own stdout stays its own
// ---------------------------------------------------------------------------
#[test]
fn rm_force_reports_a_locked_worktree() {
    let env = TestEnv::new();
    let repo = env.create_repo("alpha");
    create_worktree(
        &repo,
        &env.workspaces_dir,
        "locked-ws",
        &BranchStrategy::NewBranch("locked-ws".to_string()),
    )
    .unwrap();
    let wt = env.workspaces_dir.join("locked-ws").join("alpha");
    let out = std::process::Command::new("git")
        .args(["worktree", "lock", "--reason", "on usb"])
        .arg(&wt)
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "fixture: lock the worktree");

    space(&env)
        .args(["rm", "--force", "locked-ws"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("alpha"))
        .stderr(predicate::str::contains("locked working tree"))
        .stderr(predicate::str::contains("git worktree unlock"))
        .stdout(predicate::str::contains("Removed workspace").not());

    assert!(
        wt.join(".git").exists(),
        "the space must survive a refused removal"
    );
}

/// git inherits the CLI's stdout too, so a git that prints (a wrapper on PATH,
/// or `GIT_TRACE` pointed at stdout) used to mix its lines into what `space rm`
/// wrote. Nothing parses the CLI's stdout, but the same inherited handle is the
/// JSON-RPC stream under MCP; this is the cheap half of that guard.
#[test]
fn rm_force_prints_only_its_own_line() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();
    let repo = env.create_repo("alpha");
    create_worktree(
        &repo,
        &env.workspaces_dir,
        "quiet",
        &BranchStrategy::NewBranch("quiet".to_string()),
    )
    .unwrap();

    let bin = env.dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let marker = env.dir.path().join("shim-ran.txt");
    let real_path = std::env::var("PATH").unwrap();
    std::fs::write(
        bin.join("git"),
        format!(
            "#!/bin/sh\necho \"shim: git $*\"\necho \"$*\" >> '{}'\nPATH='{real_path}'\nexec git \"$@\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();

    space(&env)
        .env("PATH", format!("{}:{}", bin.display(), real_path))
        .args(["rm", "--force", "quiet"])
        .assert()
        .success()
        .stdout(predicate::eq("Removed workspace 'quiet'\n"));

    // Without this the test passes when nothing printed because nothing ran:
    // stdout would be that one line whether or not git was ever started.
    let ran = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        ran.contains("worktree remove"),
        "the printing git must be the one that removed the worktree, got {ran:?}"
    );
}

// ---------------------------------------------------------------------------
// create / add -- repo names on the command line (ticket 23)
//
// Each name must equal one cached repo's directory name, as on disk and
// case-sensitive. A name that matches nothing, several repos, or a repo the
// workspace already holds stops the command before the TUI is started, so
// these run through the real binary on a pipe and never need a terminal.
// ---------------------------------------------------------------------------

/// Cache `names` as bare directories under `repos_dir`, returning their paths.
fn cache_dirs(env: &TestEnv, names: &[&str]) -> Vec<std::path::PathBuf> {
    let paths: Vec<std::path::PathBuf> = names
        .iter()
        .map(|n| {
            let p = env.repos_dir.join(n);
            std::fs::create_dir_all(&p).unwrap();
            p
        })
        .collect();
    env.write_cache(&paths);
    paths
}

#[test]
fn create_unknown_name_refuses_before_the_tui() {
    let env = TestEnv::new();
    cache_dirs(&env, &["api", "web"]);

    space(&env)
        .args(["create", "api", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no repo named 'nope' in the repo list",
        ))
        .stderr(predicate::str::contains("space repos --refresh"))
        .stdout(predicate::str::contains("__SPACE_CD__").not());
}

#[test]
fn create_scope_argument_is_refused_as_not_a_name() {
    let env = TestEnv::new();
    cache_dirs(&env, &["api"]);

    space(&env)
        .args(["create", "repos/"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("'repos/' is not a repo name;"))
        .stderr(predicate::str::contains("rescan").not());
}

#[test]
fn create_case_mismatch_refuses_and_names_the_exact_one() {
    let env = TestEnv::new();
    cache_dirs(&env, &["api"]);

    space(&env)
        .args(["create", "Api"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no repo named 'Api'"))
        .stderr(predicate::str::contains("case-sensitive"))
        .stderr(predicate::str::contains("did you mean 'api'?"));
}

#[test]
fn create_ambiguous_name_lists_both_paths() {
    let env = TestEnv::new();
    let paths = cache_dirs(&env, &["a/api", "b/api"]);

    space(&env)
        .args(["create", "api"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("'api' names 2 repos:"))
        .stderr(predicate::str::contains(paths[0].display().to_string()))
        .stderr(predicate::str::contains(paths[1].display().to_string()))
        .stderr(predicate::str::contains("pick it in the picker instead"));
}

#[test]
fn add_name_already_in_the_workspace_says_so() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    let beta = env.repos_dir.join("beta");
    std::fs::create_dir_all(&beta).unwrap();
    env.write_cache(&[alpha.clone(), beta]);
    create_worktree(
        &alpha,
        &env.workspaces_dir,
        "ws",
        &BranchStrategy::NewBranch("ws".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["add", "ws", "alpha"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "repo 'alpha' is already in workspace 'ws'",
        ))
        .stderr(predicate::str::contains("no repo named").not());
}

#[test]
fn add_unknown_name_refuses_before_the_tui() {
    let env = TestEnv::new();
    let alpha = env.create_repo("alpha");
    env.write_cache(std::slice::from_ref(&alpha));
    create_worktree(
        &alpha,
        &env.workspaces_dir,
        "ws",
        &BranchStrategy::NewBranch("ws".to_string()),
    )
    .unwrap();

    space(&env)
        .args(["add", "ws", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no repo named 'nope' in the repo list",
        ))
        .stdout(predicate::str::contains("__SPACE_CD__").not());
}

/// U+2014 as UTF-8. Spelled as bytes so this file does not hold it.
const EM_DASH: [u8; 3] = [0xE2, 0x80, 0x94];

/// Other spellings that a tracked file type here turns into U+2014, in lower
/// case and split so this file does not hold them: the four-digit escape of
/// JSON, TOML and zsh, and the three HTML entities GitHub renders in Markdown.
const EM_DASH_SPELLINGS: [&str; 4] = [
    concat!("\\", "u2014"),
    concat!("&", "mdash;"),
    concat!("&", "#8212;"),
    concat!("&", "#x2014;"),
];

/// Whether `line` holds U+2014: raw, as one of `EM_DASH_SPELLINGS` in any
/// case, or as a Rust `\u{...}` escape of it (leading zeros or underscores).
fn holds_em_dash(line: &[u8]) -> bool {
    let lower = line.to_ascii_lowercase();
    let holds = |needle: &[u8]| lower.windows(needle.len()).any(|w| w == needle);
    if holds(&EM_DASH) || EM_DASH_SPELLINGS.iter().any(|s| holds(s.as_bytes())) {
        return true;
    }
    line.windows(3)
        .enumerate()
        .filter(|(_, w)| *w == b"\\u{")
        .any(|(i, _)| {
            let rest = &line[i + 3..];
            let len = rest
                .iter()
                .take_while(|b| b.is_ascii_hexdigit() || **b == b'_')
                .count();
            let hex: String = rest[..len]
                .iter()
                .filter(|b| **b != b'_')
                .map(|b| *b as char)
                .collect();
            rest.get(len) == Some(&b'}') && u32::from_str_radix(&hex, 16) == Ok(0x2014)
        })
}

/// House style: no tracked file holds an em dash, in its name or its text
/// (ticket 17). Reads every file `git ls-files` lists, from the working tree,
/// and names each `path:line`.
#[test]
fn no_tracked_file_holds_an_em_dash() {
    use std::os::unix::ffi::OsStrExt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("cannot run git ls-files in {}: {e}", root.display()));
    assert!(
        out.status.success(),
        "git ls-files failed in {}: {}",
        root.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut files: Vec<&[u8]> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect();
    // A conflicted path is listed once per stage, next to each other.
    files.dedup();
    assert!(
        files.iter().any(|f| *f == b"tests/cli_test.rs"),
        "git ls-files in {} did not list this test's own file",
        root.display()
    );
    let mut hits = Vec::new();
    for file in files {
        let name = String::from_utf8_lossy(file);
        if holds_em_dash(file) {
            hits.push(format!("{name}: (the file name)"));
        }
        // The name as git wrote it, bytes and all.
        let path = root.join(std::ffi::OsStr::from_bytes(file));
        match std::fs::symlink_metadata(&path) {
            // Deleted in the working tree, not yet staged.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => panic!("cannot stat {name}: {e}"),
            // Git keeps a symlink as the text of its target, dangling or not.
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = std::fs::read_link(&path)
                    .unwrap_or_else(|e| panic!("cannot read the link {name}: {e}"));
                if holds_em_dash(target.as_os_str().as_bytes()) {
                    hits.push(format!("{name}: (the symlink's target)"));
                }
                continue;
            }
            // A directory where a file was: no text of its own to read.
            Ok(meta) if !meta.is_file() => continue,
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {name}: {e}"));
        // Git's own test for a binary file: an image can hold the three bytes
        // by chance.
        if bytes[..bytes.len().min(8000)].contains(&0) {
            continue;
        }
        for (n, line) in bytes.split(|b| *b == b'\n').enumerate() {
            if holds_em_dash(line) {
                hits.push(format!(
                    "{name}:{}: {}",
                    n + 1,
                    String::from_utf8_lossy(line).trim()
                ));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} tracked line(s) hold U+2014 (em dash); use a comma, colon, semicolon or parentheses:\n{}",
        hits.len(),
        hits.join("\n")
    );
}
