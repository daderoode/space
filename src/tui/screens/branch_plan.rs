//! The branch plan shared by the create and add flows: the strategy each
//! selected repo is placed with, recorded as the strategy picker chooses it.
//!
//! Choices are keyed by repo path rather than held in a list beside the
//! screen's `selected_repos`, so the selection stays the one list of repos and
//! a choice cannot drift onto another repo by position.
//!
//! With more than one repo selected, the flow forks after the sync report:
//! one strategy for every repo (the picker runs once, as it always has), or
//! one per repo (the picker runs once for each repo in selection order, and
//! a review table lists every repo's choice before anything is created).
//!
//! A run attempts every repo. A repo refused because its branch is checked
//! out elsewhere does not stop the others; once the run ends the flow returns
//! to the review table with that repo marked, the repos in place shown as
//! such and closed to change, and the next run attempts only what is not in
//! place.

use crate::core::git::BranchInfo;
use crate::core::workspace::BranchStrategy;
use crate::tui::actions::RepoPlan;
use ratatui::crossterm::event::KeyCode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tui_input::Input;

/// How many recent branches the strategy picker lists.
const RECENT_BRANCHES: usize = 5;

/// Display columns a message gives one repo's name. The messages are drawn
/// in two wrapped rows of the dialog's inner width, 60 at the documented
/// 80-column minimum, and a name past this would push the part that says
/// what to do onto a third row that is never drawn. Pinned by
/// `the_refusal_messages_read_in_full_at_eighty_columns`.
const MESSAGE_NAME_COLS: usize = 32;

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
    /// Open the strategy picker for the next repo (`BranchPlan::current`).
    Configure,
    /// Every repo has its strategy: show the review table.
    Review,
    /// Every repo has its strategy: start the run.
    Run,
}

/// Where Esc on the strategy picker goes.
#[derive(Debug, PartialEq, Eq)]
pub enum Back {
    /// The previous repo's picker, as that repo was left.
    PreviousRepo,
    /// The review table, the repo's choice unchanged: it was reopened from
    /// there.
    Review,
    /// The fork between one strategy for all and one per repo.
    Fork,
    /// The repo picker: a single repo has no fork.
    Repos,
}

/// How a repo's last attempt in this flow turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowOutcome {
    /// Its worktree is in the space, created by a run or found there.
    Placed,
    /// Git refused it because the branch is checked out elsewhere.
    Refused,
    /// It failed for another reason, with git's text, which the review
    /// table shows for the highlighted row.
    Failed(String),
}

/// What a key on the review table did.
#[derive(Debug, PartialEq, Eq)]
pub enum ReviewStep {
    Stay,
    /// Enter on a repo already in place: its strategy no longer decides
    /// anything (a run skips a repo already in the space), so it is closed
    /// to change.
    Locked,
    /// Esc after a run: leave the flow, with what is in place left there.
    Leave,
    /// Enter on a repo: open its picker (`BranchPlan::current`), to return
    /// to the table once it is chosen.
    Edit,
    /// Enter on the last row: start the run.
    Run,
    /// Esc: back to the last repo's picker.
    LastRepo,
}

/// What a key on the fork did.
#[derive(Debug, PartialEq, Eq)]
pub enum ForkStep {
    Stay,
    /// A mode was chosen: open the strategy picker.
    Chosen,
    /// Esc: back to the repo picker.
    Back,
}

#[derive(Debug, Default)]
pub struct BranchPlan {
    choices: BTreeMap<PathBuf, RowChoice>,
    /// One strategy per repo rather than one for all.
    pub per_repo: bool,
    /// The fork's highlighted option: 0 is one strategy for all, 1 per repo.
    pub fork_idx: usize,
    /// In per-repo mode, or while a repo reopened from the review table is
    /// being chosen, the index in the selection of the repo the picker is
    /// choosing for.
    pub current: usize,
    /// The review table's highlighted row: a repo's index in the selection,
    /// or the selection's length for the row that starts the run.
    pub review_cursor: usize,
    /// The picker is choosing for one repo reopened from the review table,
    /// and goes back there once it has.
    editing: bool,
    /// What each repo's last attempt came to; empty until a run ends.
    outcomes: BTreeMap<PathBuf, RowOutcome>,
    /// A run has ended in this flow. Kept apart from `outcomes`, which
    /// empties again as the repos marked are chosen anew, while the table
    /// stays the one after a run: Esc leaves rather than reopening a picker.
    ran: bool,
}

