#!/usr/bin/env bash
set -euo pipefail

: "${PREFIX:=${HOME}/.local}"
: "${BIN_DIR:=${PREFIX}/bin}"
: "${SHARE_DIR:=${PREFIX}/share/codex-waybar}"
: "${SYSTEMD_USER_DIR:=${HOME}/.config/systemd/user}"

BIN_PATH="${BIN_DIR}/codex-waybar"
LIB_PATHS=(
  "${LIB_WAYBAR_DIR:-${PREFIX}/lib/waybar/wb_codex_shimmer.so}"
  "${LIB64_WAYBAR_DIR:-${PREFIX}/lib64/waybar/wb_codex_shimmer.so}"
)
SERVICE_PATH="${SYSTEMD_USER_DIR}/codex-waybar.service"
README_PATH="${SHARE_DIR}/README.md"
EXAMPLES_DIR="${SHARE_DIR}/examples"
EXAMPLE_FILES=(
  "codex-waybar.service"
  "waybar-config-snippet.jsonc"
  "waybar-style.css"
)

remove_file() {
  local path="$1"
  if [[ -f "${path}" ]]; then
    echo "==> Removing ${path}"
    rm -f "${path}"
  else
    echo "Skipping ${path}; not found."
  fi
}

remove_directory_if_empty() {
  local dir="$1"
  if [[ -d "${dir}" ]]; then
    if rmdir "${dir}" 2>/dev/null; then
      echo "==> Removed empty directory ${dir}"
    else
      echo "Directory ${dir} not empty; leaving in place."
    fi
  fi
}

remove_file "${BIN_PATH}"

if [[ -d "${SHARE_DIR}" ]]; then
  remove_file "${README_PATH}"
  if [[ -d "${EXAMPLES_DIR}" ]]; then
    for example in "${EXAMPLE_FILES[@]}"; do
      remove_file "${EXAMPLES_DIR}/${example}"
    done
    remove_directory_if_empty "${EXAMPLES_DIR}"
  fi
  remove_directory_if_empty "${SHARE_DIR}"
else
  echo "Skipping shared assets; ${SHARE_DIR} not found."
fi

for lib_path in "${LIB_PATHS[@]}"; do
  remove_file "${lib_path}"
  parent_dir="$(dirname "${lib_path}")"
  remove_directory_if_empty "${parent_dir}"
done

systemctl_available() {
  command -v systemctl >/dev/null 2>&1
}

if [[ -f "${SERVICE_PATH}" ]]; then
  echo "==> Removing user systemd unit ${SERVICE_PATH}"
  if systemctl_available; then
    systemctl --user stop codex-waybar.service || true
    systemctl --user disable codex-waybar.service || true
  fi
  rm -f "${SERVICE_PATH}"
  if systemctl_available; then
    systemctl --user daemon-reload || true
  fi
else
  echo "Skipping systemd unit; ${SERVICE_PATH} not found."
  if systemctl_available; then
    systemctl --user disable codex-waybar.service 2>/dev/null || true
    systemctl --user daemon-reload || true
  fi
fi

restart_waybar() {
  if command -v pkill >/dev/null 2>&1; then
    pkill waybar >/dev/null 2>&1 || true
  else
    echo "pkill not available; skipping Waybar stop."
  fi

  if command -v waybar >/dev/null 2>&1; then
    (waybar >/dev/null 2>&1 & disown) || true
    echo "Waybar restarted."
  else
    echo "Waybar executable not found on PATH; skipping restart."
  fi
}

restart_waybar

echo "codex-waybar has been uninstalled."
