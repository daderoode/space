#[derive(Debug)]
pub struct DeleteState {
    pub workspace_name: String,
    pub workspace_path: std::path::PathBuf,
    pub repo_names: Vec<String>,
}