impl BranchPlan {
    /// The fork's keys. Enter settles the mode and starts at the first repo.
    pub fn fork_key(&mut self, code: KeyCode) -> ForkStep {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.fork_idx = 0;
                ForkStep::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.fork_idx = 1;
                ForkStep::Stay
            }
            KeyCode::Enter => {
                self.per_repo = self.fork_idx == 1;
                self.current = 0;
                ForkStep::Chosen
            }
            KeyCode::Esc => ForkStep::Back,
            _ => ForkStep::Stay,
        }
    }

    /// Whether the picker is choosing one strategy for every repo, rather
    /// than for the one repo at `current`.
    fn for_all(&self) -> bool {
        !self.per_repo && !self.editing
    }

    /// The branches the picker offers: the repo being configured's own, or
    /// with one strategy for all, the branches every selected repo has
    /// (`git::shared_branches`), so whatever is picked exists in each repo
    /// it is used for.
    fn picker_branches(&self, repos: &[PathBuf]) -> anyhow::Result<Vec<BranchInfo>> {
        if self.for_all() {
            crate::core::git::shared_branches(repos)
        } else {
            match repos.get(self.current) {
                Some(repo) => crate::core::git::list_branches(repo),
                None => Ok(Vec::new()),
            }
        }
    }

    /// Where the picker's branches come from, as its titles name it: the
    /// repo, or with one strategy for several, all of them.
    fn picker_source(&self, repos: &[PathBuf]) -> String {
        match repos {
            [_, _, ..] if self.for_all() => format!("all {} repos", repos.len()),
            _ => repos
                .get(if self.for_all() { 0 } else { self.current })
                .map(|p| repo_name(p))
                .unwrap_or_default(),
        }
    }

    /// The full branch picker that Show more opens, or what to say when it
    /// cannot open: no branch is in every selected repo, or none could be
    /// listed.
    pub fn branch_picker(
        &self,
        repos: &[PathBuf],
    ) -> Result<crate::tui::widgets::fuzzy_picker::FuzzyPicker, String> {
        let source = self.picker_source(repos);
        let branches = self.picker_branches(repos).ok();
        let shared_none =
            self.for_all() && repos.len() > 1 && branches.as_ref().is_some_and(|b| b.is_empty());
        match branches.and_then(|b| crate::tui::app::branch_picker_of(b, &source, "Branch")) {
            Some(picker) => Ok(picker),
            None if shared_none => Err(format!("No branch is in {}", source)),
            None => Err(format!("Could not list branches for {}", source)),
        }
    }

    /// Set the strategy picker's fields for the repo it is about to choose
    /// for: its recent branches (with one strategy for all, those every repo
    /// has, which may be none), and the cursor, name field and
    /// picked branch the repo was left with, or the defaults for a repo not
    /// chosen yet. Called every time the picker changes repo, because each
    /// field otherwise carries over from the repo before: a cursor past the
    /// end of a shorter recent list indexes out of bounds on Enter, and a
    /// name typed for one repo would open as the next repo's branch.
    ///
    /// With one strategy for all, the cursor resets and the name field and
    /// picked branch are left as they are, which is how the single picker
    /// has always behaved (a typed name survives going back to rename the
    /// space).
    pub fn open_picker(
        &self,
        repos: &[PathBuf],
        strategy_idx: &mut usize,
        name_input: &mut Input,
        picked_branch: &mut Option<String>,
        recent: &mut Vec<BranchInfo>,
    ) {
        *recent = self
            .picker_branches(repos)
            .map(|b| crate::core::git::most_recent(b, RECENT_BRANCHES))
            .unwrap_or_default();
        if self.for_all() {
            *strategy_idx = 0;
            return;
        }
        match repos.get(self.current).and_then(|p| self.choices.get(p)) {
            Some(choice) => {
                *strategy_idx = cursor_for(choice, recent);
                *name_input = choice.name_input.clone();
                *picked_branch = choice.picked_branch.clone();
            }
            None => {
                *strategy_idx = 0;
                *name_input = Input::default();
                *picked_branch = None;
            }
        }
    }

    /// Record `choice` for the repo the picker was choosing for (every repo,
    /// with one strategy for all) and say where the flow goes next. Every key
    /// that chooses a strategy, in both flows, ends here.
    pub fn record(&mut self, repos: &[PathBuf], choice: RowChoice) -> Next {
        if self.editing {
            if let Some(repo) = repos.get(self.current) {
                // A new choice is a new attempt: the repo is no longer marked
                // for how the last one went.
                self.outcomes.remove(repo);
                self.choices.insert(repo.clone(), choice);
            }
            self.editing = false;
            self.review_cursor = repos.len();
            return Next::Review;
        }
        if self.for_all() {
            for repo in repos {
                if self.outcomes.get(repo) != Some(&RowOutcome::Placed) {
                    self.outcomes.remove(repo);
                    self.choices.insert(repo.clone(), choice.clone());
                }
            }
            return Next::Run;
        }
        if let Some(repo) = repos.get(self.current) {
            self.choices.insert(repo.clone(), choice);
        }
        if self.current + 1 < repos.len() {
            self.current += 1;
            Next::Configure
        } else {
            // The table opens on the row that starts the run, so Enter
            // confirms what it lists.
            self.review_cursor = repos.len();
            Next::Review
        }
    }

    /// Esc on the strategy picker.
    pub fn back(&mut self, repos: &[PathBuf]) -> Back {
        if self.editing {
            self.editing = false;
            Back::Review
        } else if self.per_repo && self.current > 0 {
            self.current -= 1;
            Back::PreviousRepo
        } else if repos.len() > 1 {
            Back::Fork
        } else {
            Back::Repos
        }
    }

    /// The review table's keys. Its rows are the repos in selection order,
    /// then the row that starts the run.
    pub fn review_key(&mut self, code: KeyCode, repos: &[PathBuf]) -> ReviewStep {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.review_cursor = self.review_cursor.saturating_sub(1);
                ReviewStep::Stay
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.review_cursor = (self.review_cursor + 1).min(repos.len());
                ReviewStep::Stay
            }
            KeyCode::Enter if self.review_cursor >= repos.len() => ReviewStep::Run,
            KeyCode::Enter => {
                let placed = repos
                    .get(self.review_cursor)
                    .is_some_and(|r| self.outcomes.get(r) == Some(&RowOutcome::Placed));
                if placed {
                    return ReviewStep::Locked;
                }
                self.current = self.review_cursor;
                self.editing = true;
                ReviewStep::Edit
            }
            KeyCode::Esc if self.has_run() => ReviewStep::Leave,
            KeyCode::Esc => {
                self.current = repos.len().saturating_sub(1);
                ReviewStep::LastRepo
            }
            _ => ReviewStep::Stay,
        }
    }

    /// Record how a repo's attempt turned out. Called as each repo of a run
    /// finishes.
    pub fn note_outcome(&mut self, repo: &Path, outcome: RowOutcome) {
        self.outcomes.insert(repo.to_path_buf(), outcome);
    }

    /// How a repo's last attempt turned out, if it has had one.
    pub fn outcome_of(&self, repo: &Path) -> Option<&RowOutcome> {
        self.outcomes.get(repo)
    }

    /// Whether a run has ended in this flow.
    pub fn has_run(&self) -> bool {
        self.ran
    }

    /// The repos refused because their branch is checked out elsewhere.
    pub fn refused(&self, repos: &[PathBuf]) -> usize {
        repos
            .iter()
            .filter(|r| self.outcomes.get(*r) == Some(&RowOutcome::Refused))
            .count()
    }

    /// The repos that failed for another reason.
    pub fn failed(&self, repos: &[PathBuf]) -> usize {
        repos
            .iter()
            .filter(|r| matches!(self.outcomes.get(*r), Some(RowOutcome::Failed(_))))
            .count()
    }

    /// The repos whose worktree is in the space.
    pub fn placed(&self, repos: &[PathBuf]) -> usize {
        repos
            .iter()
            .filter(|r| self.outcomes.get(*r) == Some(&RowOutcome::Placed))
            .count()
    }

    /// Open the review table after a run with refusals, on the first repo
    /// refused.
    pub fn review_after_run(&mut self, repos: &[PathBuf]) {
        self.ran = true;
        self.editing = false;
        self.review_cursor = repos
            .iter()
            .position(|r| self.outcomes.get(r) == Some(&RowOutcome::Refused))
            .unwrap_or(repos.len());
    }

    /// What the flow says on returning after a run in which repos were
    /// refused because their branch is checked out elsewhere: with one repo,
    /// which returns to its picker, the repo by name; otherwise how many,
    /// since the table marks which, and how many failed for another reason,
    /// whose own reason the table shows for the highlighted row.
    pub fn refusal_message(&self, repos: &[PathBuf]) -> String {
        let failed = self.failed(repos);
        match repos {
            [only] => format!(
                "'{}' is already checked out elsewhere. Pick another strategy",
                crate::tui::ui::truncate_for_width(&repo_name(only), MESSAGE_NAME_COLS)
            ),
            _ if failed > 0 => format!(
                "{} of {} repos refused (branch checked out elsewhere), {} failed. \
                 Choose again for them, then retry",
                self.refused(repos),
                repos.len(),
                failed
            ),
            _ => format!(
                "{} of {} repos refused: the branch is checked out elsewhere. \
                 Choose again for them, then retry",
                self.refused(repos),
                repos.len()
            ),
        }
    }

    /// What Enter on a repo already in place says.
    pub fn locked_message(&self, repos: &[PathBuf]) -> String {
        let name = repos
            .get(self.review_cursor)
            .map(|r| repo_name(r))
            .unwrap_or_default();
        format!(
            "'{}' is already in the space, so its strategy cannot change",
            crate::tui::ui::truncate_for_width(&name, MESSAGE_NAME_COLS)
        )
    }

    /// What leaving the table after a run says: how much of the selection is
    /// in place, and how to add the rest. No space name, so it fits the
    /// one-row status line at 80 columns.
    pub fn leave_message(&self, repos: &[PathBuf]) -> String {
        format!(
            "{} of {} repos in place. Press a to add the rest.",
            self.placed(repos),
            repos.len()
        )
    }

    /// A repo's recorded strategy.
    pub fn strategy_of(&self, repo: &Path) -> Option<&BranchStrategy> {
        self.choices.get(repo).map(|c| &c.strategy)
    }

    /// The run's repos in selection order, each with its recorded strategy.
    /// A repo already in place is left out: a retry attempts only what is
    /// missing.
    pub fn plans(&self, repos: &[PathBuf]) -> Vec<RepoPlan> {
        repos
            .iter()
            .filter(|path| self.outcomes.get(*path) != Some(&RowOutcome::Placed))
            .filter_map(|path| {
                self.choices.get(path).map(|c| RepoPlan {
                    path: path.clone(),
                    strategy: c.strategy.clone(),
                })
            })
            .collect()
    }

    /// The strategy picker's title: which repo it is choosing for. The repo
    /// name is cut to `name_cols` so the count after it stays on the border.
    pub fn picker_title(&self, repos: &[PathBuf], name_cols: usize) -> String {
        if !self.for_all() {
            let name = repos
                .get(self.current)
                .map(|p| repo_name(p))
                .unwrap_or_default();
            format!(
                " Branch Strategy: {} ({} of {}) ",
                crate::tui::ui::truncate_for_width(&name, name_cols),
                self.current + 1,
                repos.len()
            )
        } else if repos.len() > 1 {
            format!(" Branch Strategy: all {} repos ", repos.len())
        } else {
            " Branch Strategy ".to_string()
        }
    }
}

