#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./commit_policy.sh
source "${SCRIPT_DIR}/commit_policy.sh"

ensure_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Required command '$1' is not available." >&2
    exit 1
  }
}

prompt_yes_no() {
  local question="$1"
  local default_no="${2:-true}"
  local hint="[y/N]"
  [[ "$default_no" == "false" ]] && hint="[Y/n]"
  while true; do
    read -r -p "$question $hint " answer
    answer="${answer,,}"
    if [[ -z "$answer" ]]; then
      [[ "$default_no" == "true" ]] && return 1 || return 0
    fi
    [[ "$answer" == "y" || "$answer" == "yes" ]] && return 0
    [[ "$answer" == "n" || "$answer" == "no" ]] && return 1
    echo "Please answer y or n."
  done
}

prompt_menu_selection() {
  local title="$1"
  shift
  local options=("$@")
  local count="${#options[@]}"
  (( count > 0 )) || {
    echo "No options available for selection." >&2
    exit 1
  }
  echo "$title"
  local i
  for ((i=0; i<count; i++)); do
    echo "  $((i + 1)). ${options[$i]}"
  done
  while true; do
    read -r -p "Choose option (1-${count}): " choice
    if [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= count )); then
      echo "${options[$((choice - 1))]}"
      return 0
    fi
    echo "Invalid selection."
  done
}

read_multiline_input() {
  local prompt="$1"
  echo "$prompt"
  echo "Submit by entering an empty line."
  local line
  while IFS= read -r line; do
    [[ -z "$line" ]] && break
    printf '%s\n' "$line"
  done
}

get_current_branch() {
  git rev-parse --abbrev-ref HEAD
}

get_working_tree_changes() {
  git status --porcelain
}

get_staged_paths() {
  git diff --cached --name-only
}

ensure_clean_working_tree() {
  if [[ -n "$(get_working_tree_changes)" ]]; then
    echo "Working tree has uncommitted changes. Commit/stash before running this task." >&2
    exit 1
  fi
}

ensure_branch_upstream() {
  local branch="$1"
  if ! git rev-parse --abbrev-ref --symbolic-full-name "@{u}" >/dev/null 2>&1; then
    echo "No upstream configured for '$branch'. Pushing with upstream tracking..."
    git push -u origin "$branch"
  fi
}

prompt_commit_entry() {
  local has_src_flag="$1"
  local allowed=()
  local t
  if [[ "$has_src_flag" == "true" ]]; then
    allowed=("${ALL_TYPES[@]}")
  else
    for t in "${ALL_TYPES[@]}"; do
      is_releasable_type "$t" || allowed+=("$t")
    done
  fi

  local selected
  selected="$(prompt_menu_selection "Select commit change type" "${allowed[@]}")"
  local desc=""
  while [[ -z "${desc// }" ]]; do
    read -r -p "Enter change description: " desc
  done
  local breaking=""
  if is_releasable_type "$selected"; then
    if prompt_yes_no "Mark this entry as breaking?" "true"; then
      breaking="!"
    fi
  fi
  printf '%s%s: %s\n' "$selected" "$breaking" "$desc"
}

build_commit_message_text() {
  local entries_file="$1"
  local freeform_file="$2"

  local subject
  subject="$(head -n 1 "$entries_file")"
  [[ -n "${subject// }" ]] || {
    echo "At least one change entry is required." >&2
    exit 1
  }

  printf '%s\n' "$subject"
  if [[ -s "$freeform_file" || "$(wc -l <"$entries_file" | tr -d ' ')" -gt 1 ]]; then
    printf '\n'
  fi

  if [[ -s "$freeform_file" ]]; then
    cat "$freeform_file"
    printf '\n'
  fi

  tail -n +2 "$entries_file"
}

