//! State shared by the Syncing and Creating stages of the create and add flows.
//!
//! `SyncReport` is the per-repo results screen shown before the branch picker
//! (glossary: sync report); each row carries a sync outcome. `LogView` is the
//! scroll state of the Creating stage's string log. Rendering lives in
//! `ui.rs`; everything here is plain data and pure formatting so it can be
//! unit-tested without a terminal.

use crate::core::workspace::{FetchOutcome, SkipReason, SyncOutcome};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Rows moved by `PgUp`/`PgDn` in the report and the Creating log.
pub const PAGE_ROWS: usize = 10;
/// Fewest list rows the report shows before the pane takes any height.
pub const MIN_LIST_ROWS: usize = 3;

/// Where a repo is in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowPhase {
    Waiting,
    Syncing,
    Done(SyncOutcome),
    /// The worker stopped before reaching this repo.
    NotSynced,
}

/// One repo of the report, in selection order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRow {
    pub name: String,
    pub path: PathBuf,
    pub phase: RowPhase,
}

/// One line of the detail pane; `dim` lines render muted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLine {
    pub text: String,
    pub dim: bool,
}

impl PaneLine {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            dim: false,
        }
    }

    fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            dim: true,
        }
    }
}

/// The detail pane's lines, wrapped, in priority order: the all-failed
/// notice when present, the `<name>  <path>` header, the status line, then
/// the skips or git's stderr. `fit_pane` cuts from the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneContent {
    pub lines: Vec<PaneLine>,
}

impl SyncRow {
    pub fn is_finished(&self) -> bool {
        matches!(self.phase, RowPhase::Done(_) | RowPhase::NotSynced)
    }

    /// Ok means the fetch succeeded, whatever happened to branches.
    pub fn is_ok(&self) -> bool {
        matches!(&self.phase, RowPhase::Done(o) if o.fetch_ok())
    }

    /// Every finished row that is not ok: fetch failed, timed out, not synced.
    pub fn is_failed(&self) -> bool {
        self.is_finished() && !self.is_ok()
    }

    pub fn glyph(&self) -> &'static str {
        match &self.phase {
            RowPhase::Waiting => "\u{b7}",
            RowPhase::Syncing => "\u{25cf}",
            RowPhase::Done(o) if o.fetch_ok() => "\u{2713}",
            RowPhase::Done(_) | RowPhase::NotSynced => "\u{2717}",
        }
    }

    pub fn outcome_label(&self) -> &'static str {
        match &self.phase {
            RowPhase::Waiting => "waiting",
            RowPhase::Syncing => "syncing\u{2026}",
            RowPhase::NotSynced => "not synced",
            RowPhase::Done(o) => match &o.fetch {
                FetchOutcome::Ok if o.forwarded.is_empty() => "up to date",
                FetchOutcome::Ok => "fast-forwarded",
                FetchOutcome::Failed { .. } => "fetch failed",
                FetchOutcome::TimedOut { .. } => "timed out",
            },
        }
    }

    /// The DETAIL column: a plain part and a dimmed suffix (the skip count).
    pub fn detail(&self) -> (String, String) {
        match &self.phase {
            RowPhase::Waiting | RowPhase::Syncing => (String::new(), String::new()),
            RowPhase::NotSynced => ("sync worker stopped".to_string(), String::new()),
            RowPhase::Done(o) => match &o.fetch {
                FetchOutcome::Ok => {
                    let plain = o.forwarded.join(", ");
                    let dim = if o.skipped.is_empty() {
                        String::new()
                    } else if plain.is_empty() {
                        format!("{} skipped", o.skipped.len())
                    } else {
                        format!(" \u{b7} {} skipped", o.skipped.len())
                    };
                    (plain, dim)
                }
                FetchOutcome::Failed { exit_code, stderr } => (
                    first_stderr_line(stderr).unwrap_or_else(|| exit_label(*exit_code, stderr)),
                    String::new(),
                ),
                FetchOutcome::TimedOut { after, .. } => (
                    format!("timed out after {}s", after.as_secs()),
                    String::new(),
                ),
            },
        }
    }

    /// Detail pane lines for this row: the header, the status line, then the
    /// skips or git's stderr. Not yet wrapped.
    fn pane_lines(&self) -> Vec<PaneLine> {
        let mut lines = vec![PaneLine::plain(format!(
            "{}  {}",
            self.name,
            display_path(&self.path)
        ))];
        match &self.phase {
            RowPhase::Waiting => lines.push(PaneLine::plain("waiting")),
            RowPhase::Syncing => lines.push(PaneLine::plain("fetching origin\u{2026}")),
            RowPhase::NotSynced => lines.push(PaneLine::plain(
                "not synced \u{b7} sync worker stopped \u{b7} branch picker will use local refs",
            )),
            RowPhase::Done(o) => match &o.fetch {
                FetchOutcome::Ok => {
                    if o.forwarded.is_empty() {
                        lines.push(PaneLine::plain("fetch ok \u{b7} nothing to fast-forward"));
                    } else {
                        lines.push(PaneLine::plain(format!(
                            "fetch ok \u{b7} fast-forwarded {}",
                            o.forwarded.join(", ")
                        )));
                    }
                    for skip in &o.skipped {
                        lines.push(PaneLine::dim(match &skip.reason {
                            SkipReason::CheckedOutAt(path) => format!(
                                "skipped {}: checked out in a worktree at {}",
                                skip.name,
                                display_path(path)
                            ),
                            SkipReason::Other(line) => format!("skipped {}: {}", skip.name, line),
                        }));
                    }
                }
                FetchOutcome::Failed { exit_code, stderr } => {
                    lines.push(PaneLine::plain(format!(
                        "fetch failed ({}) \u{b7} branch picker will use local refs",
                        exit_label(*exit_code, stderr)
                    )));
                    lines.extend(stderr.lines().map(PaneLine::plain));
                }
                FetchOutcome::TimedOut { after, stderr } => {
                    lines.push(PaneLine::plain(format!(
                        "fetch timed out after {}s, git was stopped \u{b7} branch picker will use local refs",
                        after.as_secs()
                    )));
                    lines.extend(stderr.lines().map(PaneLine::plain));
                }
            },
        }
        lines
    }
}

