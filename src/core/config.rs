use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpaceConfig {
    pub repos: RepoConfig,
    pub workspaces: WorkspaceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoConfig {
    pub roots: Vec<PathBuf>,
    pub max_depth: u32,
    pub cache_age_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceConfig {
    pub dir: PathBuf,
}

impl Default for SpaceConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            repos: RepoConfig {
                roots: vec![home.join("projects")],
                max_depth: 3,
                cache_age_secs: 3600,
            },
            workspaces: WorkspaceConfig {
                dir: home.join("workspaces"),
            },
        }
    }
}

impl SpaceConfig {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
            .join("space")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn cache_path() -> PathBuf {
        Self::config_dir().join("repos.cache")
    }

    /// Load from `~/.config/space/config.toml`, falling back to defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
