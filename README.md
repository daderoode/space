# space

A CLI workspace manager for multi-repo git worktrees.

`space` lets you create named workspaces that group multiple repositories into
git worktrees checked out on the same branch — so you can switch between feature
work across many repos in a single `space go` command.

## Install

```sh
brew install daderoode/tap/space
```

## Setup (zsh)

Add the shell wrapper to your `.zshrc` so `space go` can change your working
directory:

```zsh
space() {
  local out
  out=$(command space "$@")
  if [[ $out == __SPACE_CD__:* ]]; then
    cd "${out#__SPACE_CD__:}"
  else
    echo "$out"
  fi
}
```

Then generate completions:

```sh
space completions zsh > ~/.zfunc/_space
```

## Usage

```
space ls [--verbose]           # list workspaces
space go [name]                # cd into a workspace (fuzzy picker if no name)
space status <name>            # show repo status for a workspace
space create [repos...]        # create a new workspace with worktrees
space add <workspace> <repos>  # add repos to an existing workspace
space rm <name> [--force]      # remove a workspace
space repos [--refresh]        # list / refresh the repo cache
space config                   # edit configuration interactively
space completions <zsh|bash|fish>
```

## Configuration

On first run `space` writes defaults to `~/.config/space/config.toml`:

```toml
[repos]
roots = ["~/work", "~/StudioProjects"]
max_depth = 3
cache_age_secs = 3600

[workspaces]
dir = "~/workspaces"
```

Run `space config` to edit interactively, or edit the file directly.

## How it works

Each workspace is a directory under `workspaces.dir`. Creating a workspace
runs `git worktree add` for each selected repo, placing the worktrees at
`<workspaces_dir>/<workspace>/<repo>`. Removing a workspace runs
`git worktree remove` and deletes the directory.

## License

MIT
