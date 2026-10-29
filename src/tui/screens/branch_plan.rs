//! The branch plan shared by the create and add flows: the strategy each
//! selected repo is placed with, recorded as the strategy picker chooses it.
//!
//! Choices are keyed by repo path rather than held in a list beside the
//! screen's `selected_repos`, so the selection stays the one list of repos and
//! a choice cannot drift onto another repo by position.

use crate::core::workspace::BranchStrategy;
use crate::tui::actions::RepoPlan;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tui_input::Input;

/// One repo's recorded choice: the strategy, and the picker state that chose
/// it, so the repo can be reopened where it was left.
#[derive(Debug, Clone)]
pub struct RowChoice {
    pub strategy: BranchStrategy,
    pub strategy_idx: usize,
    pub name_input: Input,
    pub picked_branch: Option<String>,
}

/// Where the flow goes once a choice is recorded.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Every repo has its strategy: start the run.
    Run,
}

#[derive(Debug, Default)]
pub struct BranchPlan {
    choices: BTreeMap<PathBuf, RowChoice>,
}

impl BranchPlan {
    /// Record `choice` for every repo in `repos` and say where the flow goes
    /// next. Every key that chooses a strategy, in both flows, ends here.
    pub fn record(&mut self, repos: &[PathBuf], choice: RowChoice) -> Next {
        for repo in repos {
            self.choices.insert(repo.clone(), choice.clone());
        }
        Next::Run
    }

    /// The run's repos in selection order, each with its recorded strategy.
    pub fn plans(&self, repos: &[PathBuf]) -> Vec<RepoPlan> {
        repos
            .iter()
            .filter_map(|path| {
                self.choices.get(path).map(|c| RepoPlan {
                    path: path.clone(),
                    strategy: c.strategy.clone(),
                })
            })
            .collect()
    }
}
