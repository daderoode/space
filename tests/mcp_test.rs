mod common;

use common::TestEnv;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::ServerHandler;
use space::core::workspace::{create_worktree, BranchStrategy};
use space::mcp::{
    AddReposParams, CreateWorkspaceParams, ListReposParams, RemoveWorkspaceParams, SpaceServer,
    WorkspaceStatusParams,
};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// Process-global lock for tests that set SPACE_CONFIG_DIR.
/// `set_var`/`remove_var` are process-wide, so handler tests must not run concurrently.
static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Drop guard that removes SPACE_CONFIG_DIR even if the test panics.
struct EnvGuard;
impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe { std::env::remove_var("SPACE_CONFIG_DIR") };
    }
}

/// Run `f` with SPACE_CONFIG_DIR pointing at a fresh TestEnv.
/// Serialised via ENV_LOCK so parallel test threads don't collide.
fn with_test_env<F: FnOnce(&TestEnv, &SpaceServer)>(f: F) {
    let _guard = ENV_LOCK.lock().unwrap();
    let env = TestEnv::new();
    unsafe { std::env::set_var("SPACE_CONFIG_DIR", &env.config_dir) };
    let _env_guard = EnvGuard;
    let server = SpaceServer::new();
    f(&env, &server);
}

/// Extract the JSON text from a successful CallToolResult.
fn result_text(result: &rmcp::model::CallToolResult) -> String {
    result.content[0]
        .raw
        .as_text()
        .expect("expected text content")
        .text
        .clone()
}

#[test]
fn server_reports_tool_capability() {
    let server = SpaceServer::new();
    let info = server.get_info();
    let instructions = info.instructions.unwrap_or_default();
    assert!(
        info.capabilities.tools.is_some(),
        "tools capability should be enabled"
    );
    assert!(
        instructions.contains("list_workspaces"),
        "instructions should mention list_workspaces"
    );
    assert!(
        instructions.contains("create_workspace"),
        "instructions should mention create_workspace"
    );
    assert!(
        instructions.contains("remove_workspace"),
        "instructions should mention remove_workspace"
    );
}

#[test]
fn resolve_repos_finds_exact_match() {
    let cache = vec![
        PathBuf::from("/repos/alpha"),
        PathBuf::from("/repos/beta"),
        PathBuf::from("/repos/gamma"),
    ];
    let result = space::mcp::resolve_repos(&["beta".to_string()], &cache);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![PathBuf::from("/repos/beta")]);
}

#[test]
fn resolve_repos_case_insensitive() {
    let cache = vec![PathBuf::from("/repos/MyRepo")];
    let result = space::mcp::resolve_repos(&["myrepo".to_string()], &cache);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vec![PathBuf::from("/repos/MyRepo")]);
}

#[test]
fn resolve_repos_errors_on_unknown() {
    let cache = vec![PathBuf::from("/repos/alpha")];
    let result = space::mcp::resolve_repos(&["nonexistent".to_string()], &cache);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn resolve_repos_errors_on_ambiguous() {
    let cache = vec![
        PathBuf::from("/work/alpha"),
        PathBuf::from("/personal/alpha"),
    ];
    let result = space::mcp::resolve_repos(&["alpha".to_string()], &cache);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("ambiguous"));
}

#[test]
fn build_strategy_new_defaults_to_workspace_name() {
    let result = space::mcp::build_strategy("new", None, "my-feature");
    assert!(result.is_ok());
    match result.unwrap() {
        BranchStrategy::NewBranch(name) => assert_eq!(name, "my-feature"),
        other => panic!("expected NewBranch, got {:?}", other),
    }
}

#[test]
fn build_strategy_new_with_explicit_branch() {
    let result = space::mcp::build_strategy("new", Some("custom-branch"), "ws-name");
    assert!(result.is_ok());
    match result.unwrap() {
        BranchStrategy::NewBranch(name) => assert_eq!(name, "custom-branch"),
        other => panic!("expected NewBranch, got {:?}", other),
    }
}

