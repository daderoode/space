use tui_input::Input;

#[derive(Debug)]
pub struct ConfigField {
    pub label: &'static str,
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
                value: config.workspaces.dir.display().to_string(),
            },
            ConfigField {
                label: "Repo roots",
                value: config.repos.roots.iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            },
            ConfigField {
                label: "Max depth",
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
        self.input = self.input.clone().with_value(value.into());
        self.editing = true;
    }

    pub fn commit_edit(&mut self) {
        self.fields[self.focused].value = self.input.value().to_string();
        self.editing = false;
    }

    pub fn cancel_edit(&mut self) {
        self.editing = false;
    }

    /// Apply fields back to a SpaceConfig and save.
    pub fn save_to_config(&self) -> anyhow::Result<crate::core::config::SpaceConfig> {
        let mut config = crate::core::config::SpaceConfig::load()?;

        // Field 0: workspaces dir
        if let Some(f) = self.fields.get(0) {
            config.workspaces.dir = std::path::PathBuf::from(&f.value);
        }
        // Field 1: repo roots (comma-separated)
        if let Some(f) = self.fields.get(1) {
            config.repos.roots = f.value.split(',')
                .map(|s| std::path::PathBuf::from(s.trim()))
                .collect();
        }
        // Field 2: max depth
        if let Some(f) = self.fields.get(2) {
            if let Ok(d) = f.value.parse::<u32>() {
                config.repos.max_depth = d;
            }
        }

        config.save()?;
        Ok(config)
    }
}