/// Marker the worker puts at the start of `stderr` when `git` could not be
/// started at all (repo directory gone, git not on `PATH`).
use crate::core::workspace::SPAWN_FAILURE_PREFIX;

/// How the fetch ended, for the status line. A missing exit code is not
/// evidence of a signal: the worker also reports `None` when git could not
/// be started, when waiting for it failed, and for a cancelled sync. Only the
/// spawn failure is recognisable from `stderr`; every other `None` is
/// reported as what it is, an exit code the worker does not have.
fn exit_label(exit_code: Option<i32>, stderr: &str) -> String {
    match exit_code {
        Some(code) => format!("git exit {}", code),
        None if stderr.starts_with(SPAWN_FAILURE_PREFIX) => "git did not start".to_string(),
        None => "no exit code".to_string(),
    }
}

/// git's first non-empty stderr line with only the `fatal: ` prefix stripped.
fn first_stderr_line(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.strip_prefix("fatal: ").unwrap_or(l).to_string())
}

/// A path for display, with the home directory shortened to `~`.
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Truncate `text` to `width` display columns, ending with `…` when cut.
pub fn truncate_to(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('\u{2026}');
    out
}

/// Word-wrap one line to `width` columns; a word wider than the line is split.
pub fn wrap_line(text: &str, width: usize) -> Vec<String> {
    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }
    let mut lines = vec![];
    let mut current = String::new();
    let mut current_w = 0;
    for word in text.split(' ') {
        let word_w = UnicodeWidthStr::width(word);
        let sep = usize::from(!current.is_empty());
        if current_w + sep + word_w <= width {
            if sep == 1 {
                current.push(' ');
            }
            current.push_str(word);
            current_w += sep + word_w;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_w = 0;
        }
        if word_w <= width {
            current.push_str(word);
            current_w = word_w;
            continue;
        }
        // A single word wider than the line: split it by columns.
        for ch in word.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_w + w > width {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            current.push(ch);
            current_w += w;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// The sync report: one row per selected repo, a cursor, and whether the run
/// has ended. Cursor keys are ignored until `done`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub rows: Vec<SyncRow>,
    pub cursor: usize,
    pub done: bool,
}

impl SyncReport {
    pub fn new(repos: &[PathBuf]) -> Self {
        let rows = repos
            .iter()
            .map(|path| SyncRow {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "?".to_string()),
                path: path.clone(),
                phase: RowPhase::Waiting,
            })
            .collect();
        Self {
            rows,
            cursor: 0,
            done: false,
        }
    }

    pub fn empty() -> Self {
        Self::new(&[])
    }

    /// The worker began fetching row `index`; the cursor follows it.
    pub fn started(&mut self, index: usize) {
        if let Some(row) = self.rows.get_mut(index) {
            row.phase = RowPhase::Syncing;
            self.cursor = index;
        }
    }

    pub fn finished(&mut self, index: usize, outcome: SyncOutcome) {
        if let Some(row) = self.rows.get_mut(index) {
            row.phase = RowPhase::Done(outcome);
        }
    }

    /// The run ended, by `Done` or by the worker going away: rows still
    /// waiting or syncing become `not synced`, and the cursor lands on the
    /// first failed row, else the first row.
    pub fn finish(&mut self) {
        for row in &mut self.rows {
            if !row.is_finished() {
                row.phase = RowPhase::NotSynced;
            }
        }
        self.done = true;
        self.cursor = self.rows.iter().position(SyncRow::is_failed).unwrap_or(0);
    }

    pub fn finished_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_finished()).count()
    }

    pub fn ok_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_ok()).count()
    }

    pub fn failed_count(&self) -> usize {
        self.rows.iter().filter(|r| r.is_failed()).count()
    }

    pub fn all_failed(&self) -> bool {
        self.done && !self.rows.is_empty() && self.failed_count() == self.rows.len()
    }

    /// `Sync report · N of M` while running, `· N ok` when nothing failed,
    /// `· N ok, M failed` otherwise. Counts are repos.
    pub fn title(&self) -> String {
        if !self.done {
            return format!(
                "Sync report \u{b7} {} of {}",
                self.finished_count(),
                self.rows.len()
            );
        }
        let failed = self.failed_count();
        if failed == 0 {
            format!("Sync report \u{b7} {} ok", self.ok_count())
        } else {
            format!(
                "Sync report \u{b7} {} ok, {} failed",
                self.ok_count(),
                failed
            )
        }
    }

    /// The two-line notice shown at the top of the pane when every fetch failed.
    pub fn notice_lines() -> [&'static str; 2] {
        [
            "Nothing was fetched. You can still continue;",
            "the branch picker reads local refs, which may be behind origin.",
        ]
    }

    /// The detail pane for the highlighted row, wrapped to `width` columns.
    pub fn pane(&self, width: usize) -> PaneContent {
        let mut raw: Vec<PaneLine> = vec![];
        if self.all_failed() {
            raw.extend(Self::notice_lines().iter().map(|l| PaneLine::plain(*l)));
        }
        if let Some(row) = self.rows.get(self.cursor) {
            raw.extend(row.pane_lines());
        }
        let lines = raw
            .into_iter()
            .flat_map(|line| {
                wrap_line(&line.text, width)
                    .into_iter()
                    .map(move |text| PaneLine {
                        text,
                        dim: line.dim,
                    })
            })
            .collect();
        PaneContent { lines }
    }

    /// The footer for `width` columns. Running: `ESC cancel`. Done: the key
    /// hints, dropping in order the `rows a–b of n` prefix, `PgUp/PgDn page`,
    /// `↑↓ select` until it fits; `ENTER continue` and `ESC back` never drop.
    pub fn footer(&self, scrolled: Option<(usize, usize)>, width: usize) -> String {
        if !self.done {
            return "ESC cancel".to_string();
        }
        let prefix =
            scrolled.map(|(a, b)| format!("rows {}\u{2013}{} of {}", a, b, self.rows.len()));
        let cont = if self.all_failed() {
            "ENTER continue anyway"
        } else {
            "ENTER continue"
        };
        let mut show_prefix = prefix.is_some();
        let mut show_page = true;
        let mut show_select = true;
        loop {
            let mut parts: Vec<&str> = vec![];
            if show_prefix {
                parts.push(prefix.as_deref().unwrap_or_default());
            }
            if show_select {
                parts.push("\u{2191}\u{2193} select");
            }
            if show_page {
                parts.push("PgUp/PgDn page");
            }
            parts.push(cont);
            parts.push("ESC back");
            let text = parts.join(" \u{b7} ");
            if UnicodeWidthStr::width(text.as_str()) <= width {
                return text;
            }
            if show_prefix {
                show_prefix = false;
            } else if show_page {
                show_page = false;
            } else if show_select {
                show_select = false;
            } else {
                return text;
            }
        }
    }

    /// The rows on screen for a list of `list_rows` rows: `start..end`. The
    /// window starts at the top until the cursor would leave it, then keeps
    /// the cursor on the last row (the dashboard list's behaviour).
    pub fn visible_window(&self, list_rows: usize) -> (usize, usize) {
        let n = self.rows.len();
        if list_rows == 0 || n == 0 {
            return (0, 0);
        }
        let start = if self.cursor < list_rows {
            0
        } else {
            self.cursor + 1 - list_rows
        };
        (start, (start + list_rows).min(n))
    }

    /// Cursor movement once `done`; ignored while running. Returns whether
    /// the key was consumed.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if !self.done || self.rows.is_empty() {
            return false;
        }
        let last = self.rows.len() - 1;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(PAGE_ROWS),
            KeyCode::PageDown => self.cursor = (self.cursor + PAGE_ROWS).min(last),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = last,
            _ => return false,
        }
        true
    }
}