/// The picker row a recorded choice came from, in this repo's own list: a
/// fixed option keeps its row, a branch still among the recent ones gets its
/// row, and any other branch lands on the row that opens the full picker.
/// Never past that row, so Enter cannot index past the recent list.
fn cursor_for(choice: &RowChoice, recent: &[BranchInfo]) -> usize {
    if choice.strategy_idx < 3 {
        return choice.strategy_idx;
    }
    if let BranchStrategy::ExistingBranch(name) = &choice.strategy {
        if let Some(i) = recent.iter().position(|b| &b.name == name) {
            return 3 + i;
        }
    }
    3 + recent.len()
}

/// A repo's directory name, as the flows label it.
pub fn repo_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string())
}

/// A strategy in a few words, for the review table and for the Creating log
/// of a run whose repos do not all share one.
pub fn describe(strategy: &BranchStrategy) -> String {
    match strategy {
        BranchStrategy::NewBranch(name) => format!("new branch '{}'", name),
        BranchStrategy::ExistingBranch(name) => format!("branch '{}'", name),
        BranchStrategy::DetachedHead => "detached HEAD".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_current: false,
            is_remote: false,
            last_commit_time: 0,
        }
    }

    fn choice(strategy: BranchStrategy, strategy_idx: usize) -> RowChoice {
        RowChoice {
            strategy,
            strategy_idx,
            name_input: Input::default(),
            picked_branch: None,
        }
    }

    fn repos(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| PathBuf::from(format!("/r/{}", n)))
            .collect()
    }

    #[test]
    fn a_fixed_option_reopens_on_its_own_row() {
        let recent = [branch("x")];
        for idx in 0..3 {
            assert_eq!(
                cursor_for(&choice(BranchStrategy::DetachedHead, idx), &recent),
                idx
            );
        }
    }

    #[test]
    fn a_recent_branch_reopens_on_its_row_in_this_repos_list() {
        let strategy = BranchStrategy::ExistingBranch("b".to_string());
        // Chosen as the first recent row of another repo's list.
        let c = choice(strategy, 3);
        assert_eq!(
            cursor_for(&c, &[branch("a"), branch("b")]),
            4,
            "the row naming the branch, not the row number it was chosen at"
        );
    }

    #[test]
    fn a_branch_not_in_the_list_reopens_on_show_more_and_never_past_it() {
        let c = choice(BranchStrategy::ExistingBranch("gone".to_string()), 7);
        assert_eq!(
            cursor_for(&c, &[branch("a")]),
            4,
            "Show more for one recent"
        );
        assert_eq!(cursor_for(&c, &[]), 3, "Pick a branch... with none listed");
    }

    #[test]
    fn one_strategy_for_all_records_every_repo_and_runs() {
        let rs = repos(&["a", "b"]);
        let mut plan = BranchPlan::default();
        assert_eq!(
            plan.record(&rs, choice(BranchStrategy::DetachedHead, 2)),
            Next::Run
        );
        assert_eq!(plan.plans(&rs).len(), 2);
    }

    #[test]
    fn per_repo_records_one_repo_at_a_time_and_esc_walks_back() {
        let rs = repos(&["a", "b"]);
        let mut plan = BranchPlan {
            fork_idx: 1,
            ..BranchPlan::default()
        };
        assert_eq!(plan.fork_key(KeyCode::Enter), ForkStep::Chosen);
        assert!(plan.per_repo);
        assert_eq!(
            plan.record(&rs, choice(BranchStrategy::DetachedHead, 2)),
            Next::Configure
        );
        assert_eq!(plan.current, 1);
        assert_eq!(plan.plans(&rs).len(), 1, "only the first repo is chosen");
        assert_eq!(plan.back(&rs), Back::PreviousRepo);
        assert_eq!(plan.current, 0);
        assert_eq!(plan.back(&rs), Back::Fork);
        assert_eq!(BranchPlan::default().back(&rs[..1]), Back::Repos);
    }

    /// One strategy for both repos, both refused, as the run leaves them.
    fn both_refused(rs: &[PathBuf]) -> BranchPlan {
        let mut plan = BranchPlan::default();
        let topic = BranchStrategy::ExistingBranch("topic".to_string());
        assert_eq!(plan.record(rs, choice(topic, 1)), Next::Run);
        for r in rs {
            plan.note_outcome(r, RowOutcome::Refused);
        }
        plan.review_after_run(rs);
        plan
    }

    /// Choose again for the repo at `row` from the table.
    fn rechoose(plan: &mut BranchPlan, rs: &[PathBuf], row: usize) {
        plan.review_cursor = row;
        assert_eq!(plan.review_key(KeyCode::Enter, rs), ReviewStep::Edit);
        assert_eq!(
            plan.record(rs, choice(BranchStrategy::DetachedHead, 2)),
            Next::Review
        );
    }

    #[test]
    fn choosing_again_for_a_refused_repo_clears_its_mark_and_no_other() {
        let rs = repos(&["a", "b"]);
        let mut plan = both_refused(&rs);
        rechoose(&mut plan, &rs, 0);
        assert_eq!(plan.outcome_of(&rs[0]), None, "a new choice, a new attempt");
        assert_eq!(plan.outcome_of(&rs[1]), Some(&RowOutcome::Refused));
        assert_eq!(plan.refused(&rs), 1);
    }

    #[test]
    fn esc_on_the_table_leaves_after_a_run_even_once_every_mark_is_cleared() {
        let rs = repos(&["a", "b"]);
        let mut plan = both_refused(&rs);
        rechoose(&mut plan, &rs, 0);
        rechoose(&mut plan, &rs, 1);
        assert_eq!(plan.refused(&rs), 0, "every mark is cleared");
        assert!(plan.has_run());
        assert_eq!(
            plan.review_key(KeyCode::Esc, &rs),
            ReviewStep::Leave,
            "not the picker for all repos, whose choice would overwrite both"
        );
    }
}
