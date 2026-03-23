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
