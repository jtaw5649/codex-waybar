#!/usr/bin/env bash
set -euo pipefail

CACHE_FILE="${CACHE_FILE:-$HOME/.cache/codex-waybar/latest.json}"
CACHE_DIR="$(dirname "${CACHE_FILE}")"
HELPER_ARGS=(
  "--cache-file" "${CACHE_FILE}"
  "--max-chars" "${MAX_CHARS:-110}"
  "--session-window" "${SESSION_WINDOW:-6}"
  "--poll-ms" "${POLL_MS:-100}"
)

ensure_helper_running() {
  local helper_pid
  helper_pid="$(pgrep -f "codex-waybar --cache-file ${CACHE_FILE}" || true)"
  local cache_mtime
  local now
  now=$(date +%s)
  if [[ -n "${helper_pid}" ]]; then
    if [[ -f "${CACHE_FILE}" ]]; then
      cache_mtime=$(stat -c '%Y' "${CACHE_FILE}")
      if (( now - cache_mtime > ${CACHE_STALE_SECS:-5} )); then
        kill "${helper_pid}" || true
        helper_pid=""
      fi
    fi
  fi

  if [[ -z "${helper_pid}" ]]; then
    mkdir -p "${CACHE_DIR}"
    setsid codex-waybar "${HELPER_ARGS[@]}" >/tmp/codex-waybar.log 2>&1 < /dev/null &
  fi
}

ensure_helper_running

exec codex-waybar --print-cache "${CACHE_FILE}"
