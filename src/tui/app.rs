use crate::core::{
    config::SpaceConfig,
    git::{FileDiff, FileEntry},
    workspace::{self, FetchOutcome, SyncOutcome, Workspace},
};
use crate::tui::actions::StatusKind;
use crate::tui::screens::sync_report::PAGE_ROWS;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffCacheKey {
    pub repo_index: usize,
    pub path: String,
    pub staged: bool,
}

/// Sent from the App to the background workspace-loader thread.
pub struct LoadRequest {
    pub generation: u64,
    pub ws_dir: std::path::PathBuf,
    pub name: String,
}

/// Sent from the background thread back to the App.
pub struct LoadResult {
    pub generation: u64,
    /// `Some(ws)` on success, `None` if `workspace_detail` returned an error.
    pub workspace: Option<crate::core::workspace::Workspace>,
}

/// Sent from the sync worker thread back to the App during the Syncing stage.
/// `index` addresses the repo's row in the sync report (selection order); the
/// UI formats the rows from the structured outcome.
pub enum SyncProgress {
    Started { index: usize },
    Finished { index: usize, outcome: SyncOutcome },
    Done,
}

/// Sent from the Creating worker thread back to the App during the Creating
/// stage. `index` addresses the repo's position in `WorktreeParams::repos`.
pub enum CreateProgress {
    Started {
        index: usize,
    },
    Finished {
        index: usize,
        fetch: Option<FetchOutcome>,
        /// Flattened to the text at the worker boundary: the App only ever
        /// needs what to print, and this keeps `anyhow::Error` off the channel.
        created: Result<(), String>,
    },
    /// The worker chose to end early and says why. Distinct from `Done` so a
    /// deliberate stop is stated rather than inferred from a short count, and
    /// distinct from a silent disconnect, which means the worker died.
    Stopped(CreateStop),
    /// Every repo was attempted.
    Done,
}

/// Why the Creating worker ended its run early.
pub enum CreateStop {
    /// `git worktree add` refused because the branch is checked out
    /// elsewhere. The App bounces back to the branch-strategy picker.
    AlreadyCheckedOut { index: usize },
}

/// Sent from the git-ops worker thread back to the App during the Running stage.
///
/// Unlike `SyncProgress::Done`, the terminal `Done` here carries `success` so the
/// UI can decide between auto-close (success) and stay-open (failure).
pub enum GitOpProgress {
    Line(String),
    Done { success: bool },
}

/// The Creating stage's live worker: its channel, its cancel flag, the
/// parameters it was started with, and the three counts the UI reads.
///
/// The counts are separate because they answer different questions and are
/// never derived from one another: the footer shows how far the run has got,
/// the cancel message says how many worktrees exist, and the disconnect path
/// asks whether every repo was attempted.
///
/// No generation counter, unlike `ws_generation` on the workspace loader, and
/// the asymmetry is deliberate rather than an oversight. That loader reuses one
/// long-lived channel, so a stale reply can arrive on it and has to be
/// recognised and dropped. Here every write to `App::create_job` replaces the
/// whole struct, including a brand-new `sync_channel`, so the previous `rx` is
/// dropped at that instant: the old worker's next `send` returns `Err` and it
/// returns, and a message can only ever be received on the channel it was sent
/// on. Applying one run's message to another run's state is structurally
/// impossible here, not merely avoided, so there is nothing for an id to
/// discriminate.
pub struct CreateJob {
    rx: mpsc::Receiver<CreateProgress>,
    // Relaxed ordering is sufficient: this flag guards no other shared state,
    // so there is no happens-before relationship to establish with the worker.
    cancel: Arc<AtomicBool>,
    params: crate::tui::actions::WorktreeParams,
    /// Repos whose `Started` has arrived; what the footer counts.
    started: usize,
    /// Repos whose `Finished` has arrived, however it went.
    finished: usize,
    /// Repos whose worktree was confirmed created; what the cancel message counts.
    created: usize,
}

const STATUS_MESSAGE_TTL: Duration = Duration::from_secs(5);
const SCROLL_STEP: u16 = 5;

/// Convert a ratatui/crossterm 0.29 KeyEvent into a tui_input InputRequest,
/// bypassing the tui_input crossterm backend which links against crossterm 0.28.
pub(crate) fn key_to_input_request(
    key: &ratatui::crossterm::event::KeyEvent,
) -> Option<tui_input::InputRequest> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use tui_input::InputRequest;
    match (key.code, key.modifiers) {
        (KeyCode::Backspace, KeyModifiers::NONE) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
            Some(InputRequest::DeletePrevChar)
        }
        (KeyCode::Delete, KeyModifiers::NONE) => Some(InputRequest::DeleteNextChar),
        (KeyCode::Left, KeyModifiers::NONE) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            Some(InputRequest::GoToPrevChar)
        }
        (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Char('b'), KeyModifiers::META) => {
            Some(InputRequest::GoToPrevWord)
        }
        (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
            Some(InputRequest::GoToNextChar)
        }
        (KeyCode::Right, KeyModifiers::CONTROL) | (KeyCode::Char('f'), KeyModifiers::META) => {
            Some(InputRequest::GoToNextWord)
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => Some(InputRequest::DeleteLine),
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => Some(InputRequest::DeletePrevWord),
        (KeyCode::Delete, KeyModifiers::CONTROL) => Some(InputRequest::DeleteNextWord),
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => Some(InputRequest::DeleteTillEnd),
        (KeyCode::Char('a'), KeyModifiers::CONTROL) | (KeyCode::Home, KeyModifiers::NONE) => {
            Some(InputRequest::GoToStart)
        }
        (KeyCode::Char('e'), KeyModifiers::CONTROL) | (KeyCode::End, KeyModifiers::NONE) => {
            Some(InputRequest::GoToEnd)
        }
        (KeyCode::Char(c), KeyModifiers::NONE) => Some(InputRequest::InsertChar(c)),
        (KeyCode::Char(c), KeyModifiers::SHIFT) => Some(InputRequest::InsertChar(c)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Left,
    Right,
}

#[derive(Debug)]
pub enum Screen {
    Dashboard,
    CreateWorkspace(crate::tui::screens::create::CreateState),
    GoWorkspace(crate::tui::screens::go::GoState),
    /// Space filter: the `g` picker with an in-place confirm action.
    FilterWorkspace(crate::tui::screens::go::GoState),
    AddRepos(crate::tui::screens::add::AddState),
    ConfirmDelete(crate::tui::screens::delete::DeleteState),
    RepoSearch(crate::tui::screens::search::SearchState),
    ConfigEditor(crate::tui::screens::config::ConfigState),
    DiffViewer(crate::tui::screens::diff::DiffViewerState),
    SwitchBranch(crate::tui::screens::switch_branch::SwitchBranchState),
    GitOps(crate::tui::screens::gitops::GitOpsState),
}

#[derive(Debug)]
pub enum Message {
    Quit,
    FocusNext,
    SelectWorkspaceUp,
    SelectWorkspaceDown,
    SelectRepoUp,
    SelectRepoDown,
    JumpWorkspace(ListJump),
    JumpRepo(ListJump),
    GoToWorkspace,
    StartGo,
    StartFilter,
    StartCreate,
    StartAdd,
    StartDelete,
    StartSearch,
    StartConfig,
    RefreshRepos,
    ToggleRepoExpand,
    CollapseAllRepos,
    StartHelp,
    StageFile {
        repo_index: usize,
        path: String,
        currently_staged: bool,
    },
    StageBulk {
        repo_index: usize,
        stage: bool, // true = stage all, false = unstage all
    },
    OpenDiffViewer {
        repo_index: usize,
        path: String,
        staged: bool,
    },
    ScrollTableLeft,
    ScrollTableRight,
    StartSwitchBranch {
        repo_index: usize,
    },
    StartGitOps {
        repo_index: usize,
    },
}

/// A jump requested on one of the two dashboard lists. `PageUp`/`PageDown`
/// move by `PAGE_ROWS`, the same page the diff viewer, the git-ops log and the
/// sync report use; `First`/`Last` go to the ends.
///
/// Deliberately not bound to `g`/`G`: both letters already carry shipped
/// meanings on the dashboard (`g` opens the space picker on the left pane,
/// `G` the git-ops overlay on the right), so a `g`/`G` paging scheme would give
/// one letter two meanings across the two panes. Design doc open question 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListJump {
    PageUp,
    PageDown,
    First,
    Last,
}

impl ListJump {
    /// The row this jump targets in a list of `len` rows, starting from
    /// `current`. Clamped at both ends; never wraps. An empty list targets 0.
    ///
    /// `current` is clamped first as defence in depth: the cursor is an index
    /// into rows rebuilt on demand, so any path that shrinks the list between
    /// a keypress and this call would otherwise leave `PageUp` past the end and
    /// the caller indexing out of bounds. The one such path that existed,
    /// `ScreenAction::StageFile` returning to the dashboard without
    /// repositioning, is fixed at its source; `skip_headers` clamps for the
    /// same reason.
    fn target(self, current: usize, len: usize) -> usize {
        let last = len.saturating_sub(1);
        let current = current.min(last);
        match self {
            ListJump::PageUp => current.saturating_sub(PAGE_ROWS),
            ListJump::PageDown => current.saturating_add(PAGE_ROWS).min(last),
            ListJump::First => 0,
            ListJump::Last => last,
        }
    }
}

/// A row in the flattened repo table (repo header or file entry).
#[allow(dead_code)] // fields consumed by renderer (Task 5) and tests
pub enum RepoRow<'a> {
    Repo {
        index: usize,
        repo: &'a crate::core::workspace::WorkspaceRepo,
        expanded: bool,
    },
    /// Section divider within an expanded repo: "Conflicts", "Unstaged", or "Staged".
    SectionHeader {
        repo_index: usize,
        label: &'static str,
    },
    /// A file entry within an expanded repo.
    /// `partially_staged` is true when the same path appears in both the
    /// Staged and Unstaged sections (some hunks staged, some not).
    File {
        repo_index: usize,
        entry: &'a FileEntry,
        partially_staged: bool,
    },
}

pub struct App {
    pub config: SpaceConfig,
    pub workspaces: Vec<Workspace>,
    pub repos_cache: Vec<PathBuf>,
    pub selected_ws: usize,
    pub selected_repo: usize,
    pub expanded_repos: HashSet<usize>,
    pub repo_file_cache: HashMap<usize, Vec<FileEntry>>,
    pub diff_content_cache: HashMap<DiffCacheKey, Result<FileDiff, String>>,
    pub file_mtime_cache: HashMap<DiffCacheKey, std::time::SystemTime>,
    pub repo_index_mtime: HashMap<usize, std::time::SystemTime>,
    pub cursor_row: usize,
    pub table_scroll_x: u16,
    pub focus: Pane,
    pub screen: Screen,
    /// The help overlay, when open. A layer over `screen`, never a replacement
    /// for it: see `docs/adr/0001-help-is-an-overlay-layer-not-a-screen.md`.
    pub help: Option<crate::tui::screens::help::HelpState>,
    pub should_quit: bool,
    pub space_cd_target: Option<PathBuf>,
    pub status_message: Option<String>,
    pub status_kind: StatusKind,
    pub status_message_set_at: Option<Instant>,

    // Background workspace loader
    pub ws_load_tx: Option<mpsc::SyncSender<LoadRequest>>,
    pub ws_result_rx: Option<mpsc::Receiver<LoadResult>>,
    pub ws_generation: u64,

    // Background sync worker (Syncing stage)
    pub sync_rx: Option<mpsc::Receiver<SyncProgress>>,
    // Relaxed ordering is sufficient: this flag guards no other shared state, so
    // there is no happens-before relationship to establish with the worker thread.
    pub sync_cancel: Option<Arc<AtomicBool>>,

    // Background git-ops worker (Running stage). Same shape as the sync worker.
    pub gitop_rx: Option<mpsc::Receiver<GitOpProgress>>,
    pub gitop_cancel: Option<Arc<AtomicBool>>,

    // Background worktree worker (Creating stage). Channel, flag, parameters
    // and counts travel together: `Some` IS "a run is in flight".
    pub create_job: Option<CreateJob>,

    // Debounce
    pub nav_pending: Option<Instant>,

    // Loading display
    pub ws_loading: bool,
    pub ws_loading_since: Option<Instant>,
    pub spinner_tick: u64,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = SpaceConfig::load()?;
        let workspaces = workspace::list_workspaces(&config.workspaces.dir)?;
        // Load repo cache; rescan if missing or stale
        let cache_path = SpaceConfig::cache_path();
        let repos_cache = crate::core::repo::load_cache(&cache_path, config.repos.cache_age_secs)
            .unwrap_or_else(|| {
                let found =
                    crate::core::repo::find_repos_in(&config.repos.roots, config.repos.max_depth);
                crate::core::repo::save_cache(&cache_path, &found).ok();
                found
            });

        // Spawn background workspace loader
        let (load_tx, load_rx) = mpsc::sync_channel::<LoadRequest>(32);
        let (result_tx, result_rx) = mpsc::sync_channel::<LoadResult>(32);

        std::thread::spawn(move || {
            while let Ok(mut req) = load_rx.recv() {
                // Drain to latest: skip superseded requests queued while we were busy
                while let Ok(newer) = load_rx.try_recv() {
                    req = newer;
                }
                // Always send a result so the app can clear ws_loading.
                // workspace is None when workspace_detail returns Err (e.g. directory
                // deleted, transient git error) — the app surfaces a status message.
                let workspace =
                    crate::core::workspace::workspace_detail(&req.ws_dir, &req.name).ok();
                let _ = result_tx.send(LoadResult {
                    generation: req.generation,
                    workspace,
                });
            }
        });

