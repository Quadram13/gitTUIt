#!/usr/bin/env bash
set -euo pipefail

ALL_TYPES=(feat fix docs chore refactor test build ci perf revert)
RELEASABLE_TYPES=(feat fix)
SRC_PREFIXES=("src/")

entry_raw=""
entry_type=""
entry_breaking="false"
entry_description=""

trim_cr() {
  local value="$1"
  value="${value%$'\r'}"
  printf '%s' "$value"
}

contains_item() {
  local needle="$1"
  shift
  local item
  for item in "$@"; do
    [[ "$item" == "$needle" ]] && return 0
  done
  return 1
}

is_releasable_type() {
  local t="$1"
  contains_item "$t" "${RELEASABLE_TYPES[@]}"
}

is_allowed_type() {
  local t="$1"
  contains_item "$t" "${ALL_TYPES[@]}"
}

has_src_changes() {
  local path
  for path in "$@"; do
    local prefix
    for prefix in "${SRC_PREFIXES[@]}"; do
      [[ "$path" == "$prefix"* ]] && return 0
    done
  done
  return 1
}

parse_change_entry_line() {
  local line
  line="$(trim_cr "$1")"
  [[ -z "${line// }" ]] && return 1
  if [[ "$line" =~ ^([a-z]+)(!)?:[[:space:]]+(.+)$ ]]; then
    local t="${BASH_REMATCH[1]}"
    is_allowed_type "$t" || return 1
    entry_raw="$line"
    entry_type="$t"
    entry_breaking="false"
    [[ -n "${BASH_REMATCH[2]:-}" ]] && entry_breaking="true"
    entry_description="${BASH_REMATCH[3]}"
    return 0
  fi
  return 1
}

parse_message_entries_and_freeform() {
  local message_text="$1"
  local entries_file="$2"
  local freeform_file="$3"

  : >"$entries_file"
  : >"$freeform_file"

  mapfile -t lines <<<"${message_text//$'\r'/}"
  if [[ "${#lines[@]}" -eq 0 || -z "${lines[0]// }" ]]; then
    echo "Commit message subject line is empty." >&2
    return 1
  fi

  parse_change_entry_line "${lines[0]}" || {
    echo "Commit subject must follow '<type>!: <description>' (or '<type>: <description>') and use an allowed type." >&2
    return 1
  }
  printf '%s\n' "$entry_raw" >>"$entries_file"

  local body=()
  if [[ "${#lines[@]}" -gt 1 ]]; then
    body=("${lines[@]:1}")
  fi

  local end=$(( ${#body[@]} - 1 ))
  while (( end >= 0 )); do
    [[ -n "${body[$end]// }" ]] && break
    ((end--))
  done

  if (( end < 0 )); then
    return 0
  fi

  local footer_rev=()
  local idx=$end
  while (( idx >= 0 )); do
    if parse_change_entry_line "${body[$idx]}"; then
      footer_rev+=("$entry_raw")
      ((idx--))
    else
      break
    fi
  done

  if (( idx >= 0 )); then
    local i
    for ((i=0; i<=idx; i++)); do
      printf '%s\n' "${body[$i]}" >>"$freeform_file"
    done
  fi

  local i
  for ((i=${#footer_rev[@]}-1; i>=0; i--)); do
    printf '%s\n' "${footer_rev[$i]}" >>"$entries_file"
  done
}

validate_entries_file_against_policy() {
  local entries_file="$1"
  local has_src_flag="$2"
  local line
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "${line// }" ]] && continue
    parse_change_entry_line "$line" || {
      echo "Invalid change entry: $line" >&2
      return 1
    }
    local releasable="false"
    is_releasable_type "$entry_type" && releasable="true"
    if [[ "$entry_breaking" == "true" && "$releasable" != "true" ]]; then
      echo "Only releasable types (${RELEASABLE_TYPES[*]}) may use the breaking marker (!): $entry_raw" >&2
      return 1
    fi
    if [[ "$has_src_flag" != "true" && "$releasable" == "true" ]]; then
      echo "Type '$entry_type' is reserved for commits that include src changes." >&2
      return 1
    fi
  done <"$entries_file"
}

validate_commit_message_text() {
  local message_text="$1"
  local has_src_flag="$2"
  local entries_file freeform_file
  entries_file="$(mktemp)"
  freeform_file="$(mktemp)"
  trap 'rm -f "$entries_file" "$freeform_file"' RETURN
  parse_message_entries_and_freeform "$message_text" "$entries_file" "$freeform_file"
  validate_entries_file_against_policy "$entries_file" "$has_src_flag"
}

validate_commit_message_file() {
  local msg_file="$1"
  local has_src_flag="$2"
  [[ -f "$msg_file" ]] || {
    echo "Commit message file not found: $msg_file" >&2
    return 1
  }
  local content
  content="$(<"$msg_file")"
  validate_commit_message_text "$content" "$has_src_flag"
}