commit_workflow() {
  ensure_command git
  git add -A

  mapfile -t staged_paths < <(get_staged_paths)
  if (( ${#staged_paths[@]} == 0 )); then
    echo "No staged changes found. Nothing to commit."
    return 0
  fi

  local has_src_flag="false"
  has_src_changes "${staged_paths[@]}" && has_src_flag="true"
  echo "Detected src changes: $has_src_flag"

  local entries_file freeform_file message_file
  entries_file="$(mktemp)"
  freeform_file="$(mktemp)"
  message_file="$(mktemp)"
  trap 'rm -f "$entries_file" "$freeform_file" "$message_file"' RETURN

  prompt_commit_entry "$has_src_flag" >>"$entries_file"
  if prompt_yes_no "Add additional change entries?" "true"; then
    while true; do
      prompt_commit_entry "$has_src_flag" >>"$entries_file"
      prompt_yes_no "Add another entry?" "true" || break
    done
  fi

  if prompt_yes_no "Add freeform body paragraphs?" "true"; then
    read_multiline_input "Enter freeform body lines" >"$freeform_file"
  fi

  build_commit_message_text "$entries_file" "$freeform_file" >"$message_file"
  local message_text
  message_text="$(<"$message_file")"
  validate_commit_message_text "$message_text" "$has_src_flag"

  git commit -F "$message_file"
}

push_workflow() {
  ensure_command git
  local branch
  branch="$(get_current_branch)"
  [[ "$branch" != "HEAD" ]] || {
    echo "Detached HEAD state detected. Checkout a branch first." >&2
    exit 1
  }

  if [[ -n "$(get_working_tree_changes)" ]]; then
    if prompt_yes_no "Uncommitted changes found. Run commit task before push?" "true"; then
      commit_workflow
    else
      echo "Leaving uncommitted changes untouched; pushing existing commits only."
    fi
  fi

  if git rev-parse --abbrev-ref --symbolic-full-name "@{u}" >/dev/null 2>&1; then
    git push
  else
    git push -u origin "$branch"
  fi
}

load_commit_records_since_base() {
  local branch="$1"
  local base_branch="$2"
  local out_file="$3"
  local base_ref="origin/$base_branch"
  git rev-parse --verify "$base_ref" >/dev/null 2>&1 || base_ref="$base_branch"
  local merge_base
  merge_base="$(git merge-base "$branch" "$base_ref")"
  [[ -n "$merge_base" ]] || {
    echo "Could not determine merge-base for '$branch' and '$base_ref'." >&2
    exit 1
  }
  local range="${merge_base}..${branch}"
  git log --reverse --format='%H%x1f%s%x1f%b%x1e' "$range" >"$out_file"
}

pr_workflow() {
  ensure_command git
  ensure_command gh
  local base_branch="${1:-main}"
  push_workflow

  local branch
  branch="$(get_current_branch)"
  [[ "$branch" != "HEAD" ]] || {
    echo "Detached HEAD state detected. Checkout a branch first." >&2
    exit 1
  }
  [[ "$branch" != "$base_branch" && "$branch" != "main" ]] || {
    echo "Refusing PR creation from '$branch'. Use a non-main branch." >&2
    exit 1
  }

  ensure_branch_upstream "$branch"
  ensure_clean_working_tree

  local existing_state
  existing_state="$(gh pr view "$branch" --json state --jq '.state' 2>/dev/null || true)"
  if [[ "$existing_state" == "OPEN" ]]; then
    local existing_url existing_number
    existing_url="$(gh pr view "$branch" --json url --jq '.url')"
    existing_number="$(gh pr view "$branch" --json number --jq '.number')"
    echo "PR already exists for '$branch': #$existing_number $existing_url"
    return 0
  fi

  local records_file
  records_file="$(mktemp)"
  trap 'rm -f "$records_file"' RETURN
  load_commit_records_since_base "$branch" "$base_branch" "$records_file"
  [[ -s "$records_file" ]] || {
    echo "No commits found between '$branch' and '$base_branch'." >&2
    exit 1
  }

  local entries_file freeform_map_file commit_subjects_file
  entries_file="$(mktemp)"
  freeform_map_file="$(mktemp)"
  commit_subjects_file="$(mktemp)"
  : >"$entries_file"
  : >"$freeform_map_file"
  : >"$commit_subjects_file"
  trap 'rm -f "$records_file" "$entries_file" "$freeform_map_file" "$commit_subjects_file"' RETURN

  local rec
  while IFS= read -r -d $'\x1e' rec; do
    [[ -z "${rec// }" ]] && continue
    IFS=$'\x1f' read -r hash subject body <<<"$rec"
    hash="$(trim_cr "$hash")"
    subject="$(trim_cr "$subject")"
    local msg="${subject}"
    [[ -n "${body:-}" ]] && msg+=$'\n'"${body%$'\n'}"

    local commit_entries_file commit_freeform_file
    commit_entries_file="$(mktemp)"
    commit_freeform_file="$(mktemp)"
    parse_message_entries_and_freeform "$msg" "$commit_entries_file" "$commit_freeform_file" || {
      rm -f "$commit_entries_file" "$commit_freeform_file"
      continue
    }

    while IFS= read -r entry || [[ -n "$entry" ]]; do
      [[ -z "${entry// }" ]] && continue
      printf '%s\t%s\n' "$hash" "$entry" >>"$entries_file"
    done <"$commit_entries_file"
    printf '%s\t%s\t%s\n' "$hash" "$subject" "$commit_freeform_file" >>"$freeform_map_file"
    printf '%s\t%s\n' "$hash" "$subject" >>"$commit_subjects_file"
    rm -f "$commit_entries_file"
  done <"$records_file"

  mapfile -t entry_rows <"$entries_file"
  (( ${#entry_rows[@]} > 0 )) || {
    echo "No change entries were discovered from branch commits." >&2
    exit 1
  }

  local options=()
  local row hash entry short
  for row in "${entry_rows[@]}"; do
    hash="${row%%$'\t'*}"
    entry="${row#*$'\t'}"
    short="${hash:0:7}"
    options+=("[$short] $entry")
  done

  local selected selected_index=-1 i
  selected="$(prompt_menu_selection "Select PR title change entry" "${options[@]}")"
  for ((i=0; i<${#options[@]}; i++)); do
    [[ "${options[$i]}" == "$selected" ]] && selected_index=$i && break
  done
  (( selected_index >= 0 )) || {
    echo "Could not resolve selected title entry." >&2
    exit 1
  }

  local selected_hash selected_title
  selected_hash="${entry_rows[$selected_index]%%$'\t'*}"
  selected_title="${entry_rows[$selected_index]#*$'\t'}"

  local include_freeform="false"
  if prompt_yes_no "Include freeform body paragraphs from commits in PR body?" "true"; then
    include_freeform="true"
  fi

  local pr_body_file
  pr_body_file="$(mktemp)"
  trap 'rm -f "$records_file" "$entries_file" "$freeform_map_file" "$commit_subjects_file" "$pr_body_file"' RETURN

  {
    echo "## Change Entries"
    local wrote_any="false"
    for ((i=0; i<${#entry_rows[@]}; i++)); do
      [[ "$i" -eq "$selected_index" ]] && continue
      echo "- ${entry_rows[$i]#*$'\t'}"
      wrote_any="true"
    done
    [[ "$wrote_any" == "true" ]] || echo "- (none)"

    if [[ "$include_freeform" == "true" ]]; then
      echo
      echo "## Additional Context"
      local found_ctx="false"
      while IFS=$'\t' read -r chash csubject cfile || [[ -n "${chash:-}" ]]; do
        [[ -f "$cfile" ]] || continue
        if [[ -s "$cfile" ]]; then
          found_ctx="true"
          echo "### ${chash:0:7} $csubject"
          cat "$cfile"
          echo
        fi
      done <"$freeform_map_file"
      [[ "$found_ctx" == "true" ]] || echo "(No freeform body paragraphs found in commit history.)"
    fi
  } >"$pr_body_file"

  gh pr create --base "$base_branch" --head "$branch" --title "$selected_title" --body-file "$pr_body_file"
}

assert_pr_ready_for_merge() {
  local branch="$1"
  local state
  state="$(gh pr view "$branch" --json state --jq '.state')"
  [[ "$state" == "OPEN" ]] || {
    echo "PR for '$branch' is not open." >&2
    exit 1
  }
  local merge_state
  merge_state="$(gh pr view "$branch" --json mergeStateStatus --jq '.mergeStateStatus')"
  case "$merge_state" in
    BLOCKED|DIRTY|DRAFT|UNKNOWN|BEHIND)
      echo "PR for '$branch' is not merge-ready (mergeStateStatus=$merge_state)." >&2
      exit 1
      ;;
  esac
  local checks_output
  if ! checks_output="$(gh pr checks "$branch" --required 2>&1)"; then
    echo "PR required checks are not ready. This task fails one-shot; rerun after checks pass." >&2
    echo "$checks_output" >&2
    exit 1
  fi
}

is_release_pr_branch() {
  local branch="$1"
  local title
  title="$(gh pr view "$branch" --json title --jq '.title')"
  [[ "$title" =~ ^chore:[[:space:]]+release([[:space:]]|$) ]]
}

merge_pr_workflow() {
  ensure_command git
  ensure_command gh
  local branch="${1:-}"
  local expect_release="${2:-false}"
  local base_branch="${3:-main}"
  local current_branch
  current_branch="$(get_current_branch)"
  [[ -n "${branch// }" ]] || branch="$(get_current_branch)"
  [[ "$branch" != "HEAD" ]] || {
    echo "Detached HEAD state detected. Checkout a branch first." >&2
    exit 1
  }
  [[ "$current_branch" == "$branch" ]] || {
    echo "Current branch '$current_branch' does not match requested merge branch '$branch'. Checkout '$branch' and rerun so pr->merge chaining targets the same branch." >&2
    exit 1
  }

  pr_workflow "$base_branch"
  branch="$(get_current_branch)"

  local is_release="false"
  if is_release_pr_branch "$branch"; then
    is_release="true"
  fi
  if [[ "$expect_release" == "true" && "$is_release" != "true" ]]; then
    echo "Branch '$branch' does not point to a release PR. Use merge-pr for non-release PRs." >&2
    exit 1
  fi
  if [[ "$expect_release" != "true" && "$is_release" == "true" ]]; then
    echo "Branch '$branch' points to a release PR. Use merge-release-pr instead." >&2
    exit 1
  fi

  assert_pr_ready_for_merge "$branch"
  local args=("$branch" "--merge")
  if prompt_yes_no "Delete branch after merge?" "true"; then
    args+=("--delete-branch")
  fi
  gh pr merge "${args[@]}"
}

release_status_workflow() {
  ensure_command git
  local base_branch="${1:-main}"
  echo "Release workflow status"
  echo "-----------------------"
  echo "Current branch: $(get_current_branch)"
  local latest_tag
  latest_tag="$(git describe --tags --abbrev=0 2>/dev/null || true)"
  [[ -n "$latest_tag" ]] || latest_tag="(no tags yet)"
  echo "Latest tag: $latest_tag"
  echo
  echo "Working tree:"
  git status --short

  if command -v gh >/dev/null 2>&1; then
    local prs
    prs="$(gh pr list --state open --base "$base_branch" --search "chore: release" --limit 20 --json number,title,url --jq '.[] | "#\(.number) \(.title)\n\(.url)"' || true)"
    echo
    if [[ -z "${prs// }" ]]; then
      echo "Open release PRs: none"
    else
      echo "Open release PRs:"
      echo "$prs"
    fi
  fi
}

main() {
  local command="${1:-}"
  [[ -n "$command" ]] || {
    echo "Usage: workspace_release.sh <commit|push|pr|merge-pr|merge-release-pr|status> [args]" >&2
    exit 1
  }
  shift || true

  case "$command" in
    commit) commit_workflow ;;
    push) push_workflow ;;
    pr) pr_workflow "${1:-main}" ;;
    merge-pr) merge_pr_workflow "${1:-}" "false" "${2:-main}" ;;
    merge-release-pr) merge_pr_workflow "${1:-}" "true" "${2:-main}" ;;
    status) release_status_workflow "${1:-main}" ;;
    *)
      echo "Unknown command '$command'. Use: commit, push, pr, merge-pr, merge-release-pr, status." >&2
      exit 1
      ;;
  esac
}

main "$@"