/// Split the dialog's inner height between the list and the pane. Below the
/// dialog's height cap the content fits and each part gets what it needs.
/// At the cap the list gets `MIN_LIST_ROWS` first; the pane takes what its
/// content needs up to half of the inner height; the list gets the rest, and
/// any rows it cannot use (fewer repos than rows, the floor included) go
/// back to the pane. At the 10-row minimum dialog (inner 8) that gives a
/// one-repo report a 4-row pane and a two-repo report a 3-row pane: room for
/// the header, the status line and the `… N more lines` marker, or, when
/// every fetch failed, for the two-line notice, the header and the marker.
/// Returns `(list, pane)`.
pub fn report_layout(inner_height: usize, repos: usize, pane_need: usize) -> (usize, usize) {
    // Footer, blank separator and rule.
    let body = inner_height.saturating_sub(3);
    if body == 0 {
        return (0, 0);
    }
    let list_min = MIN_LIST_ROWS.min(body);
    if body >= repos.max(list_min) + pane_need {
        return (body - pane_need, pane_need);
    }
    let mut pane = pane_need.min(inner_height / 2).min(body - list_min);
    let mut list = body - pane;
    if list > repos {
        let give = (list - repos).min(pane_need - pane);
        pane += give;
        list -= give;
    }
    (list, pane)
}

