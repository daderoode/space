# space Roadmap

Prioritised backlog for maturing space into a feature-rich multi-repo workspace
manager. Ordered so each tier builds on the previous one.

---

## Tier 1 -- Foundation

### 1. Test coverage (golden sample)

Lock down the current working behavior before making any structural or feature
changes. These tests are the safety net for everything that follows.

**Problem:** No CLI integration tests, no TUI state transition tests, no MCP tool
invocation tests. The most complex user flows (create, add) have zero automated
coverage.

**Approach:**

- **CLI tests:** Add `assert_cmd` + `predicates` as dev dependencies. Test `ls`,
  `status`, `repos`, `go`, `rm --force` against temp-dir workspaces. Verify exit
  codes and stdout content.
- **TUI tests:** Test `App::update()` with synthetic `Message` sequences. Assert
  resulting screen, stage, and state without needing a real terminal. Cover the
  create and add happy paths.
- **MCP tests:** Spin up the server in-process, send JSON-RPC tool calls, assert
  responses. Cover all 6 tools.
- **Enforce cache_age_secs:** Implement the TTL check in `load_cache()` (mtime vs
  configured TTL), then add a test that verifies stale caches trigger rescans.

**New dev dependencies:** `assert_cmd`, `predicates`
**Files:** `tests/cli_test.rs` (new), `tests/tui_test.rs` (new), expand
`tests/mcp_test.rs`
**Done when:** `cargo test` covers all CLI commands, TUI create/add flows, and MCP
tools.

---

### 2. Split app.rs into per-screen key handlers

**Problem:** `src/tui/app.rs` is 1,377 lines. Every screen's key handling,
borrow-checker workarounds (~30 instances), and state transitions live in one file.
Adding features makes it worse.

**Approach:**

- Each screen module (`screens/create.rs`, `screens/add.rs`, etc.) gains a
  `handle_key(&mut self, ...) -> Option<Message>` method on its state struct.
- `app.rs` dispatches to the active screen's handler instead of matching inline.
- The repetitive extract-stage-match-reborrow pattern disappears because the
  state struct owns its own mutation.
- De-duplicate the shared Create/Add stages (PickBranchStrategy, PickBranch,
  Creating) into a common worktree-addition flow that both screens compose.

**Files:** `src/tui/app.rs`, all `src/tui/screens/*.rs`
**Done when:** `app.rs` is under 400 lines and each screen is self-contained.
**Safety:** The golden sample tests from step 1 catch any regressions.

---

## Tier 2 -- Core Features

### 3. Repo groups / templates

**Problem:** Users repeatedly create workspaces with the same repo combinations.
Every time they must re-select the same repos.

**Approach:**

- Add a `[groups]` table to `config.toml`:
  ```toml
  [groups]
  payments = ["payment-gateway", "transaction-service", "ledger"]
  frontend = ["web-app", "component-lib", "design-tokens"]
  ```
- CLI: `space create --group payments` pre-selects those repos, skipping the picker.
- TUI: New stage before PickRepos offering "Pick group or manual select."
- MCP: `create_workspace` gains an optional `group` parameter.
- Groups reference repo names (matched against cache, same fuzzy logic as MCP's
  `resolve_repos`).

**Files:** `src/core/config.rs`, `src/cli/mod.rs`, `src/tui/screens/create.rs`,
`src/tui/app.rs`, `src/mcp/mod.rs`
**Done when:** Groups are configurable, selectable in all 3 interfaces, and tested.

---

### 4. Post-create hooks

**Problem:** After creating worktrees, users manually run setup commands
(`npm install`, `make setup`, copy `.env`, etc.) in every repo.

**Approach:**

- Add a `[hooks]` section to config:
  ```toml
  [hooks]
  post_create = "make setup"
  ```
- After each worktree is successfully created, run the hook command in that
  worktree's directory.
- TUI shows hook output in the Creating stage's progress log.
- CLI prints hook output to stdout.
- MCP includes hook results in the response.
- Hooks are optional. If unset or empty, skip silently.
- Hooks run with the worktree directory as CWD and inherit the user's environment.

**Files:** `src/core/config.rs`, `src/core/workspace.rs`, `src/tui/app.rs`,
`src/mcp/mod.rs`
**Done when:** Hooks fire after worktree creation in all 3 interfaces and failures
are reported without aborting the workspace creation.

---

### 5. space sync

**Problem:** After working in a multi-repo workspace, repos can drift -- different
branches, unpushed commits, unfetched changes.

**Approach:**

- New CLI command: `space sync [name]`
  - Fetch all repos in the workspace.
  - Report branch consistency (all on same branch? any diverged?).
  - Report ahead/behind for each repo vs upstream.
  - Optional `--pull` flag to pull all repos (abort on conflicts, report which).
- TUI: Keybind on dashboard (e.g., `s`) to run sync for selected workspace.
  Show results as a transient overlay or status summary.
- MCP: New `sync_workspace` tool.

**Files:** `src/core/workspace.rs` (new sync function), `src/cli/sync.rs` (new),
`src/tui/app.rs`, `src/mcp/mod.rs`
**Done when:** `space sync` fetches, reports divergence, and optionally pulls across
all repos in a workspace.

---

## Tier 3 -- Power Features

### 6. space diff

**Problem:** Before creating PRs for a multi-repo feature, there's no way to see the
total change footprint across all repos at once.

**Approach:**

- New CLI command: `space diff [name]`
  - For each repo: run `git diff --stat` (or use libgit2 diff APIs).
  - Output aggregate summary: files changed, insertions, deletions per repo.
  - Optional `--name-only` for file list only.
- TUI: Keybind on dashboard (e.g., `d`) to show diff summary overlay.
- MCP: New `workspace_diff` tool returning structured diff stats.

**Files:** `src/core/git.rs` (new diff stat function), `src/cli/diff.rs` (new),
`src/tui/app.rs`, `src/tui/ui.rs`, `src/mcp/mod.rs`
**Done when:** `space diff` shows per-repo and total change stats for a workspace.

---

### 7. Stale worktree cleanup

**Problem:** Worktrees accumulate as features finish. Branches get merged or deleted
upstream, but the local worktrees remain.

**Approach:**

- New function in `core/workspace.rs`: detect worktrees whose branch has been
  merged into the base branch or deleted on the remote.
- TUI dashboard: Show a visual indicator (dimmed, strikethrough, or icon) for stale
  worktrees. Keybind to clean up all stale worktrees with confirmation.
- CLI: `space cleanup [--dry-run]` lists or removes stale worktrees.
- Detection logic: `git branch --merged <base>` for merged, check if upstream ref
  still exists for deleted.

**Files:** `src/core/workspace.rs`, `src/core/git.rs`, `src/cli/cleanup.rs` (new),
`src/tui/app.rs`, `src/tui/ui.rs`
**Done when:** Stale worktrees are detected, surfaced in the TUI, and removable via
CLI and TUI.

---

### 8. Bash and fish completions

**Problem:** Only zsh has completion support. Bash and fish users get nothing.

**Approach:**

- Use `clap_complete` to generate completions for bash and fish from the existing
  clap `Cli` definition.
- Add `space completions bash` and `space completions fish` subcommands.
- Write shell wrapper equivalents for bash and fish (the cd-target protocol needs
  a wrapper function in each shell).
- Update README and GUIDE with setup instructions for all 3 shells.

**Files:** `src/shell/mod.rs`, `src/shell/completions.rs`, new bash/fish wrapper
scripts, `README.md`, `docs/GUIDE.md`
**Done when:** All 3 shells have working completions and cd-wrapper functions.
