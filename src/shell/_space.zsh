#compdef space

_space_workspace_names() {
  local -a workspaces
  workspaces=("${(@f)$(command space __complete workspaces 2>/dev/null)}")
  (( ${#workspaces} )) && _describe 'workspace' workspaces
}

_space_repo_names() {
  local -a repos
  repos=("${(@f)$(command space __complete repos 2>/dev/null)}")
  (( ${#repos} )) && _describe 'repo' repos
}

_space_available_repos() {
  local workspace="$words[3]"
  local -a repos
  repos=("${(@f)$(command space __complete available-repos "$workspace" 2>/dev/null)}")
  (( ${#repos} )) && _describe 'repo' repos
}

_space() {
  local -a subcmds
  subcmds=(
    'ls:List workspaces'
    'status:Show workspace detail'
    'st:Show workspace detail'
    'go:cd into a workspace'
    'create:Create a new workspace'
    'add:Add repos to an existing workspace'
    'rm:Remove a workspace'
    'remove:Remove a workspace'
    'repos:List discoverable repos'
    'config:Edit configuration interactively'
    'completions:Generate shell completions'
    'init:Output shell init script'
    'mcp:Start MCP server on stdio'
  )

  if (( CURRENT == 2 )); then
    _describe 'command' subcmds
    return 0
  fi

  local cmd="${words[2]}"
  case "$cmd" in
    ls|list)
      _arguments '(-v --verbose)'{-v,--verbose}'[Show detailed information]'
      ;;
    status|st|go)
      _arguments '1:workspace:_space_workspace_names'
      ;;
    rm|remove)
      _arguments \
        '1:workspace:_space_workspace_names' \
        '(-f --force)'{-f,--force}'[Skip confirmation]'
      ;;
    add)
      if (( CURRENT == 3 )); then
        _space_workspace_names
      else
        _space_available_repos
      fi
      ;;
    create)
      _space_repo_names
      ;;
    repos)
      _arguments '(-r --refresh)'{-r,--refresh}'[Rescan repo roots]'
      ;;
    completions)
      compadd -- zsh
      ;;
    init)
      compadd -- zsh
      ;;
    config|mcp)
      # No arguments
      ;;
  esac
}

if (( $+functions[compdef] )); then
  compdef _space space
fi
