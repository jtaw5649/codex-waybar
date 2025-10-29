#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

: "${PREFIX:=${HOME}/.local}"
: "${BIN_DIR:=${PREFIX}/bin}"
: "${SHARE_DIR:=${PREFIX}/share/codex-waybar}"

echo "==> Building codex-waybar (release profile)"
cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml"

BIN_SOURCE="${REPO_ROOT}/target/release/codex-waybar"
if [[ ! -x "${BIN_SOURCE}" ]]; then
  echo "Error: expected binary at ${BIN_SOURCE} but none was found." >&2
  exit 1
fi

echo "==> Installing binary to ${BIN_DIR}"
mkdir -p "${BIN_DIR}"
install -m 755 "${BIN_SOURCE}" "${BIN_DIR}/codex-waybar"

echo "==> Installing documentation to ${SHARE_DIR}"
mkdir -p "${SHARE_DIR}"
install -m 644 "${REPO_ROOT}/README.md" "${SHARE_DIR}/README.md"

if [[ -d "${REPO_ROOT}/examples" ]]; then
  mkdir -p "${SHARE_DIR}/examples"
  install -m 644 "${REPO_ROOT}"/examples/* "${SHARE_DIR}/examples/"
fi

SYSTEMD_USER_DIR="${SYSTEMD_USER_DIR:-${HOME}/.config/systemd/user}"
if [[ -f "${REPO_ROOT}/systemd/codex-waybar.service" ]]; then
  mkdir -p "${SYSTEMD_USER_DIR}"
  install -m 644 "${REPO_ROOT}/systemd/codex-waybar.service" "${SYSTEMD_USER_DIR}/codex-waybar.service"
fi

echo "codex-waybar installed successfully."
echo "Binary location : ${BIN_DIR}/codex-waybar"
echo "Docs/examples   : ${SHARE_DIR}"
if [[ -f "${SYSTEMD_USER_DIR}/codex-waybar.service" ]]; then
  echo "Systemd unit    : ${SYSTEMD_USER_DIR}/codex-waybar.service"
fi