        let mut app = Self {
            config,
            workspaces,
            repos_cache,
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            diff_content_cache: HashMap::new(),
            file_mtime_cache: HashMap::new(),
            repo_index_mtime: HashMap::new(),
            cursor_row: 0,
            table_scroll_x: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            help: None,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_kind: StatusKind::Info,
            status_message_set_at: None,
            ws_load_tx: Some(load_tx),
            ws_result_rx: Some(result_rx),
            ws_generation: 0,
            sync_rx: None,
            sync_cancel: None,
            gitop_rx: None,
            gitop_cancel: None,
            create_job: None,
            nav_pending: None,
            ws_loading: false,
            ws_loading_since: None,
            spinner_tick: 0,
        };
        // Startup: show skeletons immediately and load in the background.
        // The first poll_background_result() call in run_loop will apply the result.
        if !app.workspaces.is_empty() {
            app.begin_workspace_load_immediate();
        }
        tracing::info!(version = env!("CARGO_PKG_VERSION"), "TUI started");
        Ok(app)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.selected_ws)
    }

    /// Build the flat list of rows for the repo table.
    ///
    /// Each expanded repo emits section headers (Conflicts / Unstaged / Staged)
    /// followed by the file entries in each group. Empty groups are omitted.
    ///
    /// # Invariant
    /// Every `SectionHeader` row is always immediately followed by at least one
    /// `File` row (empty sections are never emitted). The first and last rows of
    /// the list are therefore always `Repo` or `File` rows — never `SectionHeader`.
    /// `skip_headers` and `reposition_after_section_change` rely on this.
    pub fn flattened_rows(&self) -> Vec<RepoRow<'_>> {
        let repos = match self.selected_workspace() {
            Some(ws) => &ws.repos,
            None => return vec![],
        };
        let mut rows = Vec::new();
        for (i, repo) in repos.iter().enumerate() {
            let expanded = self.expanded_repos.contains(&i);
            rows.push(RepoRow::Repo {
                index: i,
                repo,
                expanded,
            });
            if expanded {
                if let Some(entries) = self.repo_file_cache.get(&i) {
                    use crate::core::git::FileStatus;
                    let conflicts: Vec<_> = entries
                        .iter()
                        .filter(|e| e.status == FileStatus::Conflicted)
                        .collect();
                    let unstaged: Vec<_> = entries
                        .iter()
                        .filter(|e| !e.staged && e.status != FileStatus::Conflicted)
                        .collect();
                    let staged: Vec<_> = entries
                        .iter()
                        .filter(|e| e.staged && e.status != FileStatus::Conflicted)
                        .collect();

                    // Detect partially-staged files: same path in both staged and unstaged groups
                    let partially_staged_paths: std::collections::HashSet<&str> = {
                        let staged_set: std::collections::HashSet<&str> =
                            staged.iter().map(|e| e.path.as_str()).collect();
                        unstaged
                            .iter()
                            .filter(|e| staged_set.contains(e.path.as_str()))
                            .map(|e| e.path.as_str())
                            .collect()
                    };

                    if !conflicts.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Conflicts",
                        });
                        for entry in &conflicts {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: false, // conflicts can't be partially staged
                            });
                        }
                    }
                    if !unstaged.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Unstaged",
                        });
                        for entry in &unstaged {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: partially_staged_paths
                                    .contains(entry.path.as_str()),
                            });
                        }
                    }
                    if !staged.is_empty() {
                        rows.push(RepoRow::SectionHeader {
                            repo_index: i,
                            label: "Staged",
                        });
                        for entry in &staged {
                            rows.push(RepoRow::File {
                                repo_index: i,
                                entry,
                                partially_staged: partially_staged_paths
                                    .contains(entry.path.as_str()),
                            });
                        }
                    }
                }
            }
        }
        rows
    }

    /// Return the repo index the cursor is on (whether on a Repo, SectionHeader, or File row).
    pub fn repo_index_for_cursor(&self) -> Option<usize> {
        match self.flattened_rows().get(self.cursor_row) {
            Some(RepoRow::Repo { index, .. }) => Some(*index),
            Some(RepoRow::SectionHeader { repo_index, .. }) => Some(*repo_index),
            Some(RepoRow::File { repo_index, .. }) => Some(*repo_index),
            None => None,
        }
    }

    pub fn load_selected_workspace_detail(&mut self) {
        if let Some(ws) = self.workspaces.get(self.selected_ws) {
            let name = ws.name.clone();
            match workspace::workspace_detail(&self.config.workspaces.dir, &name) {
                Ok(detail) => {
                    self.workspaces[self.selected_ws] = detail;
                }
                Err(_e) => {
                    // Keep shallow workspace entry; note error for user
                    self.set_status(
                        format!("Could not load '{}' detail", name),
                        StatusKind::Error,
                    );
                }
            }
        }
    }

    /// Reset all repo-pane state that is keyed to the current workspace.
    /// Must be called whenever `selected_ws` changes so stale rows/cursor
    /// positions from the previous workspace are never visible.
    pub fn reset_repo_pane_state(&mut self) {
        self.cursor_row = 0;
        self.table_scroll_x = 0;
        self.expanded_repos.clear();
        self.repo_file_cache.clear();
        self.diff_content_cache.clear();
        self.file_mtime_cache.clear();
        self.repo_index_mtime.clear();
    }

    /// Send a load request to the background worker. The worker will call
    /// `workspace_detail` and send the result back via `ws_result_rx`.
    fn fire_load_request(&mut self) {
        let Some(ws) = self.workspaces.get(self.selected_ws) else {
            return;
        };
        let req = LoadRequest {
            generation: self.ws_generation,
            ws_dir: self.config.workspaces.dir.clone(),
            name: ws.name.clone(),
        };
        tracing::info!(
            generation = self.ws_generation,
            repo_count = self
                .workspaces
                .get(self.selected_ws)
                .map(|w| w.repos.len())
                .unwrap_or(0),
            "background load requested"
        );
        if let Some(tx) = &self.ws_load_tx {
            match tx.try_send(req) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(_)) => {
                    // Channel is full (32 unprocessed requests). Reset nav_pending so
                    // check_debounce_timer retries on the next frame rather than
                    // leaving ws_loading stuck. Requires sustained rapid scrolling
                    // while git is very slow — rare, but must not freeze the UI.
                    self.nav_pending = Some(Instant::now() - Duration::from_millis(200));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    // Worker thread died — clear loading state so the spinner
                    // doesn't freeze. poll_background_result handles the same
                    // case via TryRecvError::Disconnected on the receive side.
                    tracing::error!("background loader thread disconnected on send");
                    self.ws_loading = false;
                    self.ws_loading_since = None;
                }
            }
        }
    }

    /// Called on workspace navigation (`j`/`k`). Populates the repos pane with
    /// skeletons immediately (fast, no git I/O), sets loading state, and starts
    /// the 150ms debounce timer. The background load fires once the user pauses.
    ///
    /// Increments `ws_generation` immediately to invalidate any in-flight result
    /// from the previous workspace — the new generation won't match until
    /// `check_debounce_timer` fires and sends a fresh request.
    pub fn begin_workspace_load(&mut self) {
        tracing::info!(ws_index = self.selected_ws, "workspace navigation");
        self.apply_skeleton_repos();
        self.ws_loading = true;
        self.ws_loading_since = Some(Instant::now());
        self.ws_generation += 1; // invalidate any in-flight result immediately
        self.nav_pending = Some(Instant::now());
    }

    /// Like `begin_workspace_load` but fires the background load immediately,
    /// without waiting for the debounce timer. Used for startup and explicit
    /// workspace jumps (`NavigateToWorkspace`) where there is no scroll gesture
    /// to debounce.
    pub fn begin_workspace_load_immediate(&mut self) {
        tracing::info!(ws_index = self.selected_ws, "workspace load (immediate)");
        self.apply_skeleton_repos();
        self.ws_loading = true;
        self.ws_loading_since = Some(Instant::now());
        self.nav_pending = None;
        self.ws_generation += 1;
        self.fire_load_request();
    }

    /// Populate the selected workspace's repos with lightweight skeleton entries
    /// (branch = "...") from a fast filesystem scan. No git operations.
    fn apply_skeleton_repos(&mut self) {
        let Some(ws) = self.workspaces.get(self.selected_ws) else {
            return;
        };
        let name = ws.name.clone();
        let skeletons =
            crate::core::workspace::workspace_repo_skeletons(&self.config.workspaces.dir, &name);
        if let Some(ws) = self.workspaces.get_mut(self.selected_ws) {
            ws.repos = skeletons;
        }
    }

    /// Apply a completed background result. Only accepted if the generation matches
    /// the current `ws_generation` (stale results from superseded requests are discarded).
    pub fn apply_workspace_result(&mut self, result: LoadResult) {
        tracing::debug!(
            result_gen = result.generation,
            current_gen = self.ws_generation,
            "background result received"
        );
        if result.generation != self.ws_generation {
            tracing::debug!(
                result_gen = result.generation,
                current_gen = self.ws_generation,
                "background result discarded (stale)"
            );
            return;
        }
        // Always clear loading state — the request for this generation is done
        // regardless of whether it succeeded or failed.
        self.ws_loading = false;
        self.ws_loading_since = None;

        match result.workspace {
            Some(ws) => {
                // Match by workspace name to find the correct slot
                if let Some(idx) = self.workspaces.iter().position(|w| w.name == ws.name) {
                    self.workspaces[idx] = ws;
                }
                tracing::info!(generation = self.ws_generation, "background load applied");
            }
            None => {
                // workspace_detail returned an error (deleted directory, transient git failure)
                tracing::warn!(
                    generation = self.ws_generation,
                    ws_index = self.selected_ws,
                    "background load failed"
                );
                self.set_status("Could not load workspace detail", StatusKind::Error);
            }
        }
    }

    /// Poll the result channel once. Called every frame in `run_loop`. Non-blocking.
    pub fn poll_background_result(&mut self) {
        // Take the receiver temporarily to avoid borrow issues with &mut self
        let rx = self.ws_result_rx.take();
        if let Some(ref r) = rx {
            match r.try_recv() {
                Ok(result) => {
                    self.ws_result_rx = rx;
                    self.apply_workspace_result(result);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Worker thread died — clear loading state so the UI does not
                    // freeze with a permanent spinner. Channel is dead; do not restore.
                    tracing::error!("background loader thread disconnected unexpectedly");
                    self.ws_loading = false;
                    self.ws_loading_since = None;
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // Normal — no result yet, keep waiting.
                }
            }
        }
        self.ws_result_rx = rx;
    }

    /// Check the debounce timer. If 150ms has elapsed since the last navigation key
    /// and a load is pending, fire the background load request.
    ///
    /// # Generation counter protocol
    ///
    /// `ws_generation` is incremented in **two** places by design:
    ///
    /// 1. `begin_workspace_load` — increments immediately on navigation to
    ///    invalidate any in-flight result from the *previous* workspace.
    /// 2. `check_debounce_timer` (here) — increments again just before firing
    ///    the actual load request, so the result the worker sends back carries
    ///    the generation value the app will be waiting for.
    ///
    /// Both increments are required. Removing either one opens a race window.
    pub fn check_debounce_timer(&mut self) {
        if let Some(t) = self.nav_pending {
            if t.elapsed() > Duration::from_millis(150) {
                self.nav_pending = None;
                self.ws_generation += 1; // see generation counter protocol above
                tracing::debug!(generation = self.ws_generation, "debounce timer fired");
                self.fire_load_request();
            }
        }
    }

    /// Rescan the configured roots, save the cache and replace `repos_cache`.
    /// Shared by the dashboard `r` and the picker's Ctrl-R; it does nothing
    /// else, so callers own any cursor or diff-cache resets of their own.
    pub fn rescan_repo_list(&mut self) -> RescanSummary {
        let roots = self.config.repos.roots.clone();
        let depth = self.config.repos.max_depth;
        let repos = crate::core::repo::find_repos_in(&roots, depth);
        let _ = crate::core::repo::save_cache(&SpaceConfig::cache_path(), &repos);
        let previous: std::collections::HashSet<&PathBuf> = self.repos_cache.iter().collect();
        let new_count = repos.iter().filter(|p| !previous.contains(p)).count();
        let total = repos.len();
        self.repos_cache = repos;
        RescanSummary { total, new_count }
    }

    /// Ctrl-R inside a repo picker: rescan the repo list and rebuild the open
    /// picker from it, keeping the user's place. The add flow reapplies its
    /// exclusion of repos already in the space; if that space is gone the flow
    /// leaves with an error notice. The dashboard cursor is never touched.
    fn rescan_open_picker(&mut self) {
        let summary = self.rescan_repo_list();
        let missing = match &mut self.screen {
            Screen::CreateWorkspace(st) => st.replace_repo_list(self.repos_cache.clone()),
            Screen::AddRepos(st) => {
                let ws = self.workspaces.iter().find(|w| w.name == st.workspace_name);
                match ws {
                    Some(ws) => st.replace_repo_list(addable_repos(&self.repos_cache, ws)),
                    None => {
                        let msg = format!("Space '{}' no longer exists", st.workspace_name);
                        self.screen = Screen::Dashboard;
                        self.set_status(msg, StatusKind::Error);
                        return;
                    }
                }
            }
            _ => return,
        };
        let (msg, kind) = rescan_notice(&summary, missing);
        self.set_status(msg, kind);
    }

    /// Fetch file diffs for all repos in the selected workspace and populate
    /// `repo_file_cache`. Called on explicit refresh (`RefreshRepos`) and when
    /// a repo is expanded (`ToggleRepoExpand`). Not called on workspace navigation
    /// — file diffs are deferred until the user actually expands a repo.
    pub fn refresh_file_diff_cache(&mut self) {
        self.repo_file_cache.clear();
        self.diff_content_cache.clear();
        self.file_mtime_cache.clear();
        let repo_paths: Vec<(usize, std::path::PathBuf)> = self
            .selected_workspace()
            .map(|ws| {
                ws.repos
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (i, r.path.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let mut failures = 0usize;
        for (idx, path) in repo_paths {
            match crate::core::git::file_diff(&path) {
                Ok(entries) => {
                    self.repo_file_cache.insert(idx, entries);
                    // Record .git/index mtime for staleness detection
                    if let Some(mtime) = crate::core::git::git_index_mtime(&path) {
                        self.repo_index_mtime.insert(idx, mtime);
                    }
                }
                Err(_) => {
                    failures += 1;
                }
            }
        }
        if failures > 0 {
            self.set_status(
                format!("Diff failed for {} repo(s)", failures),
                StatusKind::Error,
            );
        }
    }

    /// Stage or unstage a single file, invalidate caches, and set a status message.
    /// Shared by `Message::StageFile` (dashboard) and `ScreenAction::StageFile` (diff overlay).
    fn do_stage(
        &mut self,
        repo_index: usize,
        repo_path: &std::path::Path,
        path: &str,
        currently_staged: bool,
    ) {
        // Block staging on conflicted files
        if let Some(entries) = self.repo_file_cache.get(&repo_index) {
            if entries
                .iter()
                .any(|e| e.path == path && e.status == crate::core::git::FileStatus::Conflicted)
            {
                self.set_status(
                    "Cannot stage conflicted file: resolve conflicts first".to_string(),
                    StatusKind::Warning,
                );
                return;
            }
        }

        let result = if currently_staged {
            crate::core::git::unstage_file(repo_path, path)
        } else {
            crate::core::git::stage_file(repo_path, path)
        };
        match result {
            Ok(()) => {
                // Invalidate diff content cache entries for this repo
                self.diff_content_cache
                    .retain(|key, _| key.repo_index != repo_index);
                self.file_mtime_cache
                    .retain(|key, _| key.repo_index != repo_index);
                // Re-fetch file list for this repo
                match crate::core::git::file_diff(repo_path) {
                    Ok(entries) => {
                        self.repo_file_cache.insert(repo_index, entries);
                    }
                    Err(_) => {
                        // Keep stale cache entry — better than empty UI
                        let verb = if currently_staged {
                            "Unstaged"
                        } else {
                            "Staged"
                        };
                        self.set_status(
                            format!("{} {} -- refresh failed, press r", verb, path),
                            StatusKind::Warning,
                        );
                        return;
                    }
                }
                let verb = if currently_staged {
                    "Unstaged"
                } else {
                    "Staged"
                };
                self.set_status(format!("{} {}", verb, path), StatusKind::Success);
            }
            Err(err) => {
                let verb = if currently_staged { "Unstage" } else { "Stage" };
                self.set_status(format!("{} failed: {}", verb, err), StatusKind::Error);
            }
        }
    }

    /// Open the help overlay over the current screen, scrolled to the group
    /// documenting that screen.
    pub fn open_help(&mut self) {
        let group = crate::tui::screens::help::landing_group(&self.screen, self.focus);
        self.help = Some(crate::tui::screens::help::HelpState::opening_at(group));
    }

    fn set_status(&mut self, msg: impl Into<String>, kind: StatusKind) {
        self.status_message = Some(msg.into());
        self.status_kind = kind;
        self.status_message_set_at = Some(Instant::now());
    }

    fn clear_status(&mut self) {
        self.status_message = None;
        self.status_message_set_at = None;
    }

    fn expire_status_message(&mut self, now: Instant) {
        if let Some(set_at) = self.status_message_set_at {
            if now.duration_since(set_at) >= STATUS_MESSAGE_TTL {
                self.clear_status();
            }
        }
    }

    /// Refresh workspace list when leaving the Creating stage.
    /// Catches partially-created worktrees from error scenarios.
    fn refresh_if_leaving_creating_stage(&mut self) {
        let is_creating = matches!(
            &self.screen,
            Screen::CreateWorkspace(st) if st.stage == crate::tui::screens::create::CreateStage::Creating
        ) || matches!(
            &self.screen,
            Screen::AddRepos(st) if st.stage == crate::tui::screens::add::AddStage::Creating
        );
        if is_creating {
            if let Ok(ws) = crate::core::workspace::list_workspaces(&self.config.workspaces.dir) {
                self.workspaces = ws;
                self.selected_ws = 0;
                self.reset_repo_pane_state();
                // Intentionally synchronous: worktree deletion is infrequent and
                // the user expects up-to-date data before seeing the dashboard.
                self.load_selected_workspace_detail();
            }
        }
    }

    /// Whether the Creating stage's worker is running.
    pub fn creating_in_flight(&self) -> bool {
        self.create_job.is_some()
    }

    /// How far the Creating run has got: `(in_flight, started, total)`.
    /// All zero with no run, which is what the footer renders as done.
    pub fn creating_progress(&self) -> (bool, usize, usize) {
        match &self.create_job {
            Some(job) => (true, job.started, job.params.repos.len()),
            None => (false, 0, 0),
        }
    }

    /// The Creating log of the active screen, if it is in the Creating stage:
    /// its lines, its error slot and its scroll state.
    ///
    /// Every screen mutation in `poll_create_result` goes through this, so a
    /// late message can only ever touch a screen that is actually showing the
    /// log it describes.
    fn creating_mut(
        &mut self,
    ) -> Option<(
        &mut Vec<String>,
        &mut Option<String>,
        &mut crate::tui::screens::sync_report::LogView,
    )> {
        match &mut self.screen {
            Screen::CreateWorkspace(st)
                if st.stage == crate::tui::screens::create::CreateStage::Creating =>
            {
                Some((&mut st.progress, &mut st.error, &mut st.log_view))
            }
            Screen::AddRepos(st) if st.stage == crate::tui::screens::add::AddStage::Creating => {
                Some((&mut st.progress, &mut st.error, &mut st.log_view))
            }
            _ => None,
        }
    }

    /// The Creating stage's success path: refresh the space list, select the
    /// space this run built BY NAME, and return to the dashboard.
    fn finish_create_run(&mut self, params: &crate::tui::actions::WorktreeParams) {
        if let Ok(ws_list) = crate::core::workspace::list_workspaces(&params.workspace_dir) {
            self.workspaces = ws_list;
            if let Some(idx) = self
                .workspaces
                .iter()
                .position(|w| w.name == params.workspace_name)
            {
                self.selected_ws = idx;
            }
            self.reset_repo_pane_state();
            // Intentionally synchronous: worktree creation is infrequent and
            // the user expects the new workspace to appear fully loaded.
            self.load_selected_workspace_detail();
        }
        let verb = if params.is_new {
            "Created"
        } else {
            "Added repos to"
        };
        self.screen = Screen::Dashboard;
        self.set_status(
            format!("{} workspace '{}'", verb, params.workspace_name),
            StatusKind::Success,
        );
    }

    /// Stop the Creating run at the worker's next boundary and leave for the
    /// dashboard, with the partially created space selected and named.
    ///
    /// The count is creations the UI confirmed, and it can undercount by one.
    /// Cancellation is boundary-only, so an in-flight `git worktree add`
    /// finishes AFTER this message is written and that repo can appear on the
    /// next refresh. Making the count exact would mean waiting for the child,
    /// which is the freeze this whole change removes. The message also expires
    /// after `STATUS_MESSAGE_TTL` (five seconds), while the late repo may
    /// arrive after that.
    ///
    /// The recovery offered is "add the rest", not "run it again", and that is
    /// verified rather than assumed: on git 2.50.1, retrying the same space
    /// name makes every repo that already succeeded fail with
    /// `fatal: '<path>' already exists`, and because the flow does not stop on
    /// a generic failure it creates the missing repos anyway and then reports
    /// the whole run as failed. The space ends up complete and the app says it
    /// failed. The add path is clean because a repo that was never created has
    /// neither the target directory nor the branch.
    ///
    /// "Leaves at once" has one asterisk, and it is not the network wait this
    /// change removes. The refresh below calls `load_selected_workspace_detail`
    /// synchronously, which opens every repo in the partial space with git2 to
    /// read its status. That is local work bounded by the repos already
    /// created, the same call every dashboard navigation makes, and it is what
    /// makes the space appear correctly on the way out; it is not bounded by
    /// `UNATTENDED_FETCH_TIMEOUT` the way the old freeze was.
    ///
    /// Nothing is cleaned up. `create_worktree_cancellable` runs
    /// `create_dir_all` before the fetch and `list_workspaces` lists any
    /// directory, so cancelling before any worktree exists still leaves an
    /// empty space on the dashboard. That is left deliberately, for
    /// CONSISTENCY: the error path produces the identical empty directory, and
    /// a cleanup that fired for one and not the other would be the
    /// inconsistency. Cancel means stop, not undo.
    fn cancel_creating(&mut self) {
        // Drain first. `run_loop` drains, draws, then waits up to 16ms for a
        // key, so the worker's terminal message can land inside that wait and
        // still be undrained when Esc is read. The user is pressing Esc
        // against the screen they were shown, so leaving is right either way,
        // but a run that had already finished must be reported as finished,
        // not as a cancellation with a count frozen at the last drain. Without
        // this a fully successful run says "Stopped creating 'ws' after 4 of 5
        // repos", which is a success reported as a failure, and an
        // `AlreadyCheckedOut` stop loses its bounce to the strategy picker.
        self.poll_create_result();
        let job = match self.create_job.take() {
            Some(job) => job,
            None => {
                // The run ended in this very frame. If the drain left the
                // Creating stage up it did so because a repo failed, and Esc
                // still means leave. If it moved the screen itself (the
                // dashboard on success, the strategy picker on a bounce) this
                // is a no-op: the key is consumed rather than replayed into a
                // screen the user has not seen yet, which is the corruption
                // `run_loop`'s own startup drain exists to prevent.
                self.leave_failed_creating();
                return;
            }
        };
        job.cancel.store(true, Ordering::Relaxed);

        if let Ok(ws) = crate::core::workspace::list_workspaces(&job.params.workspace_dir) {
            self.workspaces = ws;
        }
        self.selected_ws = self
            .workspaces
            .iter()
            .position(|w| w.name == job.params.workspace_name)
            .unwrap_or(0);
        self.reset_repo_pane_state();
        self.load_selected_workspace_detail();
        self.screen = Screen::Dashboard;

        let verb = if job.params.is_new {
            "Stopped creating"
        } else {
            "Stopped adding to"
        };
        self.set_status(
            format!(
                "{} '{}' after {} of {} repos. Press a to add the rest.",
                verb,
                job.params.workspace_name,
                job.created,
                job.params.repos.len()
            ),
            StatusKind::Warning,
        );
    }

    /// Leave a finished-but-failed Creating stage for the dashboard, exactly
    /// as the stage's own Esc does. A no-op unless that stage is still up.
    ///
    /// Only reachable in the frame where the run ended: `Done` with a failed
    /// repo, or a worker that died, both of which keep the stage up so the log
    /// can be read.
    ///
    /// Residual, deliberately not fixed here: `Enter` in that same window is
    /// swallowed once, because the stage ignores it while the job still looks
    /// live. The screen is correct on the next frame and a second press works.
    /// The asymmetry is intended: Esc's misfire wrote a false message about
    /// durable state, while Enter's costs one keystroke.
    fn leave_failed_creating(&mut self) {
        let (verb, err) = match &self.screen {
            Screen::CreateWorkspace(st)
                if st.stage == crate::tui::screens::create::CreateStage::Creating =>
            {
                ("Create", st.error.clone())
            }
            Screen::AddRepos(st) if st.stage == crate::tui::screens::add::AddStage::Creating => {
                ("Add", st.error.clone())
            }
            _ => return,
        };
        self.refresh_if_leaving_creating_stage();
        self.screen = Screen::Dashboard;
        if let Some(err) = err {
            self.set_status(format!("{} failed: {}", verb, err), StatusKind::Error);
        }
    }

    /// Poll the Creating worker channel once per frame. Non-blocking.
    ///
    /// Applies `Started` and `Finished` to the active screen's log, bounces to
    /// the branch-strategy picker on `Stopped`, and leaves for the dashboard on
    /// `Done` unless a repo failed, in which case the stage stays up so the log
    /// can be read. `Finished` is purely a logging message: no control flow
    /// hangs off it, only off `Stopped`, `Done` and a disconnect.
    ///
    /// It deliberately differs from `poll_sync_result` in what the screen may
    /// do. That poller treats "the screen is not the one I expect" as a reason
    /// to cancel the worker and drop the receiver, which is what made a
    /// `Screen::Help { return_to }` wrapper unsafe in PR #29: wrapping a
    /// mid-sync screen made the gate false and cancelled the sync it was
    /// displaying. Here the screen gates only whether messages are drained this
    /// frame; the JOB is what says a run exists. Dropping the receiver would be
    /// the same hazard one step removed, because the worker's next send would
    /// then fail and it would stop. So an unrecognised screen leaves the job
    /// untouched and the buffered messages queue (the channel holds 64, far
    /// more than any realistic repo count) until the log comes back.
    ///
    /// That is safe because every exit from the Creating stage cancels and
    /// drops the job explicitly: `CancelCreating`, the `Stopped` bounce,
    /// re-entry into `ExecuteWorktreeFlow`, and completion. The residual is
    /// that a screen which never returns to the Creating stage would leave the
    /// job in place until the App is dropped, which is unreachable today and
    /// in any case a far better failure than a cancelled run.
    pub fn poll_create_result(&mut self) {
        let is_creating = self.creating_mut().is_some();
        if !is_creating || self.create_job.is_none() {
            return;
        }
        loop {
            let recv = match &self.create_job {
                Some(job) => job.rx.try_recv(),
                None => return,
            };
            match recv {
                Ok(CreateProgress::Started { index }) => {
                    let lines = match &mut self.create_job {
                        Some(job) => {
                            job.started = index + 1;
                            let verb = if job.params.is_new {
                                "Creating"
                            } else {
                                "Adding"
                            };
                            let mut lines = vec![format!(
                                "{} worktree for {}...",
                                verb,
                                repo_label(&job.params, index)
                            )];
                            // Unlike a skip for freshness, this one is worth
                            // saying: these refs are of unknown age.
                            if job
                                .params
                                .repos
                                .get(index)
                                .is_some_and(|p| job.params.skipped_after_timeout(p))
                            {
                                lines.push(format!(
                                    "  {}",
                                    crate::tui::screens::sync_report::SKIPPED_AFTER_TIMEOUT_NOTE
                                ));
                            }
                            lines
                        }
                        None => return,
                    };
                    if let Some((progress, _, _)) = self.creating_mut() {
                        progress.extend(lines);
                    }
                }
                Ok(CreateProgress::Finished {
                    index,
                    fetch,
                    created,
                }) => {
                    let (lines, failure) = match &mut self.create_job {
                        Some(job) => {
                            job.finished += 1;
                            let name = repo_label(&job.params, index);
                            let mut lines = Vec::new();
                            // The fetch line goes in before the outcome, so a
                            // `git worktree add` that then refused on stale
                            // refs still carries the reason.
                            if let Some(note) = fetch
                                .as_ref()
                                .and_then(crate::tui::screens::sync_report::creating_fetch_note)
                            {
                                lines.push(format!("  {}", note));
                            }
                            let failure = match created {
                                Ok(()) => {
                                    job.created += 1;
                                    lines.push(format!("  \u{2713} {}", name));
                                    None
                                }
                                Err(e) => {
                                    lines.push(format!("  \u{2717} {}: {}", name, e));
                                    Some(format!("Failed: {}", e))
                                }
                            };
                            (lines, failure)
                        }
                        None => return,
                    };
                    if let Some((progress, error, _)) = self.creating_mut() {
                        progress.extend(lines);
                        if failure.is_some() {
                            *error = failure;
                        }
                    }
                }
                Ok(CreateProgress::Stopped(CreateStop::AlreadyCheckedOut { index })) => {
                    let job = match self.create_job.take() {
                        Some(job) => job,
                        None => return,
                    };
                    job.cancel.store(true, Ordering::Relaxed);
                    let msg = format!(
                        "'{}' is already checked out — pick a different strategy",
                        repo_label(&job.params, index)
                    );
                    match &mut self.screen {
                        Screen::CreateWorkspace(st) => {
                            st.stage = crate::tui::screens::create::CreateStage::PickBranchStrategy;
                            st.progress.clear();
                            st.error = Some(msg);
                        }
                        Screen::AddRepos(st) => {
                            st.stage = crate::tui::screens::add::AddStage::PickBranchStrategy;
                            st.progress.clear();
                            st.error = Some(msg);
                        }
                        _ => {}
                    }
                    return;
                }
                Ok(CreateProgress::Done) => {
                    let job = match self.create_job.take() {
                        Some(job) => job,
                        None => return,
                    };
                    self.finish_create_run_unless_failed(&job.params);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let job = match self.create_job.take() {
                        Some(job) => job,
                        None => return,
                    };
                    if job.finished < job.params.repos.len() {
                        // The worker died without saying so: a cancelled one
                        // stops after a `Stopped` or without a job at all.
                        if let Some((progress, error, _)) = self.creating_mut() {
                            progress.push(
                                "  \u{2717} the worktree worker stopped unexpectedly".to_string(),
                            );
                            *error = Some("the worktree worker stopped unexpectedly".to_string());
                        }
                    } else {
                        self.finish_create_run_unless_failed(&job.params);
                    }
                    return;
                }
            }
        }
    }

    /// Leave for the dashboard, unless a repo failed: then the Creating stage
    /// stays up so the log can be read (Esc/Enter leaves with the error).
    fn finish_create_run_unless_failed(&mut self, params: &crate::tui::actions::WorktreeParams) {
        let failed = self
            .creating_mut()
            .map(|(_, error, _)| error.is_some())
            .unwrap_or(false);
        if !failed {
            self.finish_create_run(params);
        }
    }

    /// Load recent branches for the first selected repo and advance the active
    /// screen from Syncing to PickBranchStrategy.
    fn advance_to_branch_strategy(&mut self) {
        let repo_path = match &self.screen {
            Screen::CreateWorkspace(st) => st.selected_repos.first().cloned(),
            Screen::AddRepos(st) => st.selected_repos.first().cloned(),
            _ => return,
        };
        let recent = repo_path
            .as_ref()
            .map(|p| crate::core::git::recent_branches(p, 5))
            .unwrap_or_default();
        match &mut self.screen {
            Screen::CreateWorkspace(st) => {
                st.recent_branches = recent;
                st.branch_strategy_idx = 0;
                st.progress.clear();
                st.stage = crate::tui::screens::create::CreateStage::PickBranchStrategy;
            }
            Screen::AddRepos(st) => {
                st.recent_branches = recent;
                st.branch_strategy_idx = 0;
                st.progress.clear();
                st.stage = crate::tui::screens::add::AddStage::PickBranchStrategy;
            }
            _ => {}
        }
    }

    /// The sync report of the active screen, if it is in the Syncing stage.
    fn sync_report_mut(&mut self) -> Option<&mut crate::tui::screens::sync_report::SyncReport> {
        match &mut self.screen {
            Screen::CreateWorkspace(st)
                if st.stage == crate::tui::screens::create::CreateStage::Syncing =>
            {
                Some(&mut st.report)
            }
            Screen::AddRepos(st) if st.stage == crate::tui::screens::add::AddStage::Syncing => {
                Some(&mut st.report)
            }
            _ => None,
        }
    }

    /// Poll the sync worker channel once per frame. Non-blocking.
    ///
    /// Applies `Started` and `Finished` messages to the active screen's sync
    /// report. `Done` finishes the report and drops the channel; the screen
    /// stays on the report until the user presses Enter
    /// (`ScreenAction::ContinueFromSyncReport`). A disconnect without `Done`
    /// (the worker panicked or was dropped) is treated as `Done`: unfinished
    /// rows become `not synced`. Drops `sync_rx` whenever the screen is no
    /// longer in the Syncing stage (e.g. the user pressed Esc).
    pub fn poll_sync_result(&mut self) {
        let is_syncing = matches!(
            &self.screen,
            Screen::CreateWorkspace(st)
                if st.stage == crate::tui::screens::create::CreateStage::Syncing
        ) || matches!(
            &self.screen,
            Screen::AddRepos(st)
                if st.stage == crate::tui::screens::add::AddStage::Syncing
        );
        if !is_syncing {
            // Signal cancellation so the worker stops before its next git call.
            // Dropping the receiver below is a backstop: if the worker is mid-repo when we
            // cancel, its next tx.send() returns Err immediately (a dropped receiver on a
            // sync_channel does not block the sender), so the thread still exits cleanly.
            // The cancelled worker finishes its in-flight git call in the background and
            // starts nothing new. A restart on the same repo while that call is still
            // running can hit git's own lock error (`cannot lock ref`), which the report
            // shows as a `fetch failed` row (or a skipped branch when the colliding call
            // is a `branch -f`): the accepted residual of design item 1.5 (GitHub issue
            // #24 item 3).
            if let Some(c) = &self.sync_cancel {
                c.store(true, Ordering::Relaxed);
            }
            self.sync_cancel = None;
            self.sync_rx = None;
            return;
        }
        let rx = self.sync_rx.take();
        if let Some(ref r) = rx {
            loop {
                match r.try_recv() {
                    Ok(SyncProgress::Started { index }) => {
                        if let Some(report) = self.sync_report_mut() {
                            report.started(index);
                        }
                    }
                    Ok(SyncProgress::Finished { index, outcome }) => {
                        if let Some(report) = self.sync_report_mut() {
                            report.finished(index, outcome);
                        }
                    }
                    Ok(SyncProgress::Done) | Err(mpsc::TryRecvError::Disconnected) => {
                        if let Some(report) = self.sync_report_mut() {
                            report.finish();
                        }
                        self.sync_cancel = None;
                        return;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        self.sync_rx = rx;
    }

    /// Poll the git-ops worker channel once per frame. Non-blocking.
    ///
    /// Mirrors `poll_sync_result`: when the screen is no longer in the GitOps
    /// Running stage it signals cancellation and drops the receiver. While
    /// Running it drains `Line` output into the overlay buffer, records the
    /// success/failure on `Done`, and fires the auto-close timer on success.
    pub fn poll_gitop_result(&mut self) {
        let is_running = matches!(
            &self.screen,
            Screen::GitOps(st)
                if st.stage == crate::tui::screens::gitops::GitOpsStage::Running
        );
        if !is_running {
            // Signal cancellation so the worker stops at its next boundary, then
            // drop the receiver (a dropped sync_channel receiver makes the
            // worker's next send return Err, so the thread still exits cleanly).
            if let Some(c) = &self.gitop_cancel {
                c.store(true, Ordering::Relaxed);
            }
            self.gitop_cancel = None;
            self.gitop_rx = None;
            return;
        }

        // Drain any worker output into the overlay's buffer.
        let rx = self.gitop_rx.take();
        if let Some(ref r) = rx {
            loop {
                match r.try_recv() {
                    Ok(GitOpProgress::Line(s)) => {
                        if let Screen::GitOps(st) = &mut self.screen {
                            st.output.push(s);
                        }
                    }
                    Ok(GitOpProgress::Done { success }) => {
                        self.gitop_cancel = None;
                        if let Screen::GitOps(st) = &mut self.screen {
                            st.finished = Some(success);
                            // Success gets out of the way after ~3s; failure stays
                            // open so the error output remains readable.
                            st.close_at = if success {
                                Some(Instant::now() + Duration::from_secs(3))
                            } else {
                                None
                            };
                        }
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // Worker gone (e.g. cancelled) without a Done — stop polling.
                        self.gitop_cancel = None;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        self.gitop_rx = rx;

        // Auto-close: once the success timer elapses, return to the dashboard
        // and refresh the repo pane so newly fetched refs are reflected.
        let should_close = matches!(
            &self.screen,
            Screen::GitOps(st)
                if st.close_at.map(|t| Instant::now() >= t).unwrap_or(false)
        );
        if should_close {
            self.reset_repo_pane_state();
            self.load_selected_workspace_detail();
            self.screen = Screen::Dashboard;
            self.gitop_rx = None;
            self.gitop_cancel = None;
        }
    }

    fn process_action(&mut self, action: crate::tui::actions::ScreenAction) {
        use crate::tui::actions::ScreenAction;
        match action {
            ScreenAction::Continue => {}
            ScreenAction::OpenHelp => {
                self.open_help();
            }
            ScreenAction::Back => {
                // Refresh workspaces when leaving Creating stage (catches partial creates)
                self.refresh_if_leaving_creating_stage();
                // Closing a *successful* git op early (Esc before the ~3s
                // auto-close) must leave the same refreshed repo pane the
                // auto-close path produces — otherwise the dashboard shows
                // stale ahead/behind/file state.
                let leaving_successful_gitop = matches!(
                    &self.screen,
                    Screen::GitOps(st)
                        if st.stage == crate::tui::screens::gitops::GitOpsStage::Running
                            && st.finished == Some(true)
                );
                if leaving_successful_gitop {
                    self.reset_repo_pane_state();
                    self.load_selected_workspace_detail();
                }
                self.screen = Screen::Dashboard;
            }
            ScreenAction::BackWithStatus(msg, kind) => {
                self.refresh_if_leaving_creating_stage();
                self.screen = Screen::Dashboard;
                self.set_status(msg, kind);
            }
            ScreenAction::RescanRepoList => {
                self.rescan_open_picker();
            }
            ScreenAction::CdAndQuit(path) => {
                self.space_cd_target = Some(path);
                self.should_quit = true;
            }
            ScreenAction::DeleteWorkspace { name, force } => {
                let ws_dir = self.config.workspaces.dir.clone();
                match crate::core::workspace::remove_workspace(&ws_dir, &name, force) {
                    Ok(()) => {
                        if let Ok(ws) = crate::core::workspace::list_workspaces(&ws_dir) {
                            self.workspaces = ws;
                            self.selected_ws = 0;
                        }
                        self.reset_repo_pane_state();
                        // Intentionally synchronous: workspace deletion is infrequent
                        // and the user expects the dashboard to reflect the new state.
                        self.load_selected_workspace_detail();
                        self.screen = Screen::Dashboard;
                        self.set_status(
                            format!("Deleted workspace '{}'", name),
                            StatusKind::Success,
                        );
                    }
                    Err(e) => {
                        self.screen = Screen::Dashboard;
                        self.set_status(format!("Delete failed: {}", e), StatusKind::Error);
                    }
                }
            }
            ScreenAction::ExecuteWorktreeFlow(params) => {
                // Clear the log on whichever screen is active. A screen in
                // neither flow has nowhere to report, so nothing is started.
                match self.creating_mut() {
                    Some((progress, error, log_view)) => {
                        progress.clear();
                        log_view.reset();
                        *error = None;
                    }
                    None => return,
                }
                // Cancel any worker still live from a previous attempt before
                // dropping its handle, so it stops at its next boundary rather
                // than creating worktrees for a run the user has replaced.
                if let Some(old) = &self.create_job {
                    old.cancel.store(true, Ordering::Relaxed);
                }
                let (tx, rx) = mpsc::sync_channel::<CreateProgress>(64);
                let cancel = Arc::new(AtomicBool::new(false));
                self.create_job = Some(CreateJob {
                    rx,
                    cancel: cancel.clone(),
                    params: params.clone(),
                    started: 0,
                    finished: 0,
                    created: 0,
                });
                std::thread::spawn(move || run_create_worker(params, tx, cancel));
            }
            ScreenAction::CancelCreating => {
                self.cancel_creating();
            }
            ScreenAction::ContinueFromSyncReport => {
                self.advance_to_branch_strategy();
            }
            ScreenAction::ExecuteSyncFlow(repos) => {
                // Cancel any worker still live from a previous sync before dropping its
                // handle, so it stops before its next git call rather than running on
                // untracked. Its in-flight call still finishes in the background; if this
                // worker reaches the same repo first, git's lock error surfaces as a
                // `fetch failed` row (design item 1.5, GitHub issue #24 item 3).
                if let Some(old) = &self.sync_cancel {
                    old.store(true, Ordering::Relaxed);
                }
                let (tx, rx) = mpsc::sync_channel::<SyncProgress>(64);
                let cancel = Arc::new(AtomicBool::new(false));
                self.sync_rx = Some(rx);
                self.sync_cancel = Some(cancel.clone());
                std::thread::spawn(move || run_sync_worker(repos, tx, cancel));
            }
            ScreenAction::ExecuteGitOp { repo_path, op } => {
                // Cancel any worker still live from a previous op before dropping
                // its handle, so it stops at its boundary rather than running on.
                if let Some(old) = &self.gitop_cancel {
                    old.store(true, Ordering::Relaxed);
                }
                let (tx, rx) = mpsc::sync_channel::<GitOpProgress>(64);
                let cancel = Arc::new(AtomicBool::new(false));
                self.gitop_rx = Some(rx);
                self.gitop_cancel = Some(cancel.clone());
                std::thread::spawn(move || run_gitop_worker(repo_path, op, tx, cancel));
            }
            ScreenAction::CommitRepo { repo_path, message } => {
                // Synchronous local op: commit, then refresh the repo pane. On
                // failure keep the overlay in the Committing stage with an inline
                // error so the user can fix the message (or staging) and retry.
                let result = crate::core::workspace::commit_repo(&repo_path, &message);
                if result.success {
                    self.reset_repo_pane_state();
                    self.load_selected_workspace_detail();
                    self.screen = Screen::Dashboard;
                    self.set_status("Committed", StatusKind::Success);
                } else if let Screen::GitOps(st) = &mut self.screen {
                    st.status = Some(format!("Commit failed: {}", result.message));
                }
            }
            ScreenAction::SaveConfig(new_config) => {
                self.config = new_config;
                self.screen = Screen::Dashboard;
                self.set_status("Config saved", StatusKind::Success);
            }
            ScreenAction::NavigateToWorkspace(repo_name) => {
                self.screen = Screen::Dashboard;
                let found_idx = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.repos.iter().any(|r| r.name == repo_name));
                if let Some(idx) = found_idx {
                    self.selected_ws = idx;
                    self.selected_repo = 0;
                    self.reset_repo_pane_state();
                    self.begin_workspace_load_immediate();
                } else {
                    self.set_status(
                        "Not in any workspace — use 'c' to create one",
                        StatusKind::Info,
                    );
                }
            }
            ScreenAction::SelectWorkspace(idx) => {
                self.screen = Screen::Dashboard;
                // Re-selecting the current space keeps expanded repos and the cursor.
                if idx == self.selected_ws || idx >= self.workspaces.len() {
                    return;
                }
                self.selected_ws = idx;
                self.selected_repo = 0;
                self.reset_repo_pane_state();
                self.begin_workspace_load_immediate();
            }
            ScreenAction::StageFile {
                repo_index,
                repo_path,
                path,
                currently_staged,
            } => {
                self.do_stage(repo_index, &repo_path, &path, currently_staged);
                self.screen = Screen::Dashboard;
                // Staging refetches the repo's file list, so the row the cursor
                // was on may be gone. `Message::StageFile` repositions for the
                // same reason; without this the cursor is left indexing rows
                // that no longer exist.
                let rows = self.flattened_rows();
                self.cursor_row = reposition_after_section_change(&rows, self.cursor_row);
            }
            ScreenAction::SwitchRepoBranch {
                repo_path,
                branch,
                new_branch,
            } => {
                let repo_name = repo_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "?".to_string());

                match crate::core::workspace::switch_worktree_branch(
                    &repo_path, &branch, new_branch,
                ) {
                    Ok(()) => {
                        self.reset_repo_pane_state();
                        self.load_selected_workspace_detail();
                        self.screen = Screen::Dashboard;
                        self.set_status(
                            format!("Switched {} to {}", repo_name, branch),
                            StatusKind::Success,
                        );
                    }
                    Err(e) => {
                        self.screen = Screen::Dashboard;
                        self.set_status(format!("Switch failed: {}", e), StatusKind::Error);
                    }
                }
            }
        }
    }

    /// Process a single key press, dispatching to the appropriate screen handler
    /// or mapping to a Message for the Dashboard.
    ///
    /// Extracted from `run_loop()` so tests can drive state transitions without
    /// a terminal or event loop.
    #[allow(clippy::drop_non_drop)] // drop(ctx) releases shared borrows before &mut self
    pub fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        // Global: Ctrl-C always quits (raw mode swallows the OS signal)
        if key.code == KeyCode::Char('c')
            && key
                .modifiers
                .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
        {
            self.should_quit = true;
            return;
        }

        // Build read-only context from split borrows (disjoint from &mut self.screen)
        let ctx = crate::tui::actions::ScreenContext {
            config: &self.config,
            creating_in_flight: self.creating_in_flight(),
        };

        // The help overlay is modal: while it is open it consumes every key,
        // so nothing reaches the screen beneath it.
        if let Some(help) = &mut self.help {
            let total = crate::tui::keybindings::rendered_row_count();
            if help.handle_key(key, total) {
                self.help = None;
            }
            return;
        }

        // F1 opens help from anywhere, including a text input, where `?` is a
        // legitimate character and must be typed instead.
        if key.code == ratatui::crossterm::event::KeyCode::F(1) {
            self.open_help();
            return;
        }

        let action = match &mut self.screen {
            Screen::ConfirmDelete(state) => state.handle_key(key, &ctx),
            Screen::GoWorkspace(state) => state.handle_key(key, &ctx),
            Screen::FilterWorkspace(state) => state.handle_key(key, &ctx),
            Screen::RepoSearch(state) => state.handle_key(key, &ctx),
            Screen::CreateWorkspace(state) => state.handle_key(key, &ctx),
            Screen::AddRepos(state) => state.handle_key(key, &ctx),
            Screen::ConfigEditor(state) => state.handle_key(key, &ctx),
            Screen::DiffViewer(state) => state.handle_key(key, &ctx),
            Screen::SwitchBranch(state) => state.handle_key(key, &ctx),
            Screen::GitOps(state) => state.handle_key(key, &ctx),
            Screen::Dashboard => {
                drop(ctx);
                // Dashboard key-to-message mapping
                let msg: Option<Message> = match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) => Some(Message::Quit),
                    (KeyCode::Tab, _) => Some(Message::FocusNext),
                    // Enter: context-sensitive
                    (KeyCode::Enter, _) => match self.focus {
                        Pane::Left => Some(Message::GoToWorkspace),
                        Pane::Right => {
                            let rows = self.flattened_rows();
                            match rows.get(self.cursor_row) {
                                Some(RepoRow::Repo { .. }) => Some(Message::ToggleRepoExpand),
                                Some(RepoRow::File {
                                    repo_index, entry, ..
                                }) => Some(Message::OpenDiffViewer {
                                    repo_index: *repo_index,
                                    path: entry.path.clone(),
                                    staged: entry.staged,
                                }),
                                Some(RepoRow::SectionHeader { .. }) | None => None,
                            }
                        }
                    },
                    // `g` is documented as a workspace-pane key; ungated it
                    // would yank the user out of the repo rows they are browsing.
                    (KeyCode::Char('g'), _) if self.focus == Pane::Left => Some(Message::StartGo),
                    // `c`, `a` and `d` act on the selected space, so they
                    // belong to the pane that selects it. Ungated, `d` popped a
                    // delete-space confirm from a file row: the same surprise
                    // the `g` gate above removed.
                    (KeyCode::Char('c'), _) if self.focus == Pane::Left => {
                        Some(Message::StartCreate)
                    }
                    (KeyCode::Char('a'), _) if self.focus == Pane::Left => Some(Message::StartAdd),
                    (KeyCode::Char('d'), _) if self.focus == Pane::Left => {
                        Some(Message::StartDelete)
                    }
                    // `r` is a general key, not a workspace-pane key: it rescans
                    // the repo list and also reloads the repo pane, so it stays
                    // available from the pane it reloads.
                    (KeyCode::Char('r'), _) => Some(Message::RefreshRepos),
                    // /: pane-gated. Workspaces pane filters spaces, repos pane searches repos.
                    (KeyCode::Char('/'), _) => match self.focus {
                        Pane::Left => Some(Message::StartFilter),
                        Pane::Right => Some(Message::StartSearch),
                    },
                    (KeyCode::Char('?'), _) => Some(Message::StartHelp),
                    // s/space: stage/unstage single file (Right pane only)
                    (KeyCode::Char('s') | KeyCode::Char(' '), _) if self.focus == Pane::Right => {
                        let rows = self.flattened_rows();
                        if let Some(RepoRow::File {
                            repo_index, entry, ..
                        }) = rows.get(self.cursor_row)
                        {
                            Some(Message::StageFile {
                                repo_index: *repo_index,
                                path: entry.path.clone(),
                                currently_staged: entry.staged,
                            })
                        } else {
                            None
                        }
                    }
                    (KeyCode::Char('b'), _) if self.focus == Pane::Right => {
                        let rows = self.flattened_rows();
                        if let Some(RepoRow::Repo { index, .. }) = rows.get(self.cursor_row) {
                            Some(Message::StartSwitchBranch { repo_index: *index })
                        } else {
                            None
                        }
                    }
                    (KeyCode::Char('G'), _) if self.focus == Pane::Right => {
                        let rows = self.flattened_rows();
                        if let Some(RepoRow::Repo { index, .. }) = rows.get(self.cursor_row) {
                            Some(Message::StartGitOps { repo_index: *index })
                        } else {
                            None
                        }
                    }
                    // S: stage all (Right pane) or open config (Left pane)
                    (KeyCode::Char('S'), _) => match self.focus {
                        Pane::Right => {
                            self.repo_index_for_cursor()
                                .map(|repo_index| Message::StageBulk {
                                    repo_index,
                                    stage: true,
                                })
                        }
                        Pane::Left => Some(Message::StartConfig),
                    },
                    // U: unstage all (Right pane only)
                    (KeyCode::Char('U'), _) if self.focus == Pane::Right => self
                        .repo_index_for_cursor()
                        .map(|repo_index| Message::StageBulk {
                            repo_index,
                            stage: false,
                        }),
                    (KeyCode::Char('h'), KeyModifiers::NONE) if self.focus == Pane::Right => {
                        Some(Message::ScrollTableLeft)
                    }
                    (KeyCode::Char('l'), KeyModifiers::NONE) if self.focus == Pane::Right => {
                        Some(Message::ScrollTableRight)
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => match self.focus {
                        Pane::Left => Some(Message::SelectWorkspaceUp),
                        Pane::Right => Some(Message::SelectRepoUp),
                    },
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => match self.focus {
                        Pane::Left => Some(Message::SelectWorkspaceDown),
                        Pane::Right => Some(Message::SelectRepoDown),
                    },
                    // Right arrow: context-sensitive
                    (KeyCode::PageUp, _) => Some(match self.focus {
                        Pane::Left => Message::JumpWorkspace(ListJump::PageUp),
                        Pane::Right => Message::JumpRepo(ListJump::PageUp),
                    }),
                    (KeyCode::PageDown, _) => Some(match self.focus {
                        Pane::Left => Message::JumpWorkspace(ListJump::PageDown),
                        Pane::Right => Message::JumpRepo(ListJump::PageDown),
                    }),
                    (KeyCode::Home, _) => Some(match self.focus {
                        Pane::Left => Message::JumpWorkspace(ListJump::First),
                        Pane::Right => Message::JumpRepo(ListJump::First),
                    }),
                    (KeyCode::End, _) => Some(match self.focus {
                        Pane::Left => Message::JumpWorkspace(ListJump::Last),
                        Pane::Right => Message::JumpRepo(ListJump::Last),
                    }),
                    (KeyCode::Right, _) => match self.focus {
                        Pane::Left => Some(Message::FocusNext),
                        Pane::Right => {
                            let rows = self.flattened_rows();
                            match rows.get(self.cursor_row) {
                                Some(RepoRow::Repo { .. }) => Some(Message::ToggleRepoExpand),
                                _ => None,
                            }
                        }
                    },
                    // Left arrow and Esc: collapse-first on right pane
                    (KeyCode::Left, _) => match self.focus {
                        Pane::Left => None,
                        Pane::Right => {
                            if self.expanded_repos.is_empty() {
                                Some(Message::FocusNext)
                            } else {
                                Some(Message::CollapseAllRepos)
                            }
                        }
                    },
                    (KeyCode::Esc, _) => match self.focus {
                        // The left pane is the top of the navigation tree, so
                        // "back" has nowhere to go. Quitting on a reflex Esc is
                        // the surprising part; `q`/`Ctrl-C` remain the documented
                        // (and only) way out.
                        Pane::Left => None,
                        Pane::Right => {
                            if self.expanded_repos.is_empty() {
                                Some(Message::FocusNext)
                            } else {
                                Some(Message::CollapseAllRepos)
                            }
                        }
                    },
                    _ => None,
                };
                if let Some(m) = msg {
                    let mut next = update(self, m);
                    while let Some(m2) = next {
                        next = update(self, m2);
                    }
                }
                return;
            }
        };

        self.process_action(action);
    }
}

/// The name the Creating log shows for the repo at `index`: its directory
/// name, with the same `"?"` fallback the synchronous loop used for a path
/// with no file name.
fn repo_label(params: &crate::tui::actions::WorktreeParams, index: usize) -> String {
    params
        .repos
        .get(index)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string())
}

/// Background worker for the Creating stage: create or add one worktree per
/// repo, sending `Started` and `Finished` per repo over `tx`, then `Done`.
///
/// Cancellation is checked before every repo and again after its attempt
/// returns: when `cancel` is set, the in-flight git call runs to completion,
/// nothing further is started, and neither `Finished` for that repo nor `Done`
/// is ever sent. The flag is monotonic (see `create_worktree_cancellable`), so
/// the post-call read cannot miss a cancel that already stopped the attempt: a
/// cancelled attempt is never reported. A `send` failing means the receiver was
/// dropped, so the worker stops there.
///
/// The run costs at most `UNATTENDED_FETCH_TIMEOUT` per repo whose refs are not
/// already fresh, and the repos run in sequence, so a selection off VPN takes
/// that limit several times over. It is on this thread rather than the UI's
/// precisely so that cost is waited out with a painting screen and a live Esc.
///
/// It ends itself on the "already checked out" refusal rather than letting the
/// App do it, because the App cannot retroactively stop a worker: carrying on
/// would create worktrees for later repos while the UI had already returned to
/// the strategy picker, and the next attempt would then double-create.
fn run_create_worker(
    params: crate::tui::actions::WorktreeParams,
    tx: mpsc::SyncSender<CreateProgress>,
    cancel: Arc<AtomicBool>,
) {
    for (index, repo_path) in params.repos.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if tx.send(CreateProgress::Started { index }).is_err() {
            return;
        }
        let attempt = crate::core::workspace::create_worktree_cancellable(
            repo_path,
            &params.workspace_dir,
            &params.workspace_name,
            &params.branch_strategy,
            params.pre_create_fetch(repo_path),
            &cancel,
        );
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let created = attempt.created.map(|_| ()).map_err(|e| e.to_string());
        let stop = created
            .as_ref()
            .err()
            .is_some_and(|e| crate::core::workspace::refuses_because_checked_out(e));
        if tx
            .send(CreateProgress::Finished {
                index,
                fetch: attempt.fetch,
                created,
            })
            .is_err()
        {
            return;
        }
        if stop {
            let _ = tx.send(CreateProgress::Stopped(CreateStop::AlreadyCheckedOut {
                index,
            }));
            return;
        }
    }
    let _ = tx.send(CreateProgress::Done);
}

/// Background worker for the Syncing stage: fetch + fast-forward each repo,
/// sending `Started` and `Finished` per repo over `tx`, then `Done`.
///
/// Cancellation is checked before every git call: when `cancel` is set, the
/// in-flight git call runs to completion, nothing further is started for that
/// repo or any later one, and `Done` is never sent. A `send` failing means the
/// receiver was dropped (the user left the report), so the worker stops there.
fn run_sync_worker(
    repos: Vec<PathBuf>,
    tx: mpsc::SyncSender<SyncProgress>,
    cancel: Arc<AtomicBool>,
) {
    for (index, repo_path) in repos.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if tx.send(SyncProgress::Started { index }).is_err() {
            return;
        }
        let outcome = crate::core::workspace::sync_repo_cancellable(
            repo_path,
            crate::core::workspace::UNATTENDED_FETCH_TIMEOUT,
            &cancel,
        );
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if tx.send(SyncProgress::Finished { index, outcome }).is_err() {
            return;
        }
    }
    let _ = tx.send(SyncProgress::Done);
}

/// Background worker for the Running stage: run a single git operation in
/// `repo_path`, forwarding stdout and stderr lines as `Line`, ending with
/// `Done { success }`.
///
/// Cancellation is honored only at entry: if `cancel` is already set the worker
/// returns immediately WITHOUT sending `Done`. An in-flight git subprocess
/// cannot be interrupted, so cancellation takes effect at this boundary — the
/// same semantics as `run_sync_worker`.
fn run_gitop_worker(
    repo_path: PathBuf,
    op: crate::tui::actions::GitOp,
    tx: mpsc::SyncSender<GitOpProgress>,
    cancel: Arc<AtomicBool>,
) {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    if cancel.load(Ordering::Relaxed) {
        return; // do NOT send Done — user cancelled before we started
    }

    match op {
        // Fetch streams git's live progress lines straight through.
        crate::tui::actions::GitOp::Fetch => {
            let args: &[&str] = &["fetch"];
            let child = Command::new("git")
                .args(args)
                .current_dir(&repo_path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            let mut child = match child {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(GitOpProgress::Line(format!(
                        "git {} failed to start: {}",
                        args.join(" "),
                        e
                    )));
                    let _ = tx.send(GitOpProgress::Done { success: false });
                    return;
                }
            };

            // git fetch writes its progress to stderr, so both pipes must be
            // drained. Drain stderr on a helper thread and stdout on this thread
            // so neither pipe can fill and deadlock the child.
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let tx_err = tx.clone();
            let err_handle = std::thread::spawn(move || {
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_err.send(GitOpProgress::Line(line));
                    }
                }
            });

            if let Some(out) = stdout {
                let reader = BufReader::new(out);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = tx.send(GitOpProgress::Line(line));
                }
            }

            let _ = err_handle.join();
            let success = child.wait().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(GitOpProgress::Done { success });
        }
        // Pull runs the multi-step classify/merge logic in `pull_repo`, then
        // reports its single summary line plus the success flag.
        crate::tui::actions::GitOp::Pull => {
            let result = crate::core::workspace::pull_repo(&repo_path);
            // Git output can be multi-line (e.g. overwrite warnings); one
            // GitOpProgress::Line per visual line keeps the tail window honest.
            for line in result.message.lines() {
                let _ = tx.send(GitOpProgress::Line(line.to_string()));
            }
            let _ = tx.send(GitOpProgress::Done {
                success: result.success(),
            });
        }
        // Push publishes the current branch (with `-u origin <branch>` when it
        // has no upstream), reporting git's summary/rejection plus the flag.
        crate::tui::actions::GitOp::Push { set_upstream } => {
            let result = crate::core::workspace::push_repo(&repo_path, set_upstream);
            // Push rejections are multi-line; send each line separately.
            for line in result.message.lines() {
                let _ = tx.send(GitOpProgress::Line(line.to_string()));
            }
            let _ = tx.send(GitOpProgress::Done {
                success: result.success,
            });
        }
        // Rebase runs `rebase_repo` (fetch + rebase, auto-abort on conflict),
        // then reports its summary line(s) plus the success flag.
        crate::tui::actions::GitOp::Rebase { onto } => {
            let result = crate::core::workspace::rebase_repo(&repo_path, &onto);
            for line in result.message.lines() {
                let _ = tx.send(GitOpProgress::Line(line.to_string()));
            }
            let _ = tx.send(GitOpProgress::Done {
                success: result.success(),
            });
        }
    }
}

