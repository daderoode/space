use crate::core::config::SpaceConfig;
use crate::core::workspace::BranchStrategy;
use crate::core::{repo, workspace};
use anyhow::Result;
use rmcp::{
    handler::server::router::tool::ToolRouter, handler::server::wrapper::Parameters, model::*,
    schemars, tool, tool_handler, tool_router, transport::stdio, ErrorData as McpError,
    ServerHandler, ServiceExt,
};
use serde::Serialize;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Input parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WorkspaceStatusParams {
    /// Name of the workspace to inspect.
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListReposParams {
    /// Rescan the filesystem instead of using the cache.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateWorkspaceParams {
    /// Workspace name (will become a directory name).
    pub name: String,
    /// Repository names to include (matched against the repo cache).
    pub repos: Vec<String>,
    /// Branch strategy: "new" (default), "existing", or "detached".
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Branch name. Required when strategy is "new" or "existing".
    /// Defaults to the workspace name when strategy is "new".
    pub branch: Option<String>,
}

fn default_strategy() -> String {
    "new".to_string()
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddReposParams {
    /// Existing workspace to add repos to.
    pub workspace: String,
    /// Repository names to add.
    pub repos: Vec<String>,
    /// Branch strategy: "new" (default), "existing", or "detached".
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// Branch name. Required when strategy is "new" or "existing".
    pub branch: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveWorkspaceParams {
    /// Workspace to remove.
    pub name: String,
}

// ---------------------------------------------------------------------------
// Output types (JSON-serializable)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RepoInfo {
    name: String,
    path: PathBuf,
}

#[derive(Serialize)]
struct CreateResult {
    name: String,
    path: PathBuf,
    repos_created: Vec<String>,
}

#[derive(Serialize)]
struct AddResult {
    workspace: String,
    added: Vec<String>,
}

#[derive(Serialize)]
struct RemoveResult {
    removed: String,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SpaceServer {
    tool_router: ToolRouter<SpaceServer>,
}

/// Resolve repo names to filesystem paths using the repo cache.
/// Returns an error if any name is not found or is ambiguous.
pub fn resolve_repos(
    names: &[String],
    cache: &[PathBuf],
) -> std::result::Result<Vec<PathBuf>, String> {
    let mut resolved = Vec::with_capacity(names.len());
    for name in names {
        let matches: Vec<&PathBuf> = cache
            .iter()
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
                    .unwrap_or(false)
            })
            .collect();
        match matches.len() {
            0 => {
                return Err(format!(
                    "repo '{}' not found in cache. Run list_repos with refresh=true to rescan.",
                    name
                ))
            }
            1 => resolved.push(matches[0].clone()),
            n => {
                return Err(format!(
                    "repo '{}' is ambiguous — matched {} repos: {:?}",
                    name,
                    n,
                    matches
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                ))
            }
        }
    }
    Ok(resolved)
}

pub fn build_strategy(
    strategy: &str,
    branch: Option<&str>,
    workspace_name: &str,
) -> std::result::Result<BranchStrategy, String> {
    match strategy {
        "new" => {
            let branch_name = branch.unwrap_or(workspace_name).to_string();
            Ok(BranchStrategy::NewBranch(branch_name))
        }
        "existing" => {
            let branch_name = branch
                .ok_or_else(|| "branch name is required when strategy is 'existing'".to_string())?
                .to_string();
            Ok(BranchStrategy::ExistingBranch(branch_name))
        }
        "detached" => Ok(BranchStrategy::DetachedHead),
        other => Err(format!(
            "unknown strategy '{}'. Use 'new', 'existing', or 'detached'.",
            other
        )),
    }
}

fn load_repo_cache(cfg: &SpaceConfig, refresh: bool) -> Vec<PathBuf> {
    if !refresh {
        if let Some(cached) = repo::load_cache(&SpaceConfig::cache_path(), cfg.repos.cache_age_secs) {
            return cached;
        }
    }
    let repos = repo::find_repos_in(&cfg.repos.roots, cfg.repos.max_depth);
    let _ = repo::save_cache(&SpaceConfig::cache_path(), &repos);
    repos
}

impl Default for SpaceServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl SpaceServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// List all workspaces with their repo status.
    #[tool(description = "List all workspaces with per-repo branch and status information")]
    pub fn list_workspaces(&self) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let ws_dir = &cfg.workspaces.dir;
        let names = workspace::list_workspaces(ws_dir)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut detailed = Vec::new();
        for ws in &names {
            match workspace::workspace_detail(ws_dir, &ws.name) {
                Ok(d) => detailed.push(d),
                Err(_) => detailed.push(workspace::Workspace {
                    name: ws.name.clone(),
                    path: ws.path.clone(),
                    repos: vec![],
                }),
            }
        }

        let json = serde_json::to_string_pretty(&detailed)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get detailed status for a specific workspace.
    #[tool(
        description = "Get detailed repo status for a named workspace (branches, modified/staged/untracked counts, ahead/behind)"
    )]
    pub fn workspace_status(
        &self,
        Parameters(params): Parameters<WorkspaceStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let detail = workspace::workspace_detail(&cfg.workspaces.dir, &params.name)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&detail)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List discoverable git repositories.
    #[tool(
        description = "List discoverable git repositories from configured roots. Set refresh=true to rescan the filesystem."
    )]
    pub fn list_repos(
        &self,
        Parameters(params): Parameters<ListReposParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let repos = load_repo_cache(&cfg, params.refresh);
        let info: Vec<RepoInfo> = repos
            .into_iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                RepoInfo { name, path: p }
            })
            .collect();
        let json = serde_json::to_string_pretty(&info)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Create a new workspace with git worktrees for the specified repos.
    #[tool(
        description = "Create a new workspace with git worktrees for selected repos. Strategy: 'new' (create branch, default), 'existing' (checkout existing branch), or 'detached' (detached HEAD)."
    )]
    pub fn create_workspace(
        &self,
        Parameters(params): Parameters<CreateWorkspaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let cache = load_repo_cache(&cfg, false);
        let repo_paths =
            resolve_repos(&params.repos, &cache).map_err(|e| McpError::invalid_params(e, None))?;
        let strategy = build_strategy(&params.strategy, params.branch.as_deref(), &params.name)
            .map_err(|e| McpError::invalid_params(e, None))?;

        let ws_dir = &cfg.workspaces.dir;
        let mut created = Vec::new();
        for repo_path in &repo_paths {
            workspace::create_worktree(repo_path, ws_dir, &params.name, &strategy).map_err(
                |e| {
                    McpError::internal_error(
                        format!(
                            "failed to create worktree for {}: {}",
                            repo_path.display(),
                            e
                        ),
                        None,
                    )
                },
            )?;
            let name = repo_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            created.push(name);
        }

        let result = CreateResult {
            name: params.name.clone(),
            path: ws_dir.join(&params.name),
            repos_created: created,
        };
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Add repos to an existing workspace.
    #[tool(description = "Add git worktrees for additional repos to an existing workspace")]
    pub fn add_repos(
        &self,
        Parameters(params): Parameters<AddReposParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let ws_dir = &cfg.workspaces.dir;

        // Verify workspace exists
        let ws_path = ws_dir.join(&params.workspace);
        if !ws_path.exists() {
            return Err(McpError::invalid_params(
                format!("workspace '{}' not found", params.workspace),
                None,
            ));
        }

        let cache = load_repo_cache(&cfg, false);
        let repo_paths =
            resolve_repos(&params.repos, &cache).map_err(|e| McpError::invalid_params(e, None))?;

        // Determine branch from existing workspace repos or params
        let branch_name = params.branch.unwrap_or_else(|| params.workspace.clone());
        let strategy = build_strategy(&params.strategy, Some(&branch_name), &params.workspace)
            .map_err(|e| McpError::invalid_params(e, None))?;

        let mut added = Vec::new();
        for repo_path in &repo_paths {
            workspace::create_worktree(repo_path, ws_dir, &params.workspace, &strategy).map_err(
                |e| {
                    McpError::internal_error(
                        format!("failed to add worktree for {}: {}", repo_path.display(), e),
                        None,
                    )
                },
            )?;
            let name = repo_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            added.push(name);
        }

        let result = AddResult {
            workspace: params.workspace,
            added,
        };
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Remove a workspace and all its worktrees.
    #[tool(description = "Remove a workspace and all its git worktrees")]
    pub fn remove_workspace(
        &self,
        Parameters(params): Parameters<RemoveWorkspaceParams>,
    ) -> Result<CallToolResult, McpError> {
        let cfg = SpaceConfig::load().map_err(|e| McpError::internal_error(e.to_string(), None))?;
        workspace::remove_workspace(&cfg.workspaces.dir, &params.name, true)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let result = RemoveResult {
            removed: params.name,
        };
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for SpaceServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("space-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Space workspace manager. Tools: list_workspaces, workspace_status, list_repos, \
             create_workspace, add_repos, remove_workspace. Use list_repos to discover \
             available repositories, then create_workspace to set up multi-repo worktree \
             workspaces for feature work."
                    .to_string(),
            )
    }
}

/// Start the MCP server on stdio. Creates its own tokio runtime.
pub fn run() -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            )
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();

        tracing::info!("Starting space MCP server");

        let server = SpaceServer::new();
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}