#[test]
fn build_strategy_existing_requires_branch() {
    let result = space::mcp::build_strategy("existing", None, "ws-name");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("required"));
}

#[test]
fn build_strategy_existing_with_branch() {
    let result = space::mcp::build_strategy("existing", Some("main"), "ws-name");
    assert!(result.is_ok());
    match result.unwrap() {
        BranchStrategy::ExistingBranch(name) => assert_eq!(name, "main"),
        other => panic!("expected ExistingBranch, got {:?}", other),
    }
}

#[test]
fn build_strategy_detached() {
    let result = space::mcp::build_strategy("detached", None, "ws-name");
    assert!(result.is_ok());
    match result.unwrap() {
        BranchStrategy::DetachedHead => {}
        other => panic!("expected DetachedHead, got {:?}", other),
    }
}

#[test]
fn build_strategy_unknown_errors() {
    let result = space::mcp::build_strategy("invalid", None, "ws-name");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown strategy"));
}

// ---------------------------------------------------------------------------
// MCP handler integration tests
// ---------------------------------------------------------------------------

#[test]
fn list_workspaces_empty() {
    with_test_env(|_env, server| {
        let result = server.list_workspaces().unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, serde_json::json!([]));
    });
}

#[test]
fn list_workspaces_with_data() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("alpha");
        create_worktree(
            &repo_path,
            &env.workspaces_dir,
            "feat-ws",
            &BranchStrategy::NewBranch("feat-ws".to_string()),
        )
        .unwrap();

        let result = server.list_workspaces().unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let arr = parsed.as_array().expect("should be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "feat-ws");
        let repos = arr[0]["repos"].as_array().expect("repos should be array");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0]["name"], "alpha");
        assert_eq!(repos[0]["branch"], "feat-ws");
    });
}

#[test]
fn workspace_status_exists() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("beta");
        create_worktree(
            &repo_path,
            &env.workspaces_dir,
            "status-ws",
            &BranchStrategy::NewBranch("status-ws".to_string()),
        )
        .unwrap();

        let result = server
            .workspace_status(Parameters(WorkspaceStatusParams {
                name: "status-ws".to_string(),
            }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["name"], "status-ws");
        let repos = parsed["repos"].as_array().expect("repos should be array");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0]["name"], "beta");
        assert_eq!(repos[0]["branch"], "status-ws");
    });
}

#[test]
fn workspace_status_not_found() {
    with_test_env(|_env, server| {
        let result = server.workspace_status(Parameters(WorkspaceStatusParams {
            name: "ghost".to_string(),
        }));
        assert!(result.is_err(), "should fail for nonexistent workspace");
        let err = result.unwrap_err();
        let msg = err.message.to_string();
        assert!(
            msg.contains("not found"),
            "error should mention 'not found': {msg}"
        );
    });
}

#[test]
fn list_repos_cached() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("cached-repo");
        env.write_cache(&[repo_path]);

        let result = server
            .list_repos(Parameters(ListReposParams { refresh: false }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let arr = parsed.as_array().expect("should be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "cached-repo");
    });
}

#[test]
fn list_repos_refresh() {
    with_test_env(|env, server| {
        env.create_repo("scanned-repo");

        let result = server
            .list_repos(Parameters(ListReposParams { refresh: true }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let arr = parsed.as_array().expect("should be an array");
        assert!(
            arr.iter().any(|r| r["name"] == "scanned-repo"),
            "refresh scan should find scanned-repo"
        );
    });
}

#[test]
fn create_workspace_success() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);

        let result = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "new-ws".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["name"], "new-ws");
        let created = parsed["repos_created"].as_array().unwrap();
        assert_eq!(created, &[serde_json::json!("delta")]);
        assert!(env.workspaces_dir.join("new-ws").join("delta").exists());
    });
}

