# space

A CLI workspace manager for multi-repo git worktrees.

`space` lets you create named workspaces that group multiple repositories into
git worktrees checked out on the same branch — so you can switch between feature
work across many repos in a single `space go` command.

Running `space` with no arguments opens the TUI dashboard.

For the full guide, see [docs/GUIDE.md](docs/GUIDE.md).

## Install

```sh
brew install daderoode/tap/space
```

## Setup (zsh)

Add to your `~/.zshrc`:

```zsh
eval "$(space init zsh)"
```

This sets up the shell wrapper (required for `space go` and TUI commands to
work correctly) and registers completions.

If you installed via Homebrew, completions are installed to Homebrew's zsh
site-functions directory. The `eval` line adds the shell wrapper needed for
`space go` and TUI commands, and also works as a fallback for completions.

## TUI Dashboard

Running `space` (no arguments) opens an interactive terminal dashboard with two
panes:

```
┌─ Workspaces (25%) ──────────┬─ my-feature ──────────────────────────────┐
│  my-feature                 │  ▶ api-service  feat/x  clean             │
│  hotfix-payment             │  ▼ sak          feat/x  2 modified  +38 -4 │
│  ...                        │    ── Unstaged ──                           │
│                             │    M src/main.rs                   +12 -4  │
│                             │    A src/new.rs                    +26 -0  │
└─────────────────────────────┴───────────────────────────────────────────┘
 enter expand · ←/esc back · h/l scroll · ? help · q quit
```

The `STATUS` column uses plain-language summaries like `2 modified` or
`14 modified, 3 new`.
Press `→` or `Enter` on a repo to expand it and see per-file diffs grouped
into Conflicts, Unstaged, and Staged sections. Expanded repos show total line
changes (`+N -M` in green/red) per file and in aggregate.
Press `Enter` on a file row to open a scrollable diff viewer. Stage/unstage
files with `s`/`space`, or bulk stage/unstage all files in the selected repo with `S`/`U`.

**Key bindings — Workspaces pane:**

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate workspaces |
| `→` | Focus repos pane |
| `Enter` | Go to selected workspace (cd) |
| `c` | Create a new workspace |
| `a` | Add repos to selected workspace |
| `d` | Delete selected workspace |
| `PgUp` / `PgDn` | Page up / down |
| `Home` / `End` | First / last workspace |
| `/` | Filter spaces (selects in place) |
| `S` | Open config editor |

**Key bindings — Repos pane:**

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate repos and file rows |
| `h` / `l` | Scroll table left / right (reveal long branch names) |
| `→` or `Enter` (on repo) | Expand / collapse repo to show file-level diffs |
| `Enter` (on file) | Open file diff viewer |
| `←` or `Esc` | Collapse all expanded repos, then refocus workspaces pane |
| `s` / `space` | Stage / unstage file |
| `S` | Stage all unstaged files in repo |
| `U` | Unstage all staged files in repo |
| `b` | Switch branch for selected repo |
| `PgUp` / `PgDn` | Page up / down |
| `Home` / `End` | First / last row |
| `/` | Search all repos |

**Key bindings, repo picker** (create and add flows):

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move the highlight (every letter types into the filter) |
| `Tab` | Toggle the highlighted repo |
| `Ctrl-S` | Cycle the parent-directory scope |
| `Ctrl-R` | Rescan the repo list without leaving the picker |

**Key bindings, general** (either pane):

| Key | Action |
|-----|--------|
| `r` | Rescan the repo list and reload the repos pane |
| `?` | Open help overlay |
| `q` | Quit |
| `Ctrl-C` | Force quit (works on every screen) |

Interactive commands (`go`, `create`, `add`, `config`, `rm` without `--force`)
also launch TUI flows when invoked from the command line.

## Usage

```
space                          # open TUI dashboard (default)
space ls [--verbose]           # list workspaces
space go [name]                # cd into a workspace (fuzzy picker if no name)
space status <name>            # show repo status for a workspace
space create [repos...]        # create a new workspace with worktrees
space add <workspace> <repos>  # add repos to an existing workspace
space rm <name> [--force]      # remove a workspace
space repos [--refresh]        # list / refresh the repo cache
space config                   # edit configuration interactively
space init zsh                 # output shell init script (wrapper + completions)
space completions zsh          # print shell completions only
space mcp                      # start MCP server on stdio
```

## MCP Server

`space mcp` starts a [Model Context Protocol](https://modelcontextprotocol.io)
server on stdio, exposing workspace management to AI coding agents.

**Tools:**

| Tool | Description |
|------|-------------|
| `list_repos` | Discover git repositories from configured roots |
| `list_workspaces` | List all workspaces with per-repo branch and status |
| `workspace_status` | Detailed status for a specific workspace |
| `create_workspace` | Create a workspace with git worktrees for selected repos |
| `add_repos` | Add repos to an existing workspace |
| `remove_workspace` | Remove a workspace and all its worktrees |

Add to your agent's MCP config:

```json
{
  "mcpServers": {
    "space": {
      "command": "space",
      "args": ["mcp"]
    }
  }
}
```

See [docs/GUIDE.md](docs/GUIDE.md#mcp-tools) for the full tool reference with
parameters, return types, and error handling.

## Configuration

On first run `space` writes defaults to `~/.config/space/config.toml`:

```toml
[repos]
roots = ["~/projects"]
max_depth = 3
cache_age_secs = 3600

[workspaces]
dir = "~/workspaces"
```

Run `space config` to edit interactively via the TUI:

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Move between fields |
| `Enter` | Edit focused field |
| `Esc` | Cancel edit |
| `Ctrl-S` | Save and exit |

The `Repo roots` field accepts a comma-separated list of paths (e.g. `~/projects, ~/work`).

Or edit `~/.config/space/config.toml` directly.

## How it works

Each workspace is a directory under `workspaces.dir`. Creating a workspace
runs `git worktree add` for each selected repo, placing the worktrees at
`<workspaces_dir>/<workspace>/<repo>`. Removing a workspace runs
`git worktree remove` and deletes the directory.

There is no metadata database — the filesystem is the state.

## Diagnostics

`space` writes diagnostic logs by default to help with bug reports:

| Platform | Default log directory |
|---|---|
| macOS | `~/Library/Application Support/space/` |
| Linux | `~/.local/share/space/` |

Files are named `space.log.YYYY-MM-DD`. The last 3 days are kept.

| Variable | Effect |
|---|---|
| `SPACE_LOG=off` | Disable logging entirely |
| `SPACE_LOG=/path/to/dir` | Write logs to a custom directory |
| `SPACE_LOG_LEVEL=debug` | Verbose logging (default: `info`) |
| `SPACE_LOG_LEVEL=off` | Disable logging (alternative to `SPACE_LOG=off`) |

When filing a bug report, include the log file from the day the issue occurred.

## Development

Before pushing changes, run the same local verification sequence used by CI:

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## License

MIT
