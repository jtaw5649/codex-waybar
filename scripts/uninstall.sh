#!/usr/bin/env bash
set -euo pipefail

: "${PREFIX:=${HOME}/.local}"
: "${BIN_DIR:=${PREFIX}/bin}"
: "${SHARE_DIR:=${PREFIX}/share/codex-waybar}"

BIN_PATH="${BIN_DIR}/codex-waybar"

if [[ -f "${BIN_PATH}" ]]; then
  echo "==> Removing binary ${BIN_PATH}"
  rm -f "${BIN_PATH}"
else
  echo "Binary not found at ${BIN_PATH}; skipping."
fi

if [[ -d "${SHARE_DIR}" ]]; then
  echo "==> Removing shared resources ${SHARE_DIR}"
  rm -rf "${SHARE_DIR}"
else
  echo "Share directory ${SHARE_DIR} not found; skipping."
fi

echo "codex-waybar has been uninstalled."
