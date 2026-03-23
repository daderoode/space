use rmcp::ServerHandler;
use space::core::workspace::BranchStrategy;
use space::mcp::SpaceServer;
use std::path::PathBuf;

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
