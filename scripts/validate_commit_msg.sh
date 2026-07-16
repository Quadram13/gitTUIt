#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: validate_commit_msg.sh <commit-message-file>" >&2
  exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./commit_policy.sh
source "${SCRIPT_DIR}/commit_policy.sh"

msg_file="$1"
mapfile -t staged_paths < <(git diff --cached --name-only)

has_src_flag="false"
if has_src_changes "${staged_paths[@]:-}"; then
  has_src_flag="true"
fi

validate_commit_message_file "$msg_file" "$has_src_flag"