#[test]
fn create_workspace_unknown_repo() {
    with_test_env(|env, server| {
        // Write an empty cache so load_repo_cache doesn't try to scan
        env.write_cache(&[]);

        let result = server.create_workspace(Parameters(CreateWorkspaceParams {
            name: "bad-ws".to_string(),
            repos: vec!["ghost".to_string()],
            strategy: "new".to_string(),
            branch: None,
        }));
        assert!(result.is_err(), "should fail for unknown repo");
        let err = result.unwrap_err();
        let msg = err.message.to_string();
        assert!(
            msg.contains("not found"),
            "error should mention 'not found': {msg}"
        );
    });
}

#[test]
fn add_repos_success() {
    with_test_env(|env, server| {
        // Create initial workspace with one repo
        let repo_a = env.create_repo("repo-a");
        let repo_b = env.create_repo("repo-b");
        env.write_cache(&[repo_a.clone(), repo_b.clone()]);

        create_worktree(
            &repo_a,
            &env.workspaces_dir,
            "add-ws",
            &BranchStrategy::NewBranch("add-ws".to_string()),
        )
        .unwrap();

        let result = server
            .add_repos(Parameters(AddReposParams {
                workspace: "add-ws".to_string(),
                repos: vec!["repo-b".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["workspace"], "add-ws");
        let added = parsed["added"].as_array().unwrap();
        assert_eq!(added, &[serde_json::json!("repo-b")]);
        assert!(env.workspaces_dir.join("add-ws").join("repo-b").exists());
    });
}

#[test]
fn add_repos_nonexistent_ws() {
    with_test_env(|env, server| {
        env.write_cache(&[]);

        let result = server.add_repos(Parameters(AddReposParams {
            workspace: "ghost".to_string(),
            repos: vec!["anything".to_string()],
            strategy: "new".to_string(),
            branch: None,
        }));
        assert!(result.is_err(), "should fail for nonexistent workspace");
        let err = result.unwrap_err();
        let msg = err.message.to_string();
        assert!(
            msg.contains("not found"),
            "error should mention 'not found': {msg}"
        );
    });
}

#[test]
fn remove_workspace_success() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("ephemeral");
        create_worktree(
            &repo_path,
            &env.workspaces_dir,
            "doomed",
            &BranchStrategy::NewBranch("doomed".to_string()),
        )
        .unwrap();

        let ws_path = env.workspaces_dir.join("doomed");
        assert!(ws_path.exists(), "workspace should exist before removal");

        let result = server
            .remove_workspace(Parameters(RemoveWorkspaceParams {
                name: "doomed".to_string(),
            }))
            .unwrap();
        let text = result_text(&result);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["removed"], "doomed");
        assert!(!ws_path.exists(), "workspace dir should be deleted");
    });
}

// ---------------------------------------------------------------------------
// Ticket 13: hostile names over MCP. The client is another program, so every
// tool that takes a name must refuse one that is not a plain component with
// `invalid_params` before it touches the filesystem, and the assertions are
// about what is (still) on disk, not only about the error.
// ---------------------------------------------------------------------------

fn invalid_params(err: &rmcp::ErrorData) -> String {
    assert_eq!(
        err.code,
        rmcp::model::ErrorCode::INVALID_PARAMS,
        "a bad name is the caller's error, got {:?}: {}",
        err.code,
        err.message
    );
    err.message.to_string()
}

fn entries(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

#[test]
fn create_workspace_rejects_a_traversal_name() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);
        let before = entries(&env.workspaces_dir);

        let err = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "../escape".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .expect_err("a name containing '/' must be refused");
        let msg = invalid_params(&err);
        assert_eq!(
            msg,
            "invalid space name \"../escape\": Space name cannot contain '/' or '\\'"
        );
        assert!(
            !env.dir.path().join("escape").exists(),
            "nothing may be created outside ws_dir"
        );
        assert_eq!(
            entries(&env.workspaces_dir),
            before,
            "ws_dir gained no entry"
        );
    });
}

#[test]
fn create_workspace_rejects_an_absolute_name() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);
        let outside = env.dir.path().join("abs");
        let name = outside.to_string_lossy().to_string();

        let err = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name,
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .expect_err("an absolute name replaces ws_dir entirely and must be refused");
        let msg = invalid_params(&err);
        assert!(
            msg.contains("Space name cannot contain '/' or '\\'"),
            "got {msg}"
        );
        assert!(!outside.exists(), "the absolute path must not be created");
    });
}

