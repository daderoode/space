# space

A CLI workspace manager for multi-repo git worktrees.

`space` lets you create named workspaces that group multiple repositories into
git worktrees checked out on the same branch — so you can switch between feature
work across many repos in a single `space go` command.

Running `space` with no arguments opens the TUI dashboard.

## Install

```sh
brew install daderoode/tap/space
```

## Setup (zsh)

Add the shell wrapper to your `.zshrc`. This is required for `space go` to
change directories and for TUI commands to render correctly:

```zsh
space() {
  case "${1:-}" in
    ls|list|status|st|repos|completions|--version|--help|-h|-V)
      command space "$@"
      ;;
    *)
      local cdfile="${TMPDIR:-/tmp}/.space_cd_$$"
      __SPACE_CD_FILE__="$cdfile" command space "$@"
      local ret=$?
      if [[ -s "$cdfile" ]]; then
        cd -- "$(<"$cdfile")"
      fi
      rm -f "$cdfile" 2>/dev/null
      return $ret
      ;;
  esac
}
```

Then generate completions:

```sh
space completions zsh > ~/.zfunc/_space
```

## TUI Dashboard

Running `space` (no arguments) opens an interactive terminal dashboard with two
panes:

```
┌─ Workspaces (30%) ──────────┬─ Repos (70%) ─────────────────────────────┐
│  my-feature                 │  repo-a     main  ✓ clean                 │
│  hotfix-payment             │  repo-b     main  M 3 staged               │
│  ...                        │  ...                                       │
└─────────────────────────────┴───────────────────────────────────────────┘
 space v0.2.0                                    [status message]
```

**Key bindings:**

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate list |
| `Tab` | Switch panes (workspaces ↔ repos) |
| `Enter` | Go to selected workspace (cd) |
| `c` | Create a new workspace |
| `a` | Add repos to selected workspace |
| `d` | Delete selected workspace |
| `r` | Refresh repo cache |
| `/` | Search all repos |
| `q` / `Esc` | Quit |

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
space completions zsh
```

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

## License

MIT