/// Advance `cursor_row` in the given direction, skipping `SectionHeader` rows.
/// Returns the new cursor position.
///
/// # Invariant
/// `flattened_rows()` guarantees every `SectionHeader` is immediately followed
/// (or preceded) by at least one `Repo` or `File` row — empty sections are never
/// emitted. The boundary escape paths therefore always land on a non-header row.
/// The `debug_assert` below catches any future violation in debug builds.
fn skip_headers(rows: &[RepoRow<'_>], from: usize, down: bool) -> usize {
    let max = rows.len().saturating_sub(1);
    // `from` is clamped for the same reason `reposition_after_section_change`
    // clamps its cursor: callers pass `app.cursor_row`, which can outlive the
    // rows it indexes. Clamping here means every caller inherits the guarantee
    // instead of each one re-deriving it.
    let mut pos = from.min(max);
    loop {
        if down {
            if pos >= max {
                break;
            }
            pos += 1;
        } else {
            if pos == 0 {
                break;
            }
            pos -= 1;
        }
        if !matches!(rows[pos], RepoRow::SectionHeader { .. }) {
            return pos;
        }
    }
    debug_assert!(
        !matches!(rows.get(pos), Some(RepoRow::SectionHeader { .. })),
        "skip_headers boundary landed on SectionHeader at {pos} — flattened_rows invariant violated"
    );
    pos
}

/// After a staging operation, cursor may rest on a SectionHeader.
/// Try advancing forward to the next non-header row; if none, retreat backward.
///
/// See the invariant note on `skip_headers` — the `flattened_rows()` guarantee
/// ensures this always resolves to a non-header. The `debug_assert` catches
/// violations in debug builds.
fn reposition_after_section_change(rows: &[RepoRow<'_>], cursor: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let max = rows.len().saturating_sub(1);
    let cursor = cursor.min(max);
    // Try advancing forward past any header
    let mut pos = cursor;
    while pos < max && matches!(rows[pos], RepoRow::SectionHeader { .. }) {
        pos += 1;
    }
    // If still on a header (e.g. last row is a header), retreat
    while pos > 0 && matches!(rows[pos], RepoRow::SectionHeader { .. }) {
        pos -= 1;
    }
    debug_assert!(
        !matches!(rows.get(pos), Some(RepoRow::SectionHeader { .. })),
        "reposition_after_section_change landed on SectionHeader at {pos} — invariant violated"
    );
    pos
}

pub fn update(app: &mut App, msg: Message) -> Option<Message> {
    match msg {
        Message::Quit => {
            tracing::info!(reason = "quit", "TUI exiting");
            app.should_quit = true;
            None
        }
        Message::FocusNext => {
            app.focus = match app.focus {
                Pane::Left => Pane::Right,
                Pane::Right => Pane::Left,
            };
            None
        }
        Message::SelectWorkspaceUp => {
            if app.selected_ws > 0 {
                app.selected_ws -= 1;
                app.selected_repo = 0;
                app.reset_repo_pane_state();
                app.begin_workspace_load();
            }
            None
        }
        Message::SelectWorkspaceDown => {
            if app.selected_ws + 1 < app.workspaces.len() {
                app.selected_ws += 1;
                app.selected_repo = 0;
                app.reset_repo_pane_state();
                app.begin_workspace_load();
            }
            None
        }
        Message::SelectRepoUp => {
            let rows = app.flattened_rows();
            app.cursor_row = skip_headers(&rows, app.cursor_row, false);
            None
        }
        Message::SelectRepoDown => {
            let rows = app.flattened_rows();
            app.cursor_row = skip_headers(&rows, app.cursor_row, true);
            None
        }
        Message::JumpWorkspace(jump) => {
            let target = jump.target(app.selected_ws, app.workspaces.len());
            // A jump that changes nothing must not fire the reset and the
            // reload: a reflex End on an already-last space would otherwise
            // discard the repo pane's expansions and caches.
            if target != app.selected_ws {
                app.selected_ws = target;
                app.selected_repo = 0;
                app.reset_repo_pane_state();
                app.begin_workspace_load();
            }
            None
        }
        Message::JumpRepo(jump) => {
            let rows = app.flattened_rows();
            if rows.is_empty() {
                return None;
            }
            let target = jump.target(app.cursor_row, rows.len());
            // First and Last cannot land on a header (see the `flattened_rows`
            // invariant), but a page can. Step out of it the way the jump was
            // travelling, so a PgUp never resolves downwards. `.get()` matches
            // every other `cursor_row` read in this file: the cursor can be
            // stale, and `target` is only clamped against the rows we just
            // built.
            app.cursor_row = match rows.get(target) {
                Some(RepoRow::SectionHeader { .. }) => {
                    skip_headers(&rows, target, target >= app.cursor_row)
                }
                Some(_) => target,
                None => 0,
            };
            None
        }
        Message::RefreshRepos => {
            let summary = app.rescan_repo_list();
            app.cursor_row = 0;
            app.expanded_repos.clear();
            app.refresh_file_diff_cache();
            let (msg, kind) = rescan_notice(&summary, 0);
            app.set_status(msg, kind);
            None
        }
        Message::GoToWorkspace => {
            if let Some(ws) = app.selected_workspace() {
                app.space_cd_target = Some(ws.path.clone());
                tracing::info!(reason = "cd", "TUI exiting");
                app.should_quit = true;
            }
            None
        }
        Message::StartCreate => {
            let state =
                crate::tui::screens::create::CreateState::new(app.repos_cache.clone(), vec![]);
            app.screen = Screen::CreateWorkspace(state);
            None
        }
        Message::StartGo => {
            let state = crate::tui::screens::go::GoState::new(&app.workspaces);
            app.screen = Screen::GoWorkspace(state);
            None
        }
        Message::StartFilter => {
            if app.workspaces.is_empty() {
                app.set_status("No spaces yet, press c to create one", StatusKind::Info);
                return None;
            }
            let state = crate::tui::screens::go::GoState::filter(&app.workspaces);
            app.screen = Screen::FilterWorkspace(state);
            None
        }
        Message::StartAdd => {
            if let Some(ws) = app.selected_workspace() {
                let available = addable_repos(&app.repos_cache, ws);
                let state =
                    crate::tui::screens::add::AddState::new(ws.name.clone(), available, vec![]);
                app.screen = Screen::AddRepos(state);
            }
            None
        }
        Message::StartDelete => {
            if let Some(ws) = app.selected_workspace() {
                let state = crate::tui::screens::delete::DeleteState {
                    workspace_name: ws.name.clone(),
                    repo_names: ws.repos.iter().map(|r| r.name.clone()).collect(),
                };
                app.screen = Screen::ConfirmDelete(state);
            }
            None
        }
        Message::StartSearch => {
            let state = crate::tui::screens::search::SearchState::new(app.repos_cache.clone());
            app.screen = Screen::RepoSearch(state);
            None
        }
        Message::StartConfig => {
            let state = crate::tui::screens::config::ConfigState::from_config(&app.config);
            app.screen = Screen::ConfigEditor(state);
            None
        }
        Message::StartSwitchBranch { repo_index } => {
            if let Some(repo) = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
            {
                let state = crate::tui::screens::switch_branch::SwitchBranchState::new(
                    repo.name.clone(),
                    repo.path.clone(),
                );
                app.screen = Screen::SwitchBranch(state);
            }
            None
        }
        Message::StartGitOps { repo_index } => {
            if let Some(repo) = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
            {
                let state = crate::tui::screens::gitops::GitOpsState::new(
                    repo.name.clone(),
                    repo.path.clone(),
                );
                app.screen = Screen::GitOps(state);
            }
            None
        }
        Message::ToggleRepoExpand => {
            if let Some(idx) = app.repo_index_for_cursor() {
                if app.expanded_repos.contains(&idx) {
                    // Collapsing: remove from expanded, snap cursor to the repo row
                    app.expanded_repos.remove(&idx);
                    let snap_pos = app
                        .flattened_rows()
                        .iter()
                        .position(|r| matches!(r, crate::tui::app::RepoRow::Repo { index, .. } if *index == idx))
                        .unwrap_or(0);
                    app.cursor_row = snap_pos;
                } else {
                    // Expanding: fetch diffs and cache
                    if let Some(repo_path) = app
                        .selected_workspace()
                        .and_then(|ws| ws.repos.get(idx))
                        .map(|r| r.path.clone())
                    {
                        match crate::core::git::file_diff(&repo_path) {
                            Ok(entries) => {
                                app.repo_file_cache.insert(idx, entries);
                                if let Some(mtime) = crate::core::git::git_index_mtime(&repo_path) {
                                    app.repo_index_mtime.insert(idx, mtime);
                                }
                            }
                            Err(err) => {
                                app.set_status(format!("Diff failed: {}", err), StatusKind::Error);
                                return None;
                            }
                        }
                    }
                    app.expanded_repos.insert(idx);
                }
            }
            None
        }
        Message::StartHelp => {
            app.open_help();
            None
        }
        Message::CollapseAllRepos => {
            let current_repo_idx = app.repo_index_for_cursor().unwrap_or(0);
            app.expanded_repos.clear();
            // Snap cursor to the repo row it was in
            let repos_len = app
                .selected_workspace()
                .map(|ws| ws.repos.len())
                .unwrap_or(0);
            app.cursor_row = current_repo_idx.min(repos_len.saturating_sub(1));
            None
        }
        Message::StageFile {
            repo_index,
            path,
            currently_staged,
        } => {
            let repo_path = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
                .map(|r| r.path.clone());
            let repo_path = match repo_path {
                Some(p) => p,
                None => {
                    app.set_status("Stage failed: repo not found", StatusKind::Error);
                    return None;
                }
            };
            app.do_stage(repo_index, &repo_path, &path, currently_staged);
            // Adjust cursor: clamp and skip headers after the file moved sections
            let rows = app.flattened_rows();
            app.cursor_row = reposition_after_section_change(&rows, app.cursor_row);
            None
        }
        Message::StageBulk { repo_index, stage } => {
            let repo_path = app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
                .map(|r| r.path.clone());
            let repo_path = match repo_path {
                Some(p) => p,
                None => {
                    app.set_status("Stage failed: repo not found", StatusKind::Error);
                    return None;
                }
            };
            let result = if stage {
                crate::core::git::stage_all_unstaged(&repo_path)
            } else {
                crate::core::git::unstage_all_staged(&repo_path)
            };
            match result {
                Ok(count) => {
                    // Invalidate diff content cache entries for this repo
                    app.diff_content_cache
                        .retain(|key, _| key.repo_index != repo_index);
                    app.file_mtime_cache
                        .retain(|key, _| key.repo_index != repo_index);
                    // Re-fetch file list for this repo
                    match crate::core::git::file_diff(&repo_path) {
                        Ok(entries) => {
                            app.repo_file_cache.insert(repo_index, entries);
                        }
                        Err(_) => {
                            // Keep stale cache entry — better than empty UI
                            let verb = if stage { "Staged" } else { "Unstaged" };
                            app.set_status(
                                format!("{} {} file(s) -- refresh failed, press r", verb, count),
                                StatusKind::Warning,
                            );
                            return None;
                        }
                    }
                    let verb = if stage { "Staged" } else { "Unstaged" };
                    app.set_status(format!("{} {} file(s)", verb, count), StatusKind::Success);
                }
                Err(err) => {
                    let verb = if stage { "Stage" } else { "Unstage" };
                    app.set_status(format!("{} failed: {}", verb, err), StatusKind::Error);
                    // Operation failed — section structure unchanged, no reposition needed.
                    return None;
                }
            }
            // Reposition cursor after section structure changed
            let rows = app.flattened_rows();
            app.cursor_row = reposition_after_section_change(&rows, app.cursor_row);
            None
        }
        Message::OpenDiffViewer {
            repo_index,
            path,
            staged,
        } => {
            let (repo_path, repo_name) = match app
                .selected_workspace()
                .and_then(|ws| ws.repos.get(repo_index))
            {
                Some(r) => (r.path.clone(), r.name.clone()),
                None => {
                    app.set_status("Diff viewer: repo not found", StatusKind::Error);
                    return None;
                }
            };

            // Check if .git/index has changed since cache was populated
            let current_mtime = crate::core::git::git_index_mtime(&repo_path);
            let cached_mtime = app.repo_index_mtime.get(&repo_index);
            let stale = match (current_mtime, cached_mtime) {
                (Some(current), Some(cached)) => &current != cached,
                (Some(_), None) => true, // mtime exists but wasn't recorded -- treat as stale
                _ => false,              // can't read mtime -- skip check
            };
            if stale {
                // Invalidate diff content cache for this repo
                app.diff_content_cache
                    .retain(|key, _| key.repo_index != repo_index);
                app.file_mtime_cache
                    .retain(|key, _| key.repo_index != repo_index);
                // Update recorded mtime
                if let Some(mtime) = current_mtime {
                    app.repo_index_mtime.insert(repo_index, mtime);
                }
            }

            let cache_key = DiffCacheKey {
                repo_index,
                path: path.clone(),
                staged,
            };

            // Check if the specific working-tree file has changed
            if !staged {
                // Only relevant for unstaged diffs — staged diffs depend on the index, already covered
                let file_full_path = repo_path.join(&path);
                let current_file_mtime = std::fs::metadata(&file_full_path)
                    .and_then(|m| m.modified())
                    .ok();
                let cached_file_mtime = app.file_mtime_cache.get(&cache_key);
                let file_stale = match (current_file_mtime, cached_file_mtime) {
                    (Some(current), Some(cached)) => &current != cached,
                    (Some(_), None) => false, // first time — not stale, will be recorded after caching
                    _ => false,
                };
                if file_stale {
                    app.diff_content_cache.remove(&cache_key);
                }
            }

            let diff = if let Some(cached) = app.diff_content_cache.get(&cache_key) {
                cached.clone()
            } else {
                let result = crate::core::git::file_content_diff(&repo_path, &path, staged)
                    .map_err(|e| e.to_string());
                app.diff_content_cache
                    .insert(cache_key.clone(), result.clone());
                // Record the file's mtime for future staleness checks
                if !staged {
                    let file_full_path = repo_path.join(&path);
                    if let Ok(meta) = std::fs::metadata(&file_full_path) {
                        if let Ok(mtime) = meta.modified() {
                            app.file_mtime_cache.insert(cache_key, mtime);
                        }
                    }
                }
                result
            };

            let total_lines = diff
                .as_ref()
                .map(|d| u16::try_from(d.lines.len()).unwrap_or(u16::MAX))
                .unwrap_or(0);

            let state = crate::tui::screens::diff::DiffViewerState {
                repo_index,
                repo_name,
                repo_path,
                file_path: path,
                staged,
                diff,
                scroll_offset: 0,
                total_lines,
            };
            app.screen = Screen::DiffViewer(state);
            None
        }
        Message::ScrollTableLeft => {
            app.table_scroll_x = app.table_scroll_x.saturating_sub(SCROLL_STEP);
            None
        }
        Message::ScrollTableRight => {
            app.table_scroll_x = app.table_scroll_x.saturating_add(SCROLL_STEP);
            None
        }
    }
}

/// Entry point — initialise terminal, run event loop, restore terminal.
/// Returns a path to cd into, if the user pressed enter on a workspace.
pub fn run(app: &mut App) -> Result<()> {
    color_eyre::install().ok();
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, app);
    ratatui::restore();
    result
}

/// What a rescan of the repo list found, for the status notice.
pub struct RescanSummary {
    /// Repos in the rebuilt repo list.
    pub total: usize,
    /// Repos present now that were absent from the previous repo list.
    pub new_count: usize,
}

/// Notice for a rescan: `Rescanned: 42 repos, 2 new`, with a trailing
/// `, 1 selected repo no longer found` (kind Warning) when a picker had toggled
/// repos that the rescan no longer finds.
fn rescan_notice(summary: &RescanSummary, missing_toggles: usize) -> (String, StatusKind) {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut msg = format!(
        "Rescanned: {} repo{}, {} new",
        summary.total,
        plural(summary.total),
        summary.new_count
    );
    if missing_toggles == 0 {
        (msg, StatusKind::Success)
    } else {
        msg.push_str(&format!(
            ", {} selected repo{} no longer found",
            missing_toggles,
            plural(missing_toggles)
        ));
        (msg, StatusKind::Warning)
    }
}

/// The repos a space can still add: the repo list minus the repos already in
/// the space, matched by repo name (so a repo elsewhere that shares a name with
/// one in the space is hidden too; pre-existing behaviour).
fn addable_repos(repos: &[PathBuf], ws: &Workspace) -> Vec<PathBuf> {
    let existing: std::collections::HashSet<&str> =
        ws.repos.iter().map(|r| r.name.as_str()).collect();
    repos
        .iter()
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            !existing.contains(name.as_str())
        })
        .cloned()
        .collect()
}

