#!/usr/bin/env bash
set -euo pipefail

REPO_URL=${REPO_URL:-"https://github.com/jtaw5649/codex-waybar.git"}
TMP_REPO=""
INSTALL_SYSTEMD=1

usage() {
  cat <<'EOF'
Usage: install.sh [options]

Options:
  --prefix <path>        Installation prefix (default: ~/.local or PREFIX env)
  --bin-dir <path>       Override binary install directory
  --share-dir <path>     Override shared data directory
  --no-systemd           Skip installing the user systemd unit
  --help                 Display this help and exit

Environment variables PREFIX, BIN_DIR, SHARE_DIR, SYSTEMD_USER_DIR are honoured
and override the defaults.
EOF
}

PREFIX_DEFAULT="${HOME}/.local"
PREFIX="${PREFIX:-$PREFIX_DEFAULT}"
BIN_DIR="${BIN_DIR:-}"
SHARE_DIR="${SHARE_DIR:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      [[ $# -lt 2 ]] && { echo "Missing value for --prefix" >&2; exit 1; }
      PREFIX="$2"
      shift
      ;;
    --prefix=*)
      PREFIX="${1#*=}"
      ;;
    --bin-dir=*)
      BIN_DIR="${1#*=}"
      ;;
    --share-dir=*)
      SHARE_DIR="${1#*=}"
      ;;
    --no-systemd)
      INSTALL_SYSTEMD=0
      ;;
    --help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

BIN_DIR="${BIN_DIR:-${PREFIX}/bin}"
SHARE_DIR="${SHARE_DIR:-${PREFIX}/share/codex-waybar}"
SYSTEMD_USER_DIR="${SYSTEMD_USER_DIR:-${HOME}/.config/systemd/user}"

cleanup() {
  if [[ -n "${TMP_REPO}" && -d "${TMP_REPO}" ]]; then
    rm -rf "${TMP_REPO}"
  fi
}
trap cleanup EXIT

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Error: '$1' is required but not found in PATH" >&2
    exit 1
  fi
}

resolve_repo_root() {
  # 1. Script executed from file path
  if [[ -n "${BASH_SOURCE[0]:-}" && "${BASH_SOURCE[0]}" != "stdin" ]]; then
    local script_path
    script_path="$(readlink -f "${BASH_SOURCE[0]}")"
    local script_dir
    script_dir="$(cd "$(dirname "${script_path}")" && pwd)"
    if [[ -f "${script_dir}/Cargo.toml" ]]; then
      echo "${script_dir}"
      return
    fi
    if [[ -f "${script_dir}/../Cargo.toml" ]]; then
      echo "$(cd "${script_dir}/.." && pwd)"
      return
    fi
  fi

  # 2. Running from within a checkout (current directory contains sources)
  if [[ -f "Cargo.toml" && -f "install.sh" ]]; then
    echo "$(pwd)"
    return
  fi

  # 3. Remote installer path: clone into a temporary directory
  require git
  TMP_REPO="$(mktemp -d)"
  echo "==> Cloning ${REPO_URL}"
  git clone --depth 1 "${REPO_URL}" "${TMP_REPO}"
  echo "${TMP_REPO}"
}

REPO_ROOT="$(resolve_repo_root)"

require cargo

echo "==> Using repository at ${REPO_ROOT}"
pushd "${REPO_ROOT}" >/dev/null

echo "==> Building codex-waybar (release profile)"
cargo build --release

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

if command -v meson >/dev/null 2>&1; then
  echo "==> Building codex shimmer Waybar module"
  pushd "${REPO_ROOT}/cffi/codex_shimmer" >/dev/null
  BUILD_DIR="${BUILD_DIR:-build}"
  if [[ -d "${BUILD_DIR}" ]]; then
    meson setup "${BUILD_DIR}" --prefix="${PREFIX}" --reconfigure
  else
    meson setup "${BUILD_DIR}" --prefix="${PREFIX}"
  fi
  meson compile -C "${BUILD_DIR}"
  meson install -C "${BUILD_DIR}"
  popd >/dev/null
else
  echo "==> Meson not found; skipping CFFI module build. Install meson to build wb_codex_shimmer." >&2
fi

if [[ ${INSTALL_SYSTEMD} -eq 1 && -f "${REPO_ROOT}/systemd/codex-waybar.service" ]]; then
  echo "==> Installing user systemd unit"
  mkdir -p "${SYSTEMD_USER_DIR}"
  install -m 644 "${REPO_ROOT}/systemd/codex-waybar.service" "${SYSTEMD_USER_DIR}/codex-waybar.service"
  echo "==> Reloading user systemd daemon"
  systemctl --user daemon-reload
  echo "==> Enabling and restarting codex-waybar.service"
  systemctl --user enable --now codex-waybar.service
  echo "==> Current service status"
  systemctl --user status codex-waybar.service --no-pager
else
  echo "==> Skipping systemd setup"
fi

if command -v waybar >/dev/null 2>&1; then
  echo "==> Restarting Waybar"
  pkill waybar || true
  (waybar >/dev/null 2>&1 & disown) || true
else
  echo "==> Waybar executable not found on PATH; skipping Waybar restart"
fi

popd >/dev/null

echo "codex-waybar installed successfully."
echo "Binary location : ${BIN_DIR}/codex-waybar"
echo "Docs/examples   : ${SHARE_DIR}"
if [[ ${INSTALL_SYSTEMD} -eq 1 && -f "${SYSTEMD_USER_DIR}/codex-waybar.service" ]]; then
  echo "Systemd unit    : ${SYSTEMD_USER_DIR}/codex-waybar.service"
fi
