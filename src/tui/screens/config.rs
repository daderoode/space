use tui_input::Input;

/// Replace $HOME prefix with ~ for display
pub fn tilde_collapse(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        if path.starts_with(&home_str) {
            return format!("~{}", &path[home_str.len()..]);
        }
    }
    path.to_string()
}

/// Expand leading ~ to $HOME for saving
pub fn tilde_expand(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

#[derive(Debug)]
pub struct ConfigField {
    pub label: &'static str,
    #[allow(dead_code)]
    pub hint: &'static str, // grey subtext shown next to label, empty string if none
    pub value: String,
}

#[derive(Debug)]
pub struct ConfigState {
    pub fields: Vec<ConfigField>,
    pub focused: usize,
    pub editing: bool,
    pub input: Input,
}

impl ConfigState {
    pub fn from_config(config: &crate::core::config::SpaceConfig) -> Self {
        let fields = vec![
            ConfigField {
                label: "Workspaces dir",
                hint: "",
                value: tilde_collapse(&config.workspaces.dir.display().to_string()),
            },
            ConfigField {
                label: "Repo roots",
                hint: "(comma-separated)",
                value: config
                    .repos
                    .roots
                    .iter()
                    .map(|p| tilde_collapse(&p.display().to_string()))
                    .collect::<Vec<_>>()
                    .join(", "),
            },
            ConfigField {
                label: "Max depth",
                hint: "(integer)",
                value: config.repos.max_depth.to_string(),
            },
        ];
        ConfigState {
            fields,
            focused: 0,
            editing: false,
            input: Input::default(),
        }
    }

    pub fn start_editing(&mut self) {
        let value = self.fields[self.focused].value.clone();
        self.input = self.input.clone().with_value(value);
        self.editing = true;
    }

    pub fn commit_edit(&mut self) {
        self.fields[self.focused].value = self.input.value().to_string();
        self.editing = false;
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
    }

    /// Apply fields back to the provided base config and save to disk.
    /// Takes the in-memory config as base (avoids TOCTOU with re-loading from disk).
    pub fn save_to_config(
        &self,
        base: crate::core::config::SpaceConfig,
    ) -> anyhow::Result<crate::core::config::SpaceConfig> {
        let mut config = base;

        // Field 0: workspaces dir
        if let Some(f) = self.fields.first() {
            config.workspaces.dir = std::path::PathBuf::from(tilde_expand(f.value.trim()));
        }
        // Field 1: repo roots (comma-separated)
        if let Some(f) = self.fields.get(1) {
            config.repos.roots = f
                .value
                .split(',')
                .map(|s| std::path::PathBuf::from(tilde_expand(s.trim())))
                .collect();
        }
        // Field 2: max depth — return error if not a valid number
        if let Some(f) = self.fields.get(2) {
            config.repos.max_depth = f
                .value
                .parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Max depth must be a number, got: '{}'", f.value))?;
        }

        config.save()?;
        Ok(config)
    }
}
