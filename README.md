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
┌─ Workspaces (25%) ──────────┬─ my-feature (vs base) ────────────────────┐
│  my-feature                 │  ▶ api-service  feat/x  clean  +142 -20   │
│  hotfix-payment             │  ▼ sak          feat/x  2 modified  +38 -4 │
│  ...                        │    M src/main.rs          [staged]  +12 -4 │
│                             │    A src/new.rs            [staged]  +26 -0 │
└─────────────────────────────┴───────────────────────────────────────────┘
 enter expand · ←/esc back · T switch to HEAD · q quit
```

Repos show total line divergence from the base branch (`+N -M` in green/red).
The `STATUS` column uses plain-language summaries like `2 modified` or
`14 modified, 3 new`.
Press `→` or `Enter` on a repo to expand it and see per-file diffs.

**Key bindings — Workspaces pane:**

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate workspaces |
| `→` | Focus repos pane |
| `Enter` | Go to selected workspace (cd) |
| `c` | Create a new workspace |
| `a` | Add repos to selected workspace |
| `d` | Delete selected workspace |
| `r` | Refresh repo cache |
| `/` | Search all repos |
| `S` | Open config editor |
| `q` / `Esc` | Quit |

**Key bindings — Repos pane:**

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate repos and file rows |
| `→` or `Enter` | Expand / collapse repo to show file-level diffs |
| `←` or `Esc` | Collapse all expanded repos, then refocus workspaces pane |
| `T` | Toggle diff target: base branch ↔ HEAD (uncommitted changes) |
| `q` | Quit |

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

## Development

Before pushing changes, run the same local verification sequence used by CI:

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## License

MIT