/// Fit the pane into `rows`. When everything fits the lines are returned as
/// they are. Otherwise the last row is always a dimmed `… N more lines`, with
/// `N` the number of lines not on screen, and the rows above it are the
/// leading lines in order: the all-failed notice, the header, the status
/// line, then as many following lines as fit. The marker is never dropped
/// to make room for a line, so a pane shorter than its notice, header and
/// status line loses those from the bottom up: with exactly the header and
/// status rows the marker takes the status row, and a single row shows only
/// the marker.
pub fn fit_pane(content: &PaneContent, rows: usize) -> Vec<PaneLine> {
    let total = content.lines.len();
    if total <= rows {
        return content.lines.clone();
    }
    if rows == 0 {
        return vec![];
    }
    let shown = rows - 1;
    let mut out: Vec<PaneLine> = content.lines[..shown].to_vec();
    out.push(PaneLine::dim(format!(
        "\u{2026} {} more lines",
        total - shown
    )));
    out
}

/// Scroll state of the Creating stage's log. While `follow` is set the view
/// tail-follows new lines; the cursor keys detach it and `End` resumes it.
/// The renderer records the viewport height so a key press starts from what
/// was on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogView {
    pub follow: bool,
    pub scroll: usize,
    viewport: Cell<usize>,
}

impl Default for LogView {
    fn default() -> Self {
        Self::new()
    }
}