/// Build a FuzzyPicker populated with local + remote branches from `repo_path`.
/// Returns `None` if branch listing fails (not a git repo, etc.).
/// `label` is the picker-title verb phrase naming what the selection does
/// (e.g. "Branch" for switch/checkout flows, "Rebase onto" for the rebase
/// target picker), so the title states the consequence of pressing Enter.
pub(crate) fn build_branch_picker(
    repo_path: &std::path::Path,
    repo_name: &str,
    label: &str,
) -> Option<crate::tui::widgets::fuzzy_picker::FuzzyPicker> {
    use crate::core::git::list_branches;
    use crate::tui::widgets::fuzzy_picker::{FuzzyPicker, PickerItem};

    let branches = list_branches(repo_path).ok()?;
    if branches.is_empty() {
        return None;
    }

    let items: Vec<PickerItem> = branches
        .into_iter()
        .map(|b| PickerItem {
            // `name` is what gets passed to git — must be the clean branch name.
            // Indicate the current branch via the `parent` field shown in the picker.
            name: b.name,
            parent: match (b.is_remote, b.is_current) {
                (_, true) => "current".to_string(),
                (true, false) => "remote".to_string(),
                (false, false) => "local".to_string(),
            },
            full_path: std::path::PathBuf::new(), // unused for branch picker
            branch: None,
            remote_url: None,
        })
        .collect();

    Some(FuzzyPicker::new(
        format!("{}  ({})  ENTER=select  ESC=back", label, repo_name),
        items,
        false,
    ))
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};

    // Drain any stale input that accumulated before the TUI started —
    // e.g. keystrokes typed during a previous frozen/crashed session that
    // left the terminal in raw mode.  Without this, buffered events replay
    // immediately into the first field, corrupting it.
    while event::poll(std::time::Duration::ZERO)? {
        let _ = event::read()?;
    }

    loop {
        // Check debounce timer and poll background results each frame
        app.check_debounce_timer();
        app.poll_background_result();
        app.poll_sync_result();
        app.poll_gitop_result();
        app.poll_create_result();
        app.spinner_tick = app.spinner_tick.wrapping_add(1);

        app.expire_status_message(Instant::now());
        terminal.draw(|frame| crate::tui::ui::view(app, frame))?;

        // Post-draw: clamp table_scroll_x to the actual max for the current terminal
        // size, so 'h' always responds immediately rather than unwinding an overshoot.
        if let Ok(size) = terminal.size() {
            let right_pane = ((size.width as f64 * 75.0) / 100.0).round() as usize;
            let inner = right_pane.saturating_sub(2);
            let max = crate::tui::ui::max_table_scroll(inner);
            app.table_scroll_x = app.table_scroll_x.min(max);
        }

        if event::poll(std::time::Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                app.handle_key(key);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::git::RepoStatus;
    use crate::core::workspace::{Workspace, WorkspaceRepo};
    use std::time::{Duration, Instant};

    fn make_app(workspaces: Vec<Workspace>) -> App {
        App {
            config: SpaceConfig::default(),
            workspaces,
            repos_cache: vec![],
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            diff_content_cache: HashMap::new(),
            file_mtime_cache: HashMap::new(),
            repo_index_mtime: HashMap::new(),
            cursor_row: 0,
            table_scroll_x: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            help: None,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_kind: StatusKind::Info,
            status_message_set_at: None,
            ws_load_tx: None,
            ws_result_rx: None,
            ws_generation: 0,
            sync_rx: None,
            sync_cancel: None,
            gitop_rx: None,
            gitop_cancel: None,
            create_job: None,
            nav_pending: None,
            ws_loading: false,
            ws_loading_since: None,
            spinner_tick: 0,
        }
    }

    fn stub_repo(name: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            name: name.to_string(),
            path: std::path::PathBuf::from("/tmp").join(name),
            branch: "main".to_string(),
            status: RepoStatus::default(),
            ahead: 0,
            behind: 0,
        }
    }

    fn app_with_status() -> App {
        let mut app = make_app(vec![]);
        app.set_status(
            "Added repos to workspace 'mission-control-ui'",
            StatusKind::Success,
        );
        app
    }

    #[test]
    fn status_message_expires_after_timeout() {
        let mut app = app_with_status();
        app.status_message_set_at =
            Some(Instant::now() - STATUS_MESSAGE_TTL - Duration::from_millis(1));

        app.expire_status_message(Instant::now());

        assert_eq!(app.status_message, None);
    }

    #[test]
    fn navigate_to_workspace_resets_scroll_and_cursor() {
        let ws0 = Workspace {
            name: "alpha".to_string(),
            path: std::path::PathBuf::from("/tmp/alpha"),
            repos: vec![stub_repo("repo-a")],
        };
        let ws1 = Workspace {
            name: "beta".to_string(),
            path: std::path::PathBuf::from("/tmp/beta"),
            repos: vec![stub_repo("repo-b")],
        };
        let mut app = make_app(vec![ws0, ws1]);

        // Simulate stale state from a previous workspace
        app.table_scroll_x = 20;
        app.cursor_row = 3;

        // Dispatch NavigateToWorkspace targeting a repo in workspace 1
        app.process_action(crate::tui::actions::ScreenAction::NavigateToWorkspace(
            "repo-b".to_string(),
        ));

        assert_eq!(
            app.selected_ws, 1,
            "selected_ws should switch to workspace 1"
        );
        assert_eq!(app.table_scroll_x, 0, "table_scroll_x must be reset to 0");
        assert_eq!(app.cursor_row, 0, "cursor_row must be reset to 0");
    }

    // ── Background loader / debounce tests ────────────────────────────────────

    #[test]
    fn begin_workspace_load_sets_loading_flags() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/nonexistent/my-ws"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        let gen_before = app.ws_generation;

        app.begin_workspace_load();

        assert!(
            app.ws_loading,
            "ws_loading must be true after begin_workspace_load"
        );
        assert!(
            app.nav_pending.is_some(),
            "nav_pending must be set to start the debounce timer"
        );
        assert!(
            app.ws_loading_since.is_some(),
            "ws_loading_since must be set for the spinner grace period"
        );
        assert_eq!(
            app.ws_generation,
            gen_before + 1,
            "ws_generation must increment immediately to invalidate any in-flight result"
        );
    }

    #[test]
    fn begin_workspace_load_populates_skeleton_repos() {
        // Use a real temp dir with a .git entry so skeletons are actually returned.
        let dir = tempfile::tempdir().unwrap();
        let ws_dir = dir.path().join("workspaces");
        let wt_dir = ws_dir.join("my-ws").join("alpha");
        std::fs::create_dir_all(&wt_dir).unwrap();
        std::fs::write(wt_dir.join(".git"), "gitdir: /fake").unwrap();

        let mut config = SpaceConfig::default();
        config.workspaces.dir = ws_dir;

        let ws = Workspace {
            name: "my-ws".into(),
            path: dir.path().join("workspaces").join("my-ws"),
            repos: vec![],
        };
        let mut app = App {
            config,
            workspaces: vec![ws],
            repos_cache: vec![],
            selected_ws: 0,
            selected_repo: 0,
            expanded_repos: HashSet::new(),
            repo_file_cache: HashMap::new(),
            diff_content_cache: HashMap::new(),
            file_mtime_cache: HashMap::new(),
            repo_index_mtime: HashMap::new(),
            cursor_row: 0,
            table_scroll_x: 0,
            focus: Pane::Left,
            screen: Screen::Dashboard,
            help: None,
            should_quit: false,
            space_cd_target: None,
            status_message: None,
            status_kind: StatusKind::Info,
            status_message_set_at: None,
            ws_load_tx: None,
            ws_result_rx: None,
            ws_generation: 0,
            sync_rx: None,
            sync_cancel: None,
            gitop_rx: None,
            gitop_cancel: None,
            create_job: None,
            nav_pending: None,
            ws_loading: false,
            ws_loading_since: None,
            spinner_tick: 0,
        };

        app.begin_workspace_load();

        assert_eq!(
            app.workspaces[0].repos.len(),
            1,
            "skeleton repos should be populated"
        );
        assert_eq!(
            app.workspaces[0].repos[0].branch, "...",
            "skeleton branch must be '...'"
        );
    }

    #[test]
    fn begin_workspace_load_immediate_fires_without_debounce() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/nonexistent/my-ws"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        let gen_before = app.ws_generation;

        // With no channel (ws_load_tx: None), fire_load_request is a no-op,
        // but we can verify ws_generation was incremented (proving the request
        // was attempted immediately, not deferred to the debounce timer).
        app.begin_workspace_load_immediate();

        assert_eq!(
            app.ws_generation,
            gen_before + 1,
            "ws_generation must increment immediately (no debounce wait)"
        );
        assert!(
            app.nav_pending.is_none(),
            "nav_pending must NOT be set — there is no scroll gesture to debounce"
        );
        assert!(app.ws_loading, "ws_loading must be set");
    }

    #[test]
    fn check_debounce_timer_no_op_before_150ms() {
        let mut app = make_app(vec![]);
        // Set nav_pending to "just now" — well within the 150ms window
        app.nav_pending = Some(Instant::now());
        let generation_before = app.ws_generation;

        app.check_debounce_timer();

        assert!(
            app.nav_pending.is_some(),
            "nav_pending must still be set when < 150ms has elapsed"
        );
        assert_eq!(
            app.ws_generation, generation_before,
            "ws_generation must not increment before debounce fires"
        );
    }

    #[test]
    fn check_debounce_timer_fires_after_150ms() {
        let mut app = make_app(vec![]);
        // Simulate nav_pending set 200ms ago (past the 150ms threshold)
        app.nav_pending = Some(Instant::now() - Duration::from_millis(200));
        let generation_before = app.ws_generation;

        app.check_debounce_timer();

        assert!(
            app.nav_pending.is_none(),
            "nav_pending must be cleared after debounce fires"
        );
        assert_eq!(
            app.ws_generation,
            generation_before + 1,
            "ws_generation must increment when debounce fires"
        );
    }

    #[test]
    fn apply_workspace_result_failed_load_clears_loading_and_sets_status() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/tmp/my-ws"),
            repos: vec![stub_repo("alpha")],
        };
        let mut app = make_app(vec![ws]);
        app.ws_generation = 1;
        app.ws_loading = true;
        app.ws_loading_since = Some(Instant::now());

        // workspace=None simulates workspace_detail() returning Err
        let failed = LoadResult {
            generation: 1,
            workspace: None,
        };
        app.apply_workspace_result(failed);

        assert!(
            !app.ws_loading,
            "ws_loading must be cleared even when the load failed"
        );
        assert!(
            app.ws_loading_since.is_none(),
            "ws_loading_since must be cleared on failed load"
        );
        assert!(
            app.status_message.is_some(),
            "a status error message must be set when the load fails"
        );
        // Workspace repos must be unchanged (failed result must not wipe out skeletons)
        assert_eq!(
            app.workspaces[0].repos.len(),
            1,
            "workspace repos must be unchanged after a failed load"
        );
    }

    #[test]
    fn apply_workspace_result_discards_stale_generation() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/tmp/my-ws"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        app.ws_generation = 5;
        app.ws_loading = true;

        // Result with an old generation (stale — should be discarded)
        let stale = LoadResult {
            generation: 3,
            workspace: Some(Workspace {
                name: "my-ws".into(),
                path: std::path::PathBuf::from("/tmp/my-ws"),
                repos: vec![stub_repo("new-repo")],
            }),
        };
        app.apply_workspace_result(stale);

        assert!(
            app.ws_loading,
            "ws_loading must remain true after discarding stale result"
        );
        assert!(
            app.workspaces[0].repos.is_empty(),
            "workspace repos must not be updated from a stale result"
        );
    }

    #[test]
    fn apply_workspace_result_applies_matching_generation() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/tmp/my-ws"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        app.ws_generation = 2;
        app.ws_loading = true;
        app.ws_loading_since = Some(Instant::now());

        let result = LoadResult {
            generation: 2,
            workspace: Some(Workspace {
                name: "my-ws".into(),
                path: std::path::PathBuf::from("/tmp/my-ws"),
                repos: vec![stub_repo("alpha"), stub_repo("beta")],
            }),
        };
        app.apply_workspace_result(result);

        assert!(
            !app.ws_loading,
            "ws_loading must be cleared after applying a matching result"
        );
        assert!(
            app.ws_loading_since.is_none(),
            "ws_loading_since must be cleared after applying a matching result"
        );
        assert_eq!(
            app.workspaces[0].repos.len(),
            2,
            "workspace repos must be updated from the matching result"
        );
        assert_eq!(app.workspaces[0].repos[0].name, "alpha");
        assert_eq!(app.workspaces[0].repos[1].name, "beta");
    }

    #[test]
    fn poll_background_result_no_op_with_no_channel() {
        let mut app = make_app(vec![]);
        app.ws_loading = true;
        // ws_result_rx is None (set by make_app) — poll must be a no-op
        app.poll_background_result();
        assert!(
            app.ws_loading,
            "ws_loading must be unchanged when there is no channel"
        );
    }

    #[test]
    fn poll_background_result_clears_loading_when_worker_dies() {
        let mut app = make_app(vec![]);
        app.ws_loading = true;
        app.ws_loading_since = Some(Instant::now());

        // Create a channel, drop the sender immediately — simulates worker panic
        let (tx, rx) = mpsc::sync_channel::<LoadResult>(4);
        drop(tx);
        app.ws_result_rx = Some(rx);

        app.poll_background_result();

        assert!(
            !app.ws_loading,
            "ws_loading must be cleared when the worker thread disconnects"
        );
        assert!(
            app.ws_loading_since.is_none(),
            "ws_loading_since must be cleared when the worker disconnects"
        );
        assert!(
            app.ws_result_rx.is_none(),
            "dead channel must not be restored to ws_result_rx"
        );
    }

    #[test]
    fn poll_background_result_applies_result_from_channel() {
        let ws = Workspace {
            name: "my-ws".into(),
            path: std::path::PathBuf::from("/tmp/my-ws"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        app.ws_generation = 1;
        app.ws_loading = true;
        app.ws_loading_since = Some(Instant::now());

        // Wire up a real channel and inject the receiver into the app
        let (tx, rx) = mpsc::sync_channel::<LoadResult>(4);
        app.ws_result_rx = Some(rx);

        tx.send(LoadResult {
            generation: 1,
            workspace: Some(Workspace {
                name: "my-ws".into(),
                path: std::path::PathBuf::from("/tmp/my-ws"),
                repos: vec![stub_repo("alpha")],
            }),
        })
        .unwrap();

        app.poll_background_result();

        assert!(
            !app.ws_loading,
            "ws_loading must be cleared after poll applies the result"
        );
        assert_eq!(
            app.workspaces[0].repos.len(),
            1,
            "workspace must be updated with the result from the channel"
        );
    }

    #[test]
    fn workspace_nav_key_down_sets_loading_state() {
        use ratatui::crossterm::event::KeyCode;
        let ws0 = Workspace {
            name: "alpha".into(),
            path: std::path::PathBuf::from("/tmp/alpha"),
            repos: vec![],
        };
        let ws1 = Workspace {
            name: "beta".into(),
            path: std::path::PathBuf::from("/tmp/beta"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws0, ws1]);

        // Press j (SelectWorkspaceDown) — switches from workspace 0 to 1
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));

        assert_eq!(app.selected_ws, 1, "selected_ws must advance");
        assert!(
            app.ws_loading,
            "ws_loading must be true immediately after navigation"
        );
        assert!(
            app.nav_pending.is_some(),
            "nav_pending must be set to start the debounce timer"
        );
    }

    #[test]
    fn workspace_nav_key_up_sets_loading_state() {
        use ratatui::crossterm::event::KeyCode;
        let ws0 = Workspace {
            name: "alpha".into(),
            path: std::path::PathBuf::from("/tmp/alpha"),
            repos: vec![],
        };
        let ws1 = Workspace {
            name: "beta".into(),
            path: std::path::PathBuf::from("/tmp/beta"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws0, ws1]);
        app.selected_ws = 1;

        // Press k (SelectWorkspaceUp)
        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));

        assert_eq!(app.selected_ws, 0, "selected_ws must decrease");
        assert!(
            app.ws_loading,
            "ws_loading must be true immediately after navigation"
        );
    }

    // ── O1 regression guard ───────────────────────────────────────────────────

    // ── Sync flow tests ───────────────────────────────────────────────────────

    #[test]
    fn execute_sync_flow_creates_channel() {
        let mut app = make_app(vec![]);
        app.process_action(crate::tui::actions::ScreenAction::ExecuteSyncFlow(vec![]));
        assert!(
            app.sync_rx.is_some(),
            "sync_rx must be set after ExecuteSyncFlow"
        );
    }

    #[test]
    fn poll_sync_result_drops_rx_when_not_syncing() {
        let mut app = make_app(vec![]);
        // Dashboard screen — not in Syncing stage
        let (_tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        app.poll_sync_result();
        assert!(
            app.sync_rx.is_none(),
            "sync_rx must be dropped when not in Syncing stage"
        );
    }

    #[test]
    fn run_sync_worker_honors_preset_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(64);
        run_sync_worker(
            vec![
                PathBuf::from("/nonexistent/repo-a"),
                PathBuf::from("/nonexistent/repo-b"),
            ],
            tx,
            cancel,
        );
        assert!(
            rx.try_recv().is_err(),
            "no Step or Done must be sent when cancel is preset — repos skipped"
        );
    }

    #[test]
    fn poll_gitop_result_auto_closes_on_elapsed_success_timer() {
        use crate::tui::screens::gitops::{GitOpsStage, GitOpsState};
        let mut app = make_app(vec![]);
        let mut st = GitOpsState::new("repo-a".to_string(), PathBuf::from("/tmp/repo-a"));
        st.stage = GitOpsStage::Running;
        st.finished = Some(true);
        st.close_at = Some(Instant::now() - Duration::from_secs(1));
        app.screen = Screen::GitOps(st);

        app.poll_gitop_result();

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "an elapsed success close timer must return to the dashboard"
        );
    }

    #[test]
    fn poll_gitop_result_stays_open_on_failure() {
        use crate::tui::screens::gitops::{GitOpsStage, GitOpsState};
        let mut app = make_app(vec![]);
        let mut st = GitOpsState::new("repo-a".to_string(), PathBuf::from("/tmp/repo-a"));
        st.stage = GitOpsStage::Running;
        st.finished = Some(false);
        st.close_at = None;
        app.screen = Screen::GitOps(st);

        app.poll_gitop_result();

        assert!(
            matches!(app.screen, Screen::GitOps(_)),
            "a failed op with no close timer must keep the overlay open until Esc"
        );
    }

    #[test]
    fn execute_gitop_creates_channel() {
        use crate::tui::actions::{GitOp, ScreenAction};
        let mut app = make_app(vec![]);
        app.process_action(ScreenAction::ExecuteGitOp {
            repo_path: PathBuf::from("/nonexistent/repo-a"),
            op: GitOp::Fetch,
        });
        assert!(
            app.gitop_rx.is_some(),
            "gitop_rx must be set after ExecuteGitOp"
        );
    }

    #[test]
    fn poll_gitop_result_drops_rx_when_not_running() {
        let mut app = make_app(vec![]);
        // Dashboard screen — not in the GitOps Running stage.
        let (_tx, rx) = mpsc::sync_channel::<GitOpProgress>(4);
        app.gitop_rx = Some(rx);
        app.poll_gitop_result();
        assert!(
            app.gitop_rx.is_none(),
            "gitop_rx must be dropped when not in the Running stage"
        );
    }

    #[test]
    fn run_gitop_worker_honors_preset_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::sync_channel::<GitOpProgress>(64);
        run_gitop_worker(
            PathBuf::from("/nonexistent/repo-a"),
            crate::tui::actions::GitOp::Fetch,
            tx,
            cancel,
        );
        assert!(
            rx.try_recv().is_err(),
            "no Line or Done must be sent when cancel is preset — worker returns at the boundary"
        );
    }

    #[test]
    fn poll_sync_result_signals_cancel_when_leaving_syncing_stage() {
        let mut app = make_app(vec![]);
        // Dashboard screen — not in Syncing stage
        let flag = Arc::new(AtomicBool::new(false));
        let (_tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        app.sync_cancel = Some(flag.clone());

        app.poll_sync_result();

        assert!(
            flag.load(Ordering::Relaxed),
            "cancel flag must be set when leaving the Syncing stage"
        );
        assert!(app.sync_rx.is_none(), "sync_rx must be dropped");
        assert!(
            app.sync_cancel.is_none(),
            "sync_cancel handle must be dropped"
        );
    }

    #[test]
    fn reentering_execute_sync_flow_cancels_previous_worker() {
        use crate::tui::actions::ScreenAction;
        let mut app = make_app(vec![]);

        app.process_action(ScreenAction::ExecuteSyncFlow(vec![]));
        let first = app.sync_cancel.clone().expect("first cancel flag set");
        assert!(!first.load(Ordering::Relaxed), "first flag starts unset");

        // Re-entry must signal the previous worker before replacing the handle.
        app.process_action(ScreenAction::ExecuteSyncFlow(vec![]));
        assert!(
            first.load(Ordering::Relaxed),
            "previous worker's cancel flag must be set on re-entry"
        );
        assert!(
            app.sync_cancel.is_some(),
            "a fresh cancel handle must be stored for the new worker"
        );
    }

    fn ok_outcome() -> SyncOutcome {
        SyncOutcome {
            fetch: crate::core::workspace::FetchOutcome::Ok,
            forwarded: vec![],
            skipped: vec![],
        }
    }

    #[test]
    fn poll_sync_result_started_marks_row_syncing_and_moves_cursor() {
        use crate::tui::screens::create::{CreateStage, CreateState};
        use crate::tui::screens::sync_report::{RowPhase, SyncReport};
        let mut app = make_app(vec![]);
        let mut state = CreateState::new(vec![], vec![]);
        state.stage = CreateStage::Syncing;
        state.report = SyncReport::new(&[PathBuf::from("/r/a"), PathBuf::from("/r/b")]);
        app.screen = Screen::CreateWorkspace(state);

        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        tx.send(SyncProgress::Started { index: 1 }).unwrap();

        app.poll_sync_result();

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(
                    st.stage,
                    CreateStage::Syncing,
                    "stage must remain Syncing while channel is open"
                );
                assert_eq!(st.report.rows[1].phase, RowPhase::Syncing);
                assert_eq!(st.report.cursor, 1, "cursor follows the syncing row");
                assert!(!st.report.done);
                assert_eq!(st.report.title(), "Sync report \u{b7} 0 of 2");
            }
            _ => panic!("expected CreateWorkspace screen"),
        }
        assert!(
            app.sync_rx.is_some(),
            "sync_rx must be kept while Syncing is active"
        );
    }

    #[test]
    fn poll_sync_result_done_pauses_on_report_and_enter_advances() {
        use crate::tui::screens::create::{CreateStage, CreateState};
        use crate::tui::screens::sync_report::SyncReport;
        let mut app = make_app(vec![]);
        let mut state = CreateState::new(vec![], vec![]);
        state.stage = CreateStage::Syncing;
        state.report = SyncReport::new(&[PathBuf::from("/r/a")]);
        app.screen = Screen::CreateWorkspace(state);

        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        app.sync_cancel = Some(Arc::new(AtomicBool::new(false)));
        tx.send(SyncProgress::Started { index: 0 }).unwrap();
        tx.send(SyncProgress::Finished {
            index: 0,
            outcome: ok_outcome(),
        })
        .unwrap();
        tx.send(SyncProgress::Done).unwrap();

        app.poll_sync_result();

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(
                    st.stage,
                    CreateStage::Syncing,
                    "Done must pause on the report, not advance"
                );
                assert!(st.report.done, "the report must be finished");
                assert_eq!(st.report.title(), "Sync report \u{b7} 1 ok");
            }
            _ => panic!("expected CreateWorkspace screen"),
        }
        assert!(app.sync_rx.is_none(), "sync_rx must be dropped after Done");
        assert!(
            app.sync_cancel.is_none(),
            "cancel handle must be dropped after Done"
        );

        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        match &app.screen {
            Screen::CreateWorkspace(st) => assert_eq!(
                st.stage,
                CreateStage::PickBranchStrategy,
                "Enter on a finished report must advance to PickBranchStrategy"
            ),
            _ => panic!("expected CreateWorkspace screen"),
        }
    }

    #[test]
    fn poll_sync_result_disconnect_marks_unfinished_rows_not_synced() {
        use crate::tui::screens::create::{CreateStage, CreateState};
        use crate::tui::screens::sync_report::{RowPhase, SyncReport};
        let mut app = make_app(vec![]);
        let mut state = CreateState::new(vec![], vec![]);
        state.stage = CreateStage::Syncing;
        state.report = SyncReport::new(&[
            PathBuf::from("/r/a"),
            PathBuf::from("/r/b"),
            PathBuf::from("/r/c"),
        ]);
        app.screen = Screen::CreateWorkspace(state);

        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(8);
        app.sync_rx = Some(rx);
        tx.send(SyncProgress::Started { index: 0 }).unwrap();
        tx.send(SyncProgress::Finished {
            index: 0,
            outcome: ok_outcome(),
        })
        .unwrap();
        tx.send(SyncProgress::Started { index: 1 }).unwrap();
        drop(tx); // the worker went away without sending Done

        app.poll_sync_result();

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(st.stage, CreateStage::Syncing, "must stay on the report");
                assert!(st.report.done, "disconnect counts as Done");
                assert_eq!(st.report.rows[1].phase, RowPhase::NotSynced);
                assert_eq!(st.report.rows[2].phase, RowPhase::NotSynced);
                assert_eq!(st.report.title(), "Sync report \u{b7} 1 ok, 2 failed");
                assert_eq!(
                    st.report.cursor, 1,
                    "cursor lands on the first not-synced row"
                );
            }
            _ => panic!("expected CreateWorkspace screen"),
        }
        assert!(
            app.sync_rx.is_none(),
            "sync_rx must be dropped on disconnect"
        );
    }

    #[test]
    fn poll_sync_result_add_repos_started_marks_row_syncing() {
        use crate::tui::screens::add::{AddStage, AddState};
        use crate::tui::screens::sync_report::{RowPhase, SyncReport};
        let mut app = make_app(vec![]);
        let mut state = AddState::new("my-ws".to_string(), vec![], vec![]);
        state.stage = AddStage::Syncing;
        state.report = SyncReport::new(&[PathBuf::from("/r/a")]);
        app.screen = Screen::AddRepos(state);

        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        tx.send(SyncProgress::Started { index: 0 }).unwrap();

        app.poll_sync_result();

        match &app.screen {
            Screen::AddRepos(st) => {
                assert_eq!(
                    st.stage,
                    AddStage::Syncing,
                    "stage must remain Syncing while channel is open"
                );
                assert_eq!(st.report.rows[0].phase, RowPhase::Syncing);
            }
            _ => panic!("expected AddRepos screen"),
        }
        assert!(
            app.sync_rx.is_some(),
            "sync_rx must be kept while Syncing is active"
        );
    }

    #[test]
    fn poll_sync_result_add_repos_done_pauses_on_report_and_enter_advances() {
        use crate::tui::screens::add::{AddStage, AddState};
        use crate::tui::screens::sync_report::SyncReport;
        let mut app = make_app(vec![]);
        let mut state = AddState::new("my-ws".to_string(), vec![], vec![]);
        state.stage = AddStage::Syncing;
        state.report = SyncReport::new(&[PathBuf::from("/r/a")]);
        app.screen = Screen::AddRepos(state);

        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(4);
        app.sync_rx = Some(rx);
        tx.send(SyncProgress::Done).unwrap();

        app.poll_sync_result();

        match &app.screen {
            Screen::AddRepos(st) => {
                assert_eq!(st.stage, AddStage::Syncing, "Done must pause on the report");
                assert!(st.report.done);
                assert_eq!(
                    st.report.title(),
                    "Sync report \u{b7} 0 ok, 1 failed",
                    "a row the worker never reached is not synced"
                );
            }
            _ => panic!("expected AddRepos screen"),
        }
        assert!(app.sync_rx.is_none(), "sync_rx must be dropped after Done");

        app.handle_key(ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
        match &app.screen {
            Screen::AddRepos(st) => assert_eq!(
                st.stage,
                AddStage::PickBranchStrategy,
                "Enter on a finished report must advance to PickBranchStrategy"
            ),
            _ => panic!("expected AddRepos screen"),
        }
    }

    #[test]
    fn run_sync_worker_sends_started_and_finished_per_repo_then_done() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel::<SyncProgress>(64);
        run_sync_worker(vec![PathBuf::from("/nonexistent/repo-a")], tx, cancel);

        assert!(matches!(
            rx.recv().unwrap(),
            SyncProgress::Started { index: 0 }
        ));
        match rx.recv().unwrap() {
            SyncProgress::Finished { index, outcome } => {
                assert_eq!(index, 0);
                assert!(!outcome.fetch_ok(), "a missing directory cannot be fetched");
            }
            _ => panic!("expected Finished after Started"),
        }
        assert!(matches!(rx.recv().unwrap(), SyncProgress::Done));
        assert!(rx.try_recv().is_err(), "nothing follows Done");
    }

    // ── Creating flow tests ───────────────────────────────────────────────────

    fn init_repo(dir: &std::path::Path) {
        fn git(args: &[&str], dir: &std::path::Path) {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        git(&["init", "-b", "main"], dir);
        git(&["config", "user.email", "t@local"], dir);
        git(&["config", "user.name", "T"], dir);
        git(&["config", "commit.gpgsign", "false"], dir);
        git(&["commit", "--allow-empty", "-m", "init"], dir);
    }

    /// A repo at `<parent>/<name>`, initialised with one commit on `main`.
    fn make_repo(parent: &std::path::Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        init_repo(&path);
        path
    }

    /// Params for a create run. Every repo is listed as fresh, so no
    /// pre-create fetch runs: these tests are about the loop's boundaries,
    /// and a fetch of a repo with no `origin` would only add noise.
    fn create_params(
        ws_dir: &std::path::Path,
        ws_name: &str,
        repos: Vec<PathBuf>,
    ) -> crate::tui::actions::WorktreeParams {
        crate::tui::actions::WorktreeParams {
            workspace_name: ws_name.to_string(),
            workspace_dir: ws_dir.to_path_buf(),
            repos: repos.clone(),
            branch_strategy: crate::core::workspace::BranchStrategy::DetachedHead,
            is_new: true,
            fresh_repos: repos,
            unreachable_repos: vec![],
        }
    }

    fn make_job(
        params: crate::tui::actions::WorktreeParams,
    ) -> (mpsc::SyncSender<CreateProgress>, Arc<AtomicBool>, CreateJob) {
        let (tx, rx) = mpsc::sync_channel::<CreateProgress>(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let job = CreateJob {
            rx,
            cancel: cancel.clone(),
            params,
            started: 0,
            finished: 0,
            created: 0,
        };
        (tx, cancel, job)
    }

    /// A Creating-stage create screen, with the log already cleared.
    fn creating_screen() -> Screen {
        use crate::tui::screens::create::{CreateStage, CreateState};
        let mut st = CreateState::new(vec![], vec![]);
        st.stage = CreateStage::Creating;
        Screen::CreateWorkspace(st)
    }

    /// Cancelling mid-run stops the repos that had not started, and the
    /// rendezvous channel makes that deterministic without a single sleep.
    ///
    /// The channel has capacity 0, so a `send` completes only when this test
    /// receives. Receiving `Finished { index: 0 }` proves the worker is past
    /// repo A in its entirety, because that message is sent only after A's
    /// post-call cancel check passed. Setting the flag then leaves exactly two
    /// interleavings, and both leave B untouched:
    ///
    /// - the worker had not yet reached B's top-of-loop check, so it reads the
    ///   flag, returns, and never sends `Started { index: 1 }`; or
    /// - it had passed that check and is BLOCKED in `send(Started { index: 1 })`.
    ///   It cannot proceed until this test receives, and this test only
    ///   receives after the store, so when it unblocks both checkpoints inside
    ///   `create_worktree_cancellable` read a flag that is already true, and
    ///   the post-call check means no `Finished { index: 1 }` follows either.
    ///
    /// The load-bearing step is that the worker CANNOT be past
    /// `send(Started { index: 1 })` before the store, because completing that
    /// send requires a receive that only happens after it. So a
    /// `Started { index: 1 }` is permitted and is not asserted against; what is
    /// asserted is B's absence from disk and the absence of any terminal
    /// message. Asserting A EXISTS is not decoration: without it the whole test
    /// would pass vacuously if the harness never created anything at all.
    ///
    /// Be precise about what the rendezvous buys, because it is easy to
    /// overstate. It is what makes this test DETERMINISTIC; the two checkpoints
    /// inside `create_worktree_cancellable` are what make it PASS. Measured at
    /// capacity 64 in a clean copy of this tree, it still passed 8 runs out of
    /// 8: with a buffer the worker can be past `send(Started { index: 1 })`
    /// before the store, the interleaving above no longer holds, and the
    /// checkpoints catch it anyway because a fetch plus a `git worktree add`
    /// takes far longer than the store. So raising the capacity would not fail
    /// here. It would quietly convert a proof into a race that is merely very
    /// likely to be won, on a window milliseconds wide.
    ///
    /// The capacity also does memory-model work, which is the second reason not
    /// to treat it as a performance knob. `Relaxed` is right in both places but
    /// for different reasons: in production the flag is the only thing
    /// communicated, so the worker need only see it eventually, which is all
    /// `Relaxed` promises. Here the test needs the load to see the store at a
    /// specific point, and it is the CHANNEL that supplies the happens-before
    /// edge: the store precedes this test's receive, which pairs with the
    /// worker's send, which precedes the worker's next load. A buffered channel
    /// removes that edge as surely as it removes the scheduling guarantee.
    ///
    /// This is not hypothetical. While this branch was under review, another
    /// session changed that 0 to a 64 directly in this working tree. The whole
    /// suite stayed green throughout, and the only thing that caught it was a
    /// reviewer noticing a line it had not touched. Nothing else could: not the
    /// suite, not clippy, and not the strongest gate any one author has, which
    /// is knowing what they themselves changed.
    #[test]
    fn cancelling_mid_run_creates_no_further_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_a = make_repo(tmp.path(), "repo-a");
        let repo_b = make_repo(tmp.path(), "repo-b");
        let ws_dir = tmp.path().join("spaces");
        let params = create_params(&ws_dir, "ws-a", vec![repo_a, repo_b]);

        // Capacity 0 is load-bearing and there is nothing to assert it against:
        // at any other capacity this test still passes, it just stops proving
        // anything. See the interleaving argument above before changing it.
        let (tx, rx) = mpsc::sync_channel::<CreateProgress>(0);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || run_create_worker(params, tx, worker_cancel));

        match rx.recv().expect("Started for repo A") {
            CreateProgress::Started { index } => assert_eq!(index, 0),
            _ => panic!("expected Started for repo A"),
        }
        match rx.recv().expect("Finished for repo A") {
            CreateProgress::Finished { index, created, .. } => {
                assert_eq!(index, 0);
                assert_eq!(created, Ok(()), "repo A must be created before the cancel");
            }
            _ => panic!("expected Finished for repo A"),
        }

        cancel.store(true, Ordering::Relaxed);

        while let Ok(msg) = rx.recv() {
            match msg {
                CreateProgress::Started { index: 1 } => {} // permitted, see above
                CreateProgress::Started { index } => {
                    panic!("unexpected Started for repo {}", index)
                }
                CreateProgress::Finished { index, .. } => {
                    panic!("repo {} was attempted after the cancel", index)
                }
                CreateProgress::Stopped(_) => panic!("a cancelled run must not report a stop"),
                CreateProgress::Done => panic!("a cancelled run must never send Done"),
            }
        }
        worker.join().unwrap();

        assert!(
            ws_dir.join("ws-a").join("repo-a").join(".git").exists(),
            "repo A was created before the cancel, so the harness does create worktrees"
        );
        assert!(
            !ws_dir.join("ws-a").join("repo-b").exists(),
            "nothing further may start after the cancel"
        );
    }

    /// The counterpart to the cancellation test: `Finished` absent means
    /// cancelled, `Finished` with an `Err` means failed, and the two must not
    /// be confusable. An ordinary failure does not end the run.
    #[test]
    fn a_failed_add_is_reported_and_the_run_continues() {
        let tmp = tempfile::tempdir().unwrap();
        // Not a git repo, so its `git worktree add` fails for an ordinary reason.
        let not_a_repo = tmp.path().join("not-a-repo");
        std::fs::create_dir_all(&not_a_repo).unwrap();
        let repo_b = make_repo(tmp.path(), "repo-b");
        let ws_dir = tmp.path().join("spaces");
        let params = create_params(&ws_dir, "ws-a", vec![not_a_repo, repo_b]);

        let (tx, rx) = mpsc::sync_channel::<CreateProgress>(64);
        run_create_worker(params, tx, Arc::new(AtomicBool::new(false)));

        assert!(matches!(
            rx.recv().unwrap(),
            CreateProgress::Started { index: 0 }
        ));
        match rx.recv().unwrap() {
            CreateProgress::Finished { index, created, .. } => {
                assert_eq!(index, 0);
                let err = created.expect_err("a repo that is not a git repo cannot be added");
                assert!(
                    err.contains("not a git repository"),
                    "the log needs git's reason, got {:?}",
                    err
                );
            }
            _ => panic!("expected Finished for the failing repo"),
        }
        assert!(
            matches!(rx.recv().unwrap(), CreateProgress::Started { index: 1 }),
            "an ordinary failure must not end the run"
        );
        match rx.recv().unwrap() {
            CreateProgress::Finished { index, created, .. } => {
                assert_eq!(index, 1);
                assert_eq!(created, Ok(()));
            }
            _ => panic!("expected Finished for repo B"),
        }
        assert!(matches!(rx.recv().unwrap(), CreateProgress::Done));
        assert!(
            ws_dir.join("ws-a").join("repo-b").join(".git").exists(),
            "the repo after the failure is still created"
        );
    }

    #[test]
    fn run_create_worker_honors_preset_cancel_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        let params = create_params(
            &ws_dir,
            "ws-a",
            vec![
                PathBuf::from("/nonexistent/repo-a"),
                PathBuf::from("/nonexistent/repo-b"),
            ],
        );
        let (tx, rx) = mpsc::sync_channel::<CreateProgress>(64);
        run_create_worker(params, tx, Arc::new(AtomicBool::new(true)));
        assert!(
            rx.try_recv().is_err(),
            "nothing at all is sent when cancel is preset: no repo is attempted"
        );
    }

    /// The #29 regression guard. `poll_sync_result` gates on the screen and
    /// cancels when it does not match, which is what made wrapping a mid-sync
    /// screen unsafe. This poller must not do either: a screen the App did not
    /// expect may never cancel or discard a run the user asked for.
    #[test]
    fn poll_create_result_keeps_the_job_when_the_screen_is_not_creating() {
        let mut app = make_app(vec![]);
        let params = create_params(std::path::Path::new("/ws"), "ws-a", vec![]);
        let (_tx, cancel, job) = make_job(params);
        app.create_job = Some(job);
        // Dashboard: not the Creating stage.
        app.poll_create_result();

        assert!(
            app.create_job.is_some(),
            "the job must survive a screen the poller does not recognise"
        );
        assert!(
            !cancel.load(Ordering::Relaxed),
            "the screen must never be able to cancel the run"
        );
    }

    /// The add flow's success wording, which the end-to-end add tests exercise
    /// but never assert. Without this, a typo in the "Added repos to" verb
    /// would pass the whole suite, since only the create side is pinned.
    #[test]
    fn poll_create_result_done_says_added_for_the_add_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        std::fs::create_dir_all(ws_dir.join("ws-a")).unwrap();

        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        app.screen = Screen::AddRepos(crate::tui::screens::add::AddState::new(
            "ws-a".to_string(),
            vec![],
            vec![],
        ));
        if let Screen::AddRepos(st) = &mut app.screen {
            st.stage = crate::tui::screens::add::AddStage::Creating;
        }
        let mut params = create_params(&ws_dir, "ws-a", vec![PathBuf::from("/r/a")]);
        params.is_new = false;
        let (tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);

        tx.send(CreateProgress::Done).unwrap();
        app.poll_create_result();

        assert_eq!(
            app.status_message.as_deref(),
            Some("Added repos to workspace 'ws-a'")
        );
        assert_eq!(app.status_kind, StatusKind::Success);
    }

    #[test]
    fn poll_create_result_done_selects_the_new_space_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        // Two spaces, so selecting by name is distinguishable from index 0.
        std::fs::create_dir_all(ws_dir.join("aaa-other")).unwrap();
        std::fs::create_dir_all(ws_dir.join("ws-a")).unwrap();

        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        app.screen = creating_screen();
        let params = create_params(&ws_dir, "ws-a", vec![PathBuf::from("/r/a")]);
        let (tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);

        tx.send(CreateProgress::Done).unwrap();
        app.poll_create_result();

        assert!(
            matches!(app.screen, Screen::Dashboard),
            "a run with no error returns to the dashboard"
        );
        assert_eq!(
            app.workspaces.get(app.selected_ws).map(|w| w.name.as_str()),
            Some("ws-a"),
            "the new space is selected by name, not by index"
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("Created workspace 'ws-a'")
        );
        assert_eq!(app.status_kind, StatusKind::Success);
        assert!(app.create_job.is_none(), "the finished job is dropped");
    }

    #[test]
    fn poll_create_result_stopped_bounces_to_the_branch_strategy_picker() {
        use crate::tui::screens::create::CreateStage;
        let mut app = make_app(vec![]);
        app.screen = creating_screen();
        if let Screen::CreateWorkspace(st) = &mut app.screen {
            st.progress
                .push("Creating worktree for repo-a...".to_string());
        }
        let params = create_params(
            std::path::Path::new("/ws"),
            "ws-a",
            vec![PathBuf::from("/r/repo-a")],
        );
        let (tx, cancel, job) = make_job(params);
        app.create_job = Some(job);

        tx.send(CreateProgress::Stopped(CreateStop::AlreadyCheckedOut {
            index: 0,
        }))
        .unwrap();
        app.poll_create_result();

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(st.stage, CreateStage::PickBranchStrategy);
                assert!(st.progress.is_empty(), "the log is cleared on the bounce");
                assert_eq!(
                    st.error.as_deref(),
                    Some("'repo-a' is already checked out — pick a different strategy")
                );
            }
            _ => panic!("expected the create screen"),
        }
        assert!(
            cancel.load(Ordering::Relaxed),
            "the worker must be stopped before the user picks again"
        );
        assert!(app.create_job.is_none(), "the stopped job is dropped");
    }

    #[test]
    fn poll_create_result_disconnect_before_every_repo_records_an_error() {
        let mut app = make_app(vec![]);
        app.screen = creating_screen();
        let params = create_params(
            std::path::Path::new("/ws"),
            "ws-a",
            vec![PathBuf::from("/r/a"), PathBuf::from("/r/b")],
        );
        let (tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);

        // The worker died after one repo: its sender is gone, Done never came.
        tx.send(CreateProgress::Finished {
            index: 0,
            fetch: None,
            created: Ok(()),
        })
        .unwrap();
        drop(tx);
        app.poll_create_result();

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                let err = st.error.as_deref().expect("a dead worker must be reported");
                assert!(
                    err.contains("stopped unexpectedly"),
                    "the error must name what happened, got {:?}",
                    err
                );
            }
            _ => panic!("a half-finished run stays on the Creating stage"),
        }
        assert!(app.create_job.is_none(), "the dead job is dropped");
    }

    #[test]
    fn reentering_execute_worktree_flow_cancels_previous_worker() {
        use crate::tui::actions::ScreenAction;
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        let mut app = make_app(vec![]);
        app.screen = creating_screen();

        app.process_action(ScreenAction::ExecuteWorktreeFlow(create_params(
            &ws_dir,
            "ws-a",
            vec![],
        )));
        let first = app
            .create_job
            .as_ref()
            .map(|j| j.cancel.clone())
            .expect("first job stored");
        assert!(!first.load(Ordering::Relaxed), "first flag starts unset");

        app.process_action(ScreenAction::ExecuteWorktreeFlow(create_params(
            &ws_dir,
            "ws-a",
            vec![],
        )));
        assert!(
            first.load(Ordering::Relaxed),
            "the previous worker's cancel flag must be set on re-entry"
        );
        assert!(
            app.create_job.is_some(),
            "a fresh job must be stored for the new worker"
        );
    }

    #[test]
    fn cancel_creating_leaves_for_the_dashboard_and_says_what_it_left() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        // Two spaces, so selecting by name is distinguishable from index 0.
        std::fs::create_dir_all(ws_dir.join("aaa-other")).unwrap();
        std::fs::create_dir_all(ws_dir.join("ws-a")).unwrap();

        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        app.screen = creating_screen();
        let params = create_params(
            &ws_dir,
            "ws-a",
            vec![
                PathBuf::from("/r/a"),
                PathBuf::from("/r/b"),
                PathBuf::from("/r/c"),
            ],
        );
        let (_tx, cancel, mut job) = make_job(params);
        job.created = 2;
        app.create_job = Some(job);

        app.process_action(crate::tui::actions::ScreenAction::CancelCreating);

        assert!(
            cancel.load(Ordering::Relaxed),
            "the worker must be told to stop at its next boundary"
        );
        assert!(matches!(app.screen, Screen::Dashboard));
        assert_eq!(
            app.workspaces.get(app.selected_ws).map(|w| w.name.as_str()),
            Some("ws-a"),
            "the partial space is refreshed in and selected by name"
        );
        assert_eq!(
            app.status_message.as_deref(),
            Some("Stopped creating 'ws-a' after 2 of 3 repos. Press a to add the rest.")
        );
        assert_eq!(
            app.status_kind,
            StatusKind::Warning,
            "a partial space needs attention, but stopping is not a failure"
        );
        assert!(app.create_job.is_none(), "the cancelled job is dropped");
    }

    /// A key can arrive in the same frame as the worker's terminal message.
    /// `run_loop` drains, draws, then waits up to 16ms for a key, so `Done`
    /// can land inside that wait and still be undrained when Esc is read.
    /// The user is pressing Esc against the screen they were shown, so
    /// leaving is right; calling it a cancellation is not. The run had
    /// finished, and reporting a finished run as "Stopped ... after 0 of 0
    /// repos" is a success reported as a failure, the same class of defect
    /// as retrying a partial space.
    #[test]
    fn esc_in_the_frame_the_run_finished_reports_the_run_not_a_cancellation() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        std::fs::create_dir_all(ws_dir.join("ws-a")).unwrap();

        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        app.screen = creating_screen();
        let params = create_params(&ws_dir, "ws-a", vec![]);
        let (tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);
        // Queued but not yet drained: exactly the window run_loop leaves.
        tx.send(CreateProgress::Done).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(matches!(app.screen, Screen::Dashboard));
        assert_eq!(
            app.status_message.as_deref(),
            Some("Created workspace 'ws-a'"),
            "a run that finished must be reported as finished, not as a stop"
        );
        assert_eq!(app.status_kind, StatusKind::Success);
        assert!(app.create_job.is_none());
    }

    /// The worse half of the same race. An `AlreadyCheckedOut` stop exists to
    /// put the user back on the strategy picker with a reason they can act on.
    /// Treating that frame's Esc as a cancellation would drop them on the
    /// dashboard with a generic message and silently lose the recovery path
    /// the `Stopped` variant was added to provide.
    #[test]
    fn esc_in_the_frame_a_stop_arrived_still_bounces_to_the_picker() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");

        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        app.screen = creating_screen();
        let params = create_params(&ws_dir, "ws-a", vec![PathBuf::from("/r/repo-a")]);
        let (tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);
        tx.send(CreateProgress::Stopped(CreateStop::AlreadyCheckedOut {
            index: 0,
        }))
        .unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        match &app.screen {
            Screen::CreateWorkspace(st) => {
                assert_eq!(
                    st.stage,
                    crate::tui::screens::create::CreateStage::PickBranchStrategy,
                    "the bounce must survive an Esc in the same frame"
                );
                assert_eq!(
                    st.error.as_deref(),
                    Some("'repo-a' is already checked out — pick a different strategy"),
                    "the actionable reason must not be replaced by a generic stop"
                );
            }
            _ => panic!("expected the strategy picker, not the dashboard"),
        }
        assert!(app.create_job.is_none());
    }

    #[test]
    fn cancel_creating_says_adding_for_the_add_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let ws_dir = tmp.path().join("spaces");
        let mut app = make_app(vec![]);
        app.config.workspaces.dir = ws_dir.clone();
        let mut params = create_params(&ws_dir, "ws-a", vec![PathBuf::from("/r/a")]);
        params.is_new = false;
        let (_tx, _cancel, job) = make_job(params);
        app.create_job = Some(job);

        app.process_action(crate::tui::actions::ScreenAction::CancelCreating);

        assert_eq!(
            app.status_message.as_deref(),
            Some("Stopped adding to 'ws-a' after 0 of 1 repos. Press a to add the rest.")
        );
    }

    #[test]
    fn workspace_nav_does_not_populate_file_cache() {
        let ws = Workspace {
            name: "ws".into(),
            path: std::path::PathBuf::from("/nonexistent"),
            repos: vec![],
        };
        let mut app = make_app(vec![ws]);
        // Pre-populate the cache with a sentinel entry.
        // If load_selected_workspace_detail() eagerly calls refresh_file_diff_cache(),
        // it will clear() the cache and the sentinel will be gone.
        // After the fix (no eager call), the sentinel must survive.
        app.repo_file_cache.insert(99, vec![]);
        app.load_selected_workspace_detail();
        assert!(
            app.repo_file_cache.contains_key(&99),
            "file cache should not be cleared/populated by workspace load (deferred to expand)"
        );
    }
}