#[test]
fn create_workspace_rejects_an_empty_name() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);

        let err = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: String::new(),
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .expect_err("an empty name is ws_dir itself and must be refused");
        let msg = invalid_params(&err);
        assert_eq!(msg, "invalid space name \"\": Space name cannot be empty");
        assert!(
            !env.workspaces_dir.join("delta").exists(),
            "the worktree must not land directly in ws_dir"
        );
    });
}

#[test]
fn create_workspace_rejects_a_dash_branch() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);

        for (strategy, branch) in [("new", Some("-x")), ("existing", Some("-x"))] {
            let err = server
                .create_workspace(Parameters(CreateWorkspaceParams {
                    name: "dashed".to_string(),
                    repos: vec!["delta".to_string()],
                    strategy: strategy.to_string(),
                    branch: branch.map(str::to_string),
                }))
                .expect_err("a branch beginning with '-' must be refused");
            let msg = invalid_params(&err);
            assert_eq!(
                msg, "'-x' is not a valid branch name",
                "strategy {strategy}"
            );
            assert!(
                !env.workspaces_dir.join("dashed").exists(),
                "nothing is created for strategy {strategy}"
            );
        }
    });
}

#[test]
fn add_repos_rejects_a_traversal_workspace() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);
        // `ws_dir/..` exists, so the plain exists check on master passed and
        // the worktree was added to the parent of ws_dir.
        let err = server
            .add_repos(Parameters(AddReposParams {
                workspace: "..".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: Some("topic".to_string()),
            }))
            .expect_err("'..' must be refused before the exists check");
        let msg = invalid_params(&err);
        assert_eq!(
            msg,
            "invalid space name \"..\": Space name cannot be '.' or '..'"
        );
        assert!(
            !env.dir.path().join("delta").exists(),
            "no worktree may be added to the parent of ws_dir"
        );

        // A traversal name that does not resolve: the guard must answer
        // before the exists check, or the message would say "not found"
        // and reveal whether the traversal target exists.
        let err = server
            .add_repos(Parameters(AddReposParams {
                workspace: "../nonexistent-zzz".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "new".to_string(),
                branch: Some("topic".to_string()),
            }))
            .expect_err("a traversal name must be refused whether or not it resolves");
        let msg = invalid_params(&err);
        assert_eq!(
            msg,
            "invalid space name \"../nonexistent-zzz\": Space name cannot contain '/' or '\\'"
        );
    });
}

#[test]
fn remove_workspace_refuses_dot_dot_and_dot() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        create_worktree(
            &repo_path,
            &env.workspaces_dir,
            "keep",
            &BranchStrategy::NewBranch("keep".to_string()),
        )
        .unwrap();
        let kept = env.workspaces_dir.join("keep").join("delta");

        for name in ["..", ".", ""] {
            let err = server
                .remove_workspace(Parameters(RemoveWorkspaceParams {
                    name: name.to_string(),
                }))
                .expect_err("a name that is not one plain component must be refused");
            let msg = invalid_params(&err);
            assert!(msg.starts_with("invalid space name"), "got {msg}");
            assert!(
                kept.join(".git").exists(),
                "the real space must survive a refused remove of {name:?}"
            );
            assert!(
                env.config_dir.join("config.toml").exists(),
                "the parent of ws_dir must survive a refused remove of {name:?}"
            );
        }
    });
}

#[test]
fn workspace_status_rejects_a_slash_name() {
    with_test_env(|env, server| {
        env.create_repo("delta");

        let err = server
            .workspace_status(Parameters(WorkspaceStatusParams {
                name: "../repos".to_string(),
            }))
            .expect_err("a name containing '/' must be refused, not listed");
        let msg = invalid_params(&err);
        assert!(
            msg.contains("Space name cannot contain '/' or '\\'"),
            "got {msg}"
        );
    });
}