impl LogView {
    pub fn new() -> Self {
        Self {
            follow: true,
            scroll: 0,
            viewport: Cell::new(0),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The first line to show for `total` lines in `visible` rows this frame.
    pub fn offset(&self, total: usize, visible: usize) -> usize {
        self.viewport.set(visible);
        let max = total.saturating_sub(visible);
        if self.follow {
            max
        } else {
            self.scroll.min(max)
        }
    }

    /// Scroll keys for a log of `total` lines. Returns whether consumed.
    pub fn handle_key(&mut self, key: KeyEvent, total: usize) -> bool {
        let max = total.saturating_sub(self.viewport.get());
        let current = if self.follow {
            max
        } else {
            self.scroll.min(max)
        };
        let target = match key.code {
            KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => current + 1,
            KeyCode::PageUp => current.saturating_sub(PAGE_ROWS),
            KeyCode::PageDown => current + PAGE_ROWS,
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => return false,
        };
        if target >= max {
            self.follow = true;
            self.scroll = max;
        } else {
            self.follow = false;
            self.scroll = target;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::workspace::SkippedBranch;
    use ratatui::crossterm::event::KeyModifiers;
    use std::time::Duration;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ok(forwarded: &[&str], skipped: &[&str]) -> SyncOutcome {
        SyncOutcome {
            fetch: FetchOutcome::Ok,
            forwarded: forwarded.iter().map(|s| s.to_string()).collect(),
            skipped: skipped
                .iter()
                .map(|s| SkippedBranch {
                    name: s.to_string(),
                    reason: SkipReason::CheckedOutAt(PathBuf::from("/w/x")),
                })
                .collect(),
        }
    }

    fn failed(stderr: &str) -> SyncOutcome {
        SyncOutcome {
            fetch: FetchOutcome::Failed {
                exit_code: Some(128),
                stderr: stderr.to_string(),
            },
            forwarded: vec![],
            skipped: vec![],
        }
    }

    fn report(n: usize) -> SyncReport {
        let repos: Vec<PathBuf> = (0..n)
            .map(|i| PathBuf::from(format!("/r/repo{}", i)))
            .collect();
        SyncReport::new(&repos)
    }

    #[test]
    fn title_counts_repos_by_phase() {
        let mut r = report(3);
        assert_eq!(r.title(), "Sync report \u{b7} 0 of 3");
        r.started(0);
        r.finished(0, ok(&[], &[]));
        assert_eq!(r.title(), "Sync report \u{b7} 1 of 3");
        r.finished(1, failed("fatal: nope"));
        r.finished(2, ok(&["main"], &["dev"]));
        r.finish();
        assert_eq!(r.title(), "Sync report \u{b7} 2 ok, 1 failed");
        let mut clean = report(2);
        clean.finished(0, ok(&[], &["main"]));
        clean.finished(1, ok(&["dev"], &[]));
        clean.finish();
        assert_eq!(
            clean.title(),
            "Sync report \u{b7} 2 ok",
            "skipped branches never count as failures"
        );
    }

    #[test]
    fn finish_marks_unfinished_rows_not_synced_and_lands_on_first_failure() {
        let mut r = report(4);
        r.started(0);
        r.finished(0, ok(&[], &[]));
        r.started(1);
        assert_eq!(r.cursor, 1, "cursor follows the syncing row");
        r.finish();
        assert_eq!(r.rows[1].phase, RowPhase::NotSynced);
        assert_eq!(r.rows[3].phase, RowPhase::NotSynced);
        assert!(r.done);
        assert_eq!(r.cursor, 1, "cursor lands on the first failed row");
        assert_eq!(r.title(), "Sync report \u{b7} 1 ok, 3 failed");
        assert_eq!(r.rows[1].detail().0, "sync worker stopped");
    }

    #[test]
    fn finish_with_no_failures_lands_on_first_row() {
        let mut r = report(2);
        r.started(1);
        r.finished(0, ok(&[], &[]));
        r.finished(1, ok(&[], &[]));
        r.finish();
        assert_eq!(r.cursor, 0);
    }

    #[test]
    fn row_columns_follow_the_outcome() {
        let mut r = report(4);
        r.finished(0, ok(&[], &["main"]));
        r.finished(1, ok(&["release/2.3", "hotfix"], &["a", "b"]));
        r.finished(
            2,
            failed("fatal: could not read Username for 'https://github.com': terminal prompts disabled\n"),
        );
        r.finished(
            3,
            SyncOutcome {
                fetch: FetchOutcome::TimedOut {
                    after: Duration::from_secs(60),
                    stderr: String::new(),
                },
                forwarded: vec![],
                skipped: vec![],
            },
        );
        assert_eq!(r.rows[0].glyph(), "\u{2713}");
        assert_eq!(r.rows[0].outcome_label(), "up to date");
        assert_eq!(
            r.rows[0].detail(),
            ("".to_string(), "1 skipped".to_string())
        );
        assert_eq!(r.rows[1].outcome_label(), "fast-forwarded");
        assert_eq!(
            r.rows[1].detail(),
            (
                "release/2.3, hotfix".to_string(),
                " \u{b7} 2 skipped".to_string()
            )
        );
        assert_eq!(r.rows[2].glyph(), "\u{2717}");
        assert_eq!(r.rows[2].outcome_label(), "fetch failed");
        assert_eq!(
            r.rows[2].detail().0,
            "could not read Username for 'https://github.com': terminal prompts disabled"
        );
        assert_eq!(r.rows[3].outcome_label(), "timed out");
        assert_eq!(r.rows[3].detail().0, "timed out after 60s");
        let waiting = report(1);
        assert_eq!(waiting.rows[0].glyph(), "\u{b7}");
        assert_eq!(waiting.rows[0].outcome_label(), "waiting");
    }

    #[test]
    fn pane_lines_by_outcome() {
        let mut r = report(3);
        r.finished(0, ok(&["a", "b"], &["main"]));
        r.finished(1, failed("fatal: bad thing\nsecond line\n"));
        r.finished(
            2,
            SyncOutcome {
                fetch: FetchOutcome::TimedOut {
                    after: Duration::from_secs(60),
                    stderr: String::new(),
                },
                forwarded: vec![],
                skipped: vec![],
            },
        );
        r.finish();
        r.cursor = 0;
        let pane = r.pane(200);
        assert_eq!(pane.lines[0].text, "repo0  /r/repo0");
        assert_eq!(pane.lines[1].text, "fetch ok \u{b7} fast-forwarded a, b");
        assert_eq!(
            pane.lines[2],
            PaneLine::dim("skipped main: checked out in a worktree at /w/x")
        );

        r.cursor = 1;
        let pane = r.pane(200);
        assert_eq!(
            pane.lines[1].text,
            "fetch failed (git exit 128) \u{b7} branch picker will use local refs"
        );
        assert_eq!(pane.lines[2].text, "fatal: bad thing");
        assert_eq!(pane.lines[3].text, "second line");

        r.cursor = 2;
        let pane = r.pane(200);
        assert_eq!(
            pane.lines[1].text,
            "fetch timed out after 60s, git was stopped \u{b7} branch picker will use local refs"
        );
        assert_eq!(pane.lines.len(), 2);
    }

    #[test]
    fn pane_opens_with_notice_when_every_fetch_failed() {
        let mut r = report(2);
        r.finished(0, failed("fatal: x"));
        r.finish();
        assert!(r.all_failed());
        let pane = r.pane(200);
        assert_eq!(pane.lines[0].text, SyncReport::notice_lines()[0]);
        assert_eq!(pane.lines[1].text, SyncReport::notice_lines()[1]);
        assert_eq!(pane.lines[2].text, "repo0  /r/repo0");
    }

    #[test]
    fn pane_wraps_long_lines() {
        let mut r = report(2);
        r.finished(
            0,
            failed("fatal: could not read Username for 'https://github.com': terminal prompts disabled"),
        );
        r.finished(1, ok(&[], &[]));
        r.finish();
        assert_eq!(r.cursor, 0);
        let pane = r.pane(66);
        assert!(
            pane.lines
                .iter()
                .all(|l| UnicodeWidthStr::width(l.text.as_str()) <= 66),
            "every pane line must fit the width: {:?}",
            pane.lines
        );
        assert_eq!(
            pane.lines[2].text,
            "fatal: could not read Username for 'https://github.com': terminal"
        );
        assert_eq!(pane.lines[3].text, "prompts disabled");
        let narrow = r.pane(20);
        assert!(
            narrow.lines.len() > pane.lines.len(),
            "a narrower pane wraps into more lines"
        );
        assert!(
            narrow
                .lines
                .iter()
                .all(|l| UnicodeWidthStr::width(l.text.as_str()) <= 20),
            "every pane line must fit the width: {:?}",
            narrow.lines
        );
    }

    #[test]
    fn fit_pane_keeps_leading_lines_and_reports_hidden_count() {
        let content = PaneContent {
            lines: (0..7)
                .map(|i| PaneLine::plain(format!("line {}", i)))
                .collect(),
        };
        let fitted = fit_pane(&content, 4);
        assert_eq!(fitted.len(), 4);
        assert_eq!(fitted[0].text, "line 0");
        assert_eq!(fitted[1].text, "line 1");
        assert_eq!(fitted[2].text, "line 2");
        assert_eq!(fitted[3], PaneLine::dim("\u{2026} 4 more lines"));
        assert_eq!(fit_pane(&content, 7).len(), 7, "everything fits untouched");
        let one_spare = fit_pane(&content, 3);
        assert_eq!(one_spare[2], PaneLine::dim("\u{2026} 5 more lines"));
    }

    #[test]
    fn fit_pane_always_ends_with_the_marker_when_lines_are_cut() {
        let content = PaneContent {
            lines: (0..7)
                .map(|i| PaneLine::plain(format!("line {}", i)))
                .collect(),
        };
        // Exactly the header and status rows: the marker takes the status row.
        let tight = fit_pane(&content, 2);
        assert_eq!(tight.len(), 2);
        assert_eq!(tight[0].text, "line 0", "the header is kept");
        assert_eq!(tight[1], PaneLine::dim("\u{2026} 6 more lines"));
        // A single row shows only the marker.
        assert_eq!(
            fit_pane(&content, 1),
            vec![PaneLine::dim("\u{2026} 7 more lines")]
        );
        assert!(fit_pane(&content, 0).is_empty());
        // The count always equals the lines not on screen.
        for rows in 1..7 {
            let fitted = fit_pane(&content, rows);
            assert_eq!(fitted.len(), rows);
            assert_eq!(
                fitted[rows - 1],
                PaneLine::dim(format!("\u{2026} {} more lines", 7 - (rows - 1))),
                "rows {}",
                rows
            );
        }
    }

    #[test]
    fn exit_label_only_claims_what_the_outcome_proves() {
        fn status_line(exit_code: Option<i32>, stderr: &str) -> String {
            let mut r = report(1);
            r.finished(
                0,
                SyncOutcome {
                    fetch: FetchOutcome::Failed {
                        exit_code,
                        stderr: stderr.to_string(),
                    },
                    forwarded: vec![],
                    skipped: vec![],
                },
            );
            r.rows[0].pane_lines()[1].text.clone()
        }
        let suffix = " \u{b7} branch picker will use local refs";
        assert_eq!(
            status_line(Some(128), "fatal: x"),
            format!("fetch failed (git exit 128){}", suffix)
        );
        assert_eq!(
            status_line(
                None,
                "failed to spawn git: No such file or directory (os error 2)"
            ),
            format!("fetch failed (git did not start){}", suffix)
        );
        for stderr in ["", "sync cancelled", "could not wait for git\n", "fatal: x"] {
            assert_eq!(
                status_line(None, stderr),
                format!("fetch failed (no exit code){}", suffix),
                "stderr {:?}",
                stderr
            );
        }
        // The DETAIL column falls back to the same label when git wrote nothing.
        let mut r = report(1);
        r.finished(
            0,
            SyncOutcome {
                fetch: FetchOutcome::Failed {
                    exit_code: None,
                    stderr: String::new(),
                },
                forwarded: vec![],
                skipped: vec![],
            },
        );
        assert_eq!(r.rows[0].detail().0, "no exit code");
    }

    #[test]
    fn footer_drops_segments_in_order() {
        let mut r = report(3);
        assert_eq!(r.footer(None, 80), "ESC cancel");
        r.finished(0, ok(&[], &[]));
        r.finish();
        let full =
            "\u{2191}\u{2193} select \u{b7} PgUp/PgDn page \u{b7} ENTER continue \u{b7} ESC back";
        assert_eq!(UnicodeWidthStr::width(full), 54);
        assert_eq!(r.footer(None, 58), full);
        assert_eq!(
            r.footer(Some((1, 3)), 58),
            full,
            "the prefix drops first when it does not fit"
        );
        assert_eq!(
            r.footer(Some((1, 3)), 80),
            format!("rows 1\u{2013}3 of 3 \u{b7} {}", full)
        );
        assert_eq!(
            r.footer(None, 40),
            "\u{2191}\u{2193} select \u{b7} ENTER continue \u{b7} ESC back"
        );
        assert_eq!(r.footer(None, 30), "ENTER continue \u{b7} ESC back");
        assert_eq!(
            r.footer(None, 5),
            "ENTER continue \u{b7} ESC back",
            "ENTER and ESC never drop"
        );
    }

    #[test]
    fn all_failed_footer_swaps_in_continue_anyway_and_drops_page_at_minimum_width() {
        let mut r = report(1);
        r.finished(0, failed("fatal: x"));
        r.finish();
        let full = "\u{2191}\u{2193} select \u{b7} PgUp/PgDn page \u{b7} ENTER continue anyway \u{b7} ESC back";
        assert_eq!(UnicodeWidthStr::width(full), 61);
        assert_eq!(r.footer(None, 61), full);
        assert_eq!(
            r.footer(None, 58),
            "\u{2191}\u{2193} select \u{b7} ENTER continue anyway \u{b7} ESC back"
        );
    }

    #[test]
    fn cursor_keys_are_ignored_until_done() {
        let mut r = report(30);
        assert!(!r.handle_key(key(KeyCode::Down)));
        assert_eq!(r.cursor, 0);
        r.finish();
        assert!(r.handle_key(key(KeyCode::Down)));
        assert_eq!(r.cursor, 1);
        r.handle_key(key(KeyCode::PageDown));
        assert_eq!(r.cursor, 11);
        r.handle_key(key(KeyCode::End));
        assert_eq!(r.cursor, 29);
        r.handle_key(key(KeyCode::Down));
        assert_eq!(r.cursor, 29, "clamped at the last row");
        r.handle_key(key(KeyCode::PageUp));
        assert_eq!(r.cursor, 19);
        r.handle_key(key(KeyCode::Home));
        assert_eq!(r.cursor, 0);
        r.handle_key(key(KeyCode::Up));
        assert_eq!(r.cursor, 0, "clamped at the first row");
    }

    #[test]
    fn visible_window_keeps_cursor_on_screen() {
        let mut r = report(12);
        r.finish();
        assert_eq!(r.visible_window(5), (0, 5));
        r.cursor = 4;
        assert_eq!(r.visible_window(5), (0, 5));
        r.cursor = 5;
        assert_eq!(r.visible_window(5), (1, 6));
        r.cursor = 11;
        assert_eq!(r.visible_window(5), (7, 12));
        assert_eq!(r.visible_window(20), (0, 12));
    }

    #[test]
    fn report_layout_splits_list_first_then_pane() {
        // Minimum dialog: inner 8 = 3 list + blank + rule + 2 pane + footer.
        assert_eq!(report_layout(8, 4, 2), (3, 2));
        // Content below the cap: exactly what each part needs.
        assert_eq!(report_layout(4 + 3 + 3, 4, 3), (4, 3));
        // One repo with the spec's 7-line ssh pane on a 24-row terminal:
        // the list floor plus the pane fit, so the pane is not capped.
        assert_eq!(report_layout(3 + 7 + 3, 1, 7), (3, 7));
        // At the cap the pane takes up to half the inner height.
        assert_eq!(report_layout(17, 20, 12), (6, 8));
        // Few repos: rows the list cannot use go to the pane.
        assert_eq!(report_layout(17, 2, 12), (2, 12));
        assert_eq!(report_layout(2, 5, 5), (0, 0));
    }

    #[test]
    fn report_layout_gives_unused_list_rows_to_the_pane_at_the_minimum_height() {
        // A 12-row terminal clamps the dialog to 10 rows: inner 8, body 5.
        // The pane needs 9 lines (notice, header, status, five stderr lines).
        assert_eq!(
            report_layout(8, 1, 9),
            (1, 4),
            "one repo: two spare list rows"
        );
        assert_eq!(
            report_layout(8, 2, 9),
            (2, 3),
            "two repos: one spare list row"
        );
        assert_eq!(
            report_layout(8, 3, 9),
            (3, 2),
            "a full list keeps its floor"
        );
        assert_eq!(
            report_layout(8, 5, 9),
            (3, 2),
            "a longer list scrolls instead"
        );
    }

    #[test]
    fn log_view_tail_follows_and_end_resumes() {
        let mut log = LogView::new();
        assert_eq!(log.offset(30, 16), 14, "following shows the tail");
        assert!(log.handle_key(key(KeyCode::Up), 30));
        assert!(!log.follow);
        assert_eq!(log.offset(30, 16), 13);
        log.handle_key(key(KeyCode::PageUp), 30);
        assert_eq!(log.offset(30, 16), 3);
        log.handle_key(key(KeyCode::Home), 30);
        assert_eq!(log.offset(30, 16), 0);
        assert_eq!(
            log.offset(40, 16),
            0,
            "new lines do not move a detached view"
        );
        log.handle_key(key(KeyCode::End), 40);
        assert!(log.follow);
        assert_eq!(log.offset(40, 16), 24);
        assert_eq!(log.offset(45, 16), 29, "following tracks new lines");
        log.handle_key(key(KeyCode::Up), 45);
        log.handle_key(key(KeyCode::Down), 45);
        assert!(log.follow, "scrolling back to the tail resumes following");
        assert!(!log.handle_key(key(KeyCode::Enter), 45));
    }

    #[test]
    fn truncate_and_wrap_measure_display_width() {
        assert_eq!(truncate_to("abcdef", 4), "abc\u{2026}");
        assert_eq!(truncate_to("abc", 4), "abc");
        assert_eq!(truncate_to("abc", 0), "");
        assert_eq!(wrap_line("one two three", 8), vec!["one two", "three"]);
        assert_eq!(wrap_line("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap_line("", 4), vec![""]);
    }
}
