#compdef space

_space_workspace_names() {
  local workspaces_dir
  local ws_path

  workspaces_dir="$(_space_workspaces_dir)"

  [[ -d "$workspaces_dir" ]] || return 0

  for ws_path in "$workspaces_dir"/*(/N); do
    print -r -- "${ws_path:t}"
  done
}

_space_workspaces_dir() {
  local config_file="${XDG_CONFIG_HOME:-$HOME/.config}/space/config.toml"
  local workspaces_dir="$HOME/workspaces"
  local parsed

  if [[ -r "$config_file" ]]; then
    parsed="$(grep -E '^\s*workspaces_dir\s*=' "$config_file" | head -1 | sed 's/.*=\s*"\(.*\)"/\1/')"
    [[ -n "$parsed" ]] && workspaces_dir="$parsed"
  fi

  print -r -- "$workspaces_dir"
}

_space_repo_basenames() {
  local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/space"
  local repo_cache="$config_dir/repos.cache"
  local repo_path

  [[ -r "$repo_cache" ]] || return 0

  while IFS= read -r repo_path; do
    [[ -n "$repo_path" ]] && print -r -- "${repo_path:t}"
  done < "$repo_cache"
}

_space_available_workspace_repos() {
  local workspace_name="$1"
  local workspace_dir="$(_space_workspaces_dir)/$workspace_name"
  local repo_name repo_dir
  local -A existing_repos

  if [[ -d "$workspace_dir" ]]; then
    for repo_dir in "$workspace_dir"/*(/N); do
      existing_repos[${repo_dir:t}]=1
    done
  fi

  while IFS= read -r repo_name; do
    [[ -n "$repo_name" && -z "${existing_repos[$repo_name]-}" ]] && print -r -- "$repo_name"
  done < <(_space_repo_basenames)
}

_space() {
  local cmd="${words[2]-}"
  local -a candidates

  if (( CURRENT == 2 )); then
    candidates=(ls list status st add rm remove go repos create config completions help)
    compadd -- "${candidates[@]}"
    return 0
  fi

  case "$cmd" in
    ls|list)
      if (( CURRENT == 3 )); then
        compadd -- -v --verbose
      fi
      ;;
    status|st|go)
      if (( CURRENT == 3 )); then
        candidates=("${(@f)$(_space_workspace_names)}")
        (( ${#candidates[@]} > 0 )) && compadd -- "${candidates[@]}"
      fi
      ;;
    rm|remove)
      if (( CURRENT == 3 )); then
        candidates=("${(@f)$(_space_workspace_names)}")
        (( ${#candidates[@]} > 0 )) && compadd -- "${candidates[@]}"
      elif (( CURRENT == 4 )); then
        compadd -- -f --force
      fi
      ;;
    add)
      if (( CURRENT == 3 )); then
        candidates=("${(@f)$(_space_workspace_names)}")
        (( ${#candidates[@]} > 0 )) && compadd -- "${candidates[@]}"
      elif (( CURRENT == 4 )); then
        candidates=("${(@f)$(_space_available_workspace_repos "$words[3]")}")
        (( ${#candidates[@]} > 0 )) && compadd -- "${candidates[@]}"
      fi
      ;;
    repos)
      if (( CURRENT == 3 )); then
        compadd -- -r --refresh
      fi
      ;;
    create)
      # Takes free-form repo queries — no meaningful completions
      return 0
      ;;
    config)
      # No arguments
      return 0
      ;;
    completions)
      if (( CURRENT == 3 )); then
        compadd -- zsh
      fi
      ;;
  esac
}

if (( $+functions[compdef] )); then
  compdef _space space
fi