/// Ticket 13, found in review: `existing` with `origin/-x` passed both the
/// whole-name git check and the dash guard, and the stripped `-x` reached
/// `-b`. The check now runs on the name git will create.
#[test]
fn create_workspace_rejects_an_origin_prefixed_dash_branch() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);

        let err = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "dashed".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "existing".to_string(),
                branch: Some("origin/-x".to_string()),
            }))
            .expect_err("the local name git would create begins with '-'");
        let msg = invalid_params(&err);
        assert_eq!(msg, "'-x' is not a valid branch name");
        assert!(!env.workspaces_dir.join("dashed").exists());
    });
}

/// Ticket 13, coverage found in review: the creation rule allows interior
/// spaces, so a name with one must create end to end, not only pass the
/// unit table.
#[test]
fn create_workspace_accepts_an_interior_space() {
    with_test_env(|env, server| {
        let repo_path = env.create_repo("delta");
        env.write_cache(&[repo_path]);

        let result = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "my space".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "detached".to_string(),
                branch: None,
            }))
            .expect("an interior space is allowed by the creation rule");
        let parsed: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(parsed["name"], "my space");
        assert!(env
            .workspaces_dir
            .join("my space")
            .join("delta")
            .join(".git")
            .exists());
    });
}

/// Gate `repo`'s `origin` behind a script that leaves `marker` behind when a
/// fetch reaches it (the `remote.origin.uploadpack` technique from the
/// Creating worker's tests). The origin is a bare copy of the repo's `main`,
/// fetched once before the gate goes in so `origin/main` is known.
fn gate_origin(env: &TestEnv, repo: &std::path::Path, name: &str) -> PathBuf {
    let run = |args: &[&str], dir: &std::path::Path| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let origin = env.dir.path().join(format!("{}-origin.git", name));
    run(
        &["init", "-q", "--bare", origin.to_str().unwrap()],
        env.dir.path(),
    );
    run(
        &[
            "remote",
            "add",
            "origin",
            &format!("file://{}", origin.display()),
        ],
        repo,
    );
    run(&["push", "-q", "origin", "main"], repo);
    let marker = env.dir.path().join(format!("FETCHED-{}", name));
    let script = env.dir.path().join(format!("gate-{}.sh", name));
    std::fs::write(
        &script,
        format!(
            "touch \"{}\"\nexec git upload-pack \"$@\"\n",
            marker.display()
        ),
    )
    .unwrap();
    run(
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("/bin/sh {}", script.display()),
        ],
        repo,
    );
    marker
}

/// MCP has no sync stage, so the pre-create fetch was the only fetch a
/// `create_workspace` ran. A `detached` create reads no remote ref and now
/// runs none; the result carries the same fields as before. The `new`
/// contrast on a second gated repo proves the gate: that strategy reads
/// `origin/<base>` and must reach the remote.
#[test]
fn create_workspace_detached_runs_no_fetch() {
    with_test_env(|env, server| {
        let detached = env.create_repo("delta");
        let fresh = env.create_repo("echo");
        env.write_cache(&[detached.clone(), fresh.clone()]);
        let detached_marker = gate_origin(env, &detached, "delta");
        let fresh_marker = gate_origin(env, &fresh, "echo");

        let result = server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "quiet".to_string(),
                repos: vec!["delta".to_string()],
                strategy: "detached".to_string(),
                branch: None,
            }))
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result_text(&result)).unwrap();
        assert_eq!(parsed["name"], "quiet");
        assert_eq!(
            parsed["repos_created"].as_array().unwrap(),
            &[serde_json::json!("delta")]
        );
        assert!(env.workspaces_dir.join("quiet").join("delta").exists());
        assert!(
            !detached_marker.exists(),
            "a detached create reads no remote ref, so no fetch may reach origin"
        );

        server
            .create_workspace(Parameters(CreateWorkspaceParams {
                name: "noisy".to_string(),
                repos: vec!["echo".to_string()],
                strategy: "new".to_string(),
                branch: None,
            }))
            .unwrap();
        assert!(
            fresh_marker.exists(),
            "the contrast proves the gate: a new branch reads origin/<base> and fetches"
        );
    });
}
