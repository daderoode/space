# space -- Project Review

Reviewed at v0.4.0, 2026-03-23.

---

## What It Is

A workspace manager for multi-repo git worktrees. One command to set up a
directory of git worktrees for multiple repos all on the same branch, one command
to tear it down. No metadata database -- the filesystem is the state.

Three interfaces sharing one core:
- **TUI Dashboard** (default, no args) -- ratatui-based interactive terminal UI
- **CLI Commands** -- direct commands for scripting (`space ls`, `space go`, etc.)
- **MCP Server** (`space mcp`) -- Model Context Protocol server for AI coding agents

---

## What's Working Well

### Architecture

- **Clean layering.** `core/` has zero TUI/CLI dependencies. `cli/`, `tui/`, and
  `mcp/` all depend on `core/` but not on each other. Enforced by module structure.
- **Filesystem-as-state.** No database, no `.space` metadata files. The directory
  structure is the source of truth. Smart design that eliminates an entire class of
  sync bugs.
- **Elm architecture in TUI.** `Message` enum -> `update()` -> state mutation ->
  `view()` rendering. Messages chain via `Option<Message>` return. Sound pattern
  for a stateful TUI.
- **Enums for state machines.** `CreateStage` (5 variants), `AddStage` (4 variants),
  `BranchStrategy` (3 variants), `Screen` (7 variants) -- all exhaustively matched.
  The compiler prevents invalid states.

### UX

- **Nucleo fuzzy matching with scope.** The `org/query` syntax and `Ctrl-S` scope
  cycling is genuinely useful. Match character highlighting gives good feedback.
- **CD protocol.** The temp-file approach (`__SPACE_CD_FILE__`) keeps stdout free
  for TUI rendering. Cleaner than most tools' shell integration.
- **Graceful degradation.** Offline-safe (fetch errors silently swallowed), detached
  HEAD handled, missing upstream handled. The tool doesn't panic when the network
  is unavailable.

### Documentation

- **README.md (179 lines).** Install, shell setup, TUI mockup, key bindings, full
  command reference, MCP config, dev instructions. Complete.
- **GUIDE.md (810 lines).** Comprehensive user guide with architecture diagram,
  worktree internals, MCP tool reference with example responses, 7 use case
  scenarios. Publication-quality.
- **Code comments.** Doc comments on all public `core/` functions. Inline comments
  on non-obvious decisions (stderr parsing, crossterm/tui_input version mismatch,
  input buffer draining).

### Engineering

- **Zero `unsafe` code.**
- **CI enforces `clippy -D warnings` and `cargo fmt --check`.**
- **Release pipeline** cross-compiles for Apple Silicon + Intel, generates checksums,
  publishes to GitHub Releases and Homebrew.
- **Release profile** strips symbols and uses `opt-level = 3`.

---

## Areas for Improvement

### 1. `app.rs` is 1,377 lines

Every screen's key handler lives in one file. The borrow-checker workaround pattern
(extract stage from `app.screen`, match, re-borrow) appears ~30 times. Each
`handle_*_key()` function should move into its respective screen module, with the
state struct owning its key handling.

### 2. Duplicated Create and Add flows

`handle_create_key()` and `handle_add_key()` share near-identical structure for
`PickBranchStrategy`, `PickBranch`, and `Creating` stages. `do_create()` and
`do_add()` are structurally identical. The shared stages should be extracted into
a common worktree-addition flow.

### 3. Test coverage gaps

| Area | Current state |
|------|---------------|
| CLI integration | No tests. No binary invocation, no stdout assertion. |
| TUI state transitions | One inline test (status message TTL). No create/add flow coverage. |
| MCP tool invocation | Only unit tests for `resolve_repos` and `build_strategy`. No actual tool calls. |
| Create/Add happy paths | The most complex user flows have zero automated coverage. |
| Cache TTL | `cache_age_secs` is now enforced -- stale caches are rejected by `load_cache()`. |

### 4. Shell support is zsh-only

Only zsh has completions and a shell wrapper. No bash or fish support. `clap_complete`
would generate completions mechanically; the wrapper functions need manual equivalents.

### 5. macOS-only builds

The release pipeline only targets macOS. The code has no macOS-specific dependencies --
`dirs`, `git2`, `walkdir` all work on Linux. Adding Linux targets to CI and releases
doubles the potential user base with minimal effort.

### 6. No logging outside MCP

The MCP server uses `tracing`, but CLI and TUI have none. A `SPACE_LOG` env var
writing to a file (not stdout) would help debug user-reported issues. The
infrastructure is already in `Cargo.toml`.

### 7. No input validation on workspace names

Any non-empty string is accepted. Characters like `..` or null bytes could cause
problems. Decide whether `feature/auth` style nested naming is intentional and
validate accordingly.

---

## Feature Opportunities

These are ordered by value-to-effort ratio. See `docs/ROADMAP.md` for the full
prioritised backlog.

| Feature | Value |
|---------|-------|
| **Repo groups/templates** | Named repo sets in config. Eliminates re-selecting the same repos every time. |
| **Post-create hooks** | Run `npm install`, `make setup`, etc. after worktree creation. Eliminates the most common manual step. |
| **`space sync`** | Branch consistency check + fetch/pull across all repos. Natural progression from `status`. |
| **`space diff`** | Cross-repo change summary before creating PRs. Total blast radius at a glance. |
| **Stale worktree cleanup** | Detect merged/deleted branches, surface in dashboard, offer one-click removal. |
| **Bash/fish completions** | Broaden shell support beyond zsh. |

---

## Summary

Solid foundation at v0.4.0. The architecture is right, the UX is thoughtful, and
the documentation is ahead of its weight class. The main gaps are test coverage
(the biggest risk), `app.rs` structural debt (the biggest drag on velocity), and
breadth (shell support, Linux builds). The feature opportunities are all natural
extensions of what already exists -- none require rethinking the core design.
