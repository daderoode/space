use crate::tui::actions::{ScreenAction, ScreenContext};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &ScreenContext) -> ScreenAction {
        // Ctrl-S: commit any active edit, save, exit
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.editing {
                self.commit_edit();
            }
            let base = ctx.config.clone();
            match self.save_to_config(base) {
                Ok(new_config) => return ScreenAction::SaveConfig(new_config),
                Err(e) => return ScreenAction::BackWithStatus(format!("Save failed: {}", e)),
            }
        }

        if self.editing {
            match key.code {
                KeyCode::Esc => self.cancel_edit(),
                KeyCode::Enter => {
                    self.commit_edit();
                    let next = (self.focused + 1).min(self.fields.len() - 1);
                    self.focused = next;
                }
                _ => {
                    if let Some(req) = crate::tui::app::key_to_input_request(&key) {
                        self.input.handle(req);
                    }
                }
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => return ScreenAction::Back,
                KeyCode::Enter => self.start_editing(),
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.focused > 0 {
                        self.focused -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.focused + 1 < self.fields.len() {
                        self.focused += 1;
                    }
                }
                _ => {}
            }
        }
        ScreenAction::Continue
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
