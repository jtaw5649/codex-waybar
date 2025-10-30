#!/usr/bin/env bash
set -euo pipefail

REPO_URL=${REPO_URL:-"https://github.com/jtaw5649/codex-waybar.git"}
TMP_REPO=""
INSTALL_SYSTEMD=1

RELEASE_OWNER=${CODEX_WAYBAR_RELEASE_OWNER:-jtaw5649}
RELEASE_REPO=${CODEX_WAYBAR_RELEASE_REPO:-codex-waybar}
RELEASE_TAG=${CODEX_WAYBAR_RELEASE_TAG:-latest}
RELEASE_OVERRIDE_FILE=${CODEX_WAYBAR_RELEASE_FILE:-}
USE_RELEASE=${CODEX_WAYBAR_RELEASE_FETCH:-1}

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
SYSTEMD_USER_DIR="${SYSTEMD_USER_DIR:-${HOME}/.config/systemd/user}"
WAYBAR_CONFIG_DIR="${WAYBAR_CONFIG_DIR:-${HOME}/.config/waybar}"
WAYBAR_BACKUP_ROOT="${WAYBAR_BACKUP_ROOT:-}"
SKIP_BUILD="${CODEX_WAYBAR_SKIP_BUILD:-0}"
SKIP_MESON="${CODEX_WAYBAR_SKIP_MESON:-0}"
SKIP_SYSTEMD="${CODEX_WAYBAR_SKIP_SYSTEMD:-0}"
SKIP_WAYBAR_RESTART="${CODEX_WAYBAR_SKIP_WAYBAR_RESTART:-0}"
SKIP_BUILD="${CODEX_WAYBAR_SKIP_BUILD:-0}"
SKIP_MESON="${CODEX_WAYBAR_SKIP_MESON:-0}"
SKIP_SYSTEMD="${CODEX_WAYBAR_SKIP_SYSTEMD:-0}"
SKIP_WAYBAR_RESTART="${CODEX_WAYBAR_SKIP_WAYBAR_RESTART:-0}"

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
WAYBAR_BACKUP_ROOT="${WAYBAR_BACKUP_ROOT:-${SHARE_DIR}/backups}"

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

detect_target() {
  local arch
  arch="$(uname -m)"
  case "${arch}" in
    x86_64)
      echo "x86_64-unknown-linux-gnu"
      ;;
    aarch64|arm64)
      echo "aarch64-unknown-linux-gnu"
      ;;
    *)
      echo "Unsupported architecture: ${arch}" >&2
      return 1
      ;;
  esac
}

install_from_release() {
  local archive="$1"
  local tmp_archive=""

  if [[ -z "${archive}" ]]; then
    local target
    target="$(detect_target)" || return 1
    require curl
    require tar

    local api_url
    if [[ "${RELEASE_TAG}" == "latest" ]]; then
      api_url="https://api.github.com/repos/${RELEASE_OWNER}/${RELEASE_REPO}/releases/latest"
    else
      api_url="https://api.github.com/repos/${RELEASE_OWNER}/${RELEASE_REPO}/releases/tags/${RELEASE_TAG}"
    fi

    echo "==> Searching release assets for ${target}"
    local download_url
    download_url="$(curl -fsSL "${api_url}" | grep -o "\"browser_download_url\": \"[^\"]*codex-waybar-${target}\.tar\.gz\"" | head -n1 | sed 's/.*"browser_download_url": "//;s/"$//')"
    if [[ -z "${download_url}" ]]; then
      echo "No release asset found for target ${target}" >&2
      return 1
    fi

    tmp_archive="$(mktemp)"
    echo "==> Downloading ${download_url}"
    if ! curl -fsSL "${download_url}" -o "${tmp_archive}"; then
      echo "Failed to download release asset" >&2
      rm -f "${tmp_archive}"
      return 1
    fi
    archive="${tmp_archive}"
  else
    if [[ ! -f "${archive}" ]]; then
      echo "Release override file ${archive} not found" >&2
      return 1
    fi
  fi

  require tar
  local extract_dir
  extract_dir="$(mktemp -d)"
  if ! tar -xzf "${archive}" -C "${extract_dir}"; then
    echo "Failed to extract release archive" >&2
    rm -rf "${extract_dir}" "${tmp_archive}"
    return 1
  fi

  local root
  root="$(find "${extract_dir}" -mindepth 1 -maxdepth 1 -type d | head -n1)"
  if [[ -z "${root}" ]]; then
    root="${extract_dir}"
  fi

  local binary_source=""
  if [[ -f "${root}/codex-waybar" ]]; then
    binary_source="${root}/codex-waybar"
  elif [[ -f "${root}/bin/codex-waybar" ]]; then
    binary_source="${root}/bin/codex-waybar"
  fi

  if [[ -z "${binary_source}" ]]; then
    echo "Release archive missing codex-waybar binary" >&2
    rm -rf "${extract_dir}" "${tmp_archive}"
    return 1
  fi

  echo "==> Installing prebuilt codex-waybar"
  mkdir -p "${BIN_DIR}"
  install -m 755 "${binary_source}" "${BIN_DIR}/codex-waybar"

  if [[ -d "${root}/examples" ]]; then
    mkdir -p "${SHARE_DIR}/examples"
    install -m 644 "${root}/examples"/* "${SHARE_DIR}/examples/" 2>/dev/null || true
  fi

  if [[ -f "${root}/README.md" ]]; then
    mkdir -p "${SHARE_DIR}"
    install -m 644 "${root}/README.md" "${SHARE_DIR}/README.md"
  fi

  if [[ -d "${root}/systemd" ]]; then
    mkdir -p "${SHARE_DIR}/systemd"
    cp -f "${root}/systemd/codex-waybar.service" "${SHARE_DIR}/systemd/" 2>/dev/null || true
  fi

  for candidate in "lib/waybar/wb_codex_shimmer.so" "lib64/waybar/wb_codex_shimmer.so"; do
    if [[ -f "${root}/${candidate}" ]]; then
      local target_dir
      target_dir="${PREFIX}/$(dirname "${candidate}")"
      mkdir -p "${target_dir}"
      install -m 755 "${root}/${candidate}" "${target_dir}/$(basename "${candidate}")"
    fi
  done

  rm -rf "${extract_dir}" "${tmp_archive}"
  echo "==> Release installation complete"
  return 0
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

DID_RELEASE=0
if [[ "${USE_RELEASE}" == "1" ]]; then
  if install_from_release "${RELEASE_OVERRIDE_FILE}"; then
    DID_RELEASE=1
    SKIP_BUILD="1"
  else
    echo "==> Release artifacts unavailable; falling back to source build"
  fi
fi

if [[ "${SKIP_BUILD}" != "1" ]]; then
  require cargo
fi

echo "==> Using repository at ${REPO_ROOT}"
pushd "${REPO_ROOT}" >/dev/null

BIN_SOURCE="${REPO_ROOT}/target/release/codex-waybar"

if [[ "${SKIP_BUILD}" != "1" ]]; then
  echo "==> Building codex-waybar (release profile)"
  cargo build --release
else
  echo "==> Skipping cargo build (CODEX_WAYBAR_SKIP_BUILD=1)"
fi

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

if [[ -d "${WAYBAR_CONFIG_DIR}" ]]; then
  timestamp="$(date +%Y%m%d%H%M%S)"
  backup_dir="${WAYBAR_BACKUP_ROOT}/waybar-${timestamp}"
  echo "==> Backing up Waybar configuration to ${backup_dir}"
  mkdir -p "${backup_dir}"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a -- "${WAYBAR_CONFIG_DIR}/" "${backup_dir}/"
  else
    cp -a "${WAYBAR_CONFIG_DIR}/." "${backup_dir}/"
  fi
  echo "Waybar configuration backup stored at ${backup_dir}"
else
  echo "==> Skipping Waybar backup; ${WAYBAR_CONFIG_DIR} not found."
fi

if [[ "${SKIP_MESON}" != "1" ]] && command -v meson >/dev/null 2>&1; then
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
elif [[ "${SKIP_MESON}" == "1" ]]; then
  echo "==> Skipping Meson build (CODEX_WAYBAR_SKIP_MESON=1)"
else
  echo "==> Meson not found; skipping CFFI module build. Install meson to build wb_codex_shimmer." >&2
fi

if [[ ${INSTALL_SYSTEMD} -eq 1 && "${SKIP_SYSTEMD}" != "1" && -f "${REPO_ROOT}/systemd/codex-waybar.service" ]]; then
  echo "==> Installing user systemd unit"
  mkdir -p "${SYSTEMD_USER_DIR}"
  install -m 644 "${REPO_ROOT}/systemd/codex-waybar.service" "${SYSTEMD_USER_DIR}/codex-waybar.service"
  echo "==> Reloading user systemd daemon"
  systemctl --user daemon-reload
  echo "==> Enabling and restarting codex-waybar.service"
  systemctl --user enable --now codex-waybar.service
  echo "==> Current service status"
  systemctl --user status codex-waybar.service --no-pager
elif [[ ${INSTALL_SYSTEMD} -eq 1 && "${SKIP_SYSTEMD}" == "1" ]]; then
  echo "==> Skipping systemd setup (CODEX_WAYBAR_SKIP_SYSTEMD=1)"
else
  echo "==> Skipping systemd setup"
fi

if [[ "${SKIP_WAYBAR_RESTART}" != "1" ]] && command -v waybar >/dev/null 2>&1; then
  echo "==> Restarting Waybar"
  pkill waybar || true
  (waybar >/dev/null 2>&1 & disown) || true
elif [[ "${SKIP_WAYBAR_RESTART}" == "1" ]]; then
  echo "==> Skipping Waybar restart (CODEX_WAYBAR_SKIP_WAYBAR_RESTART=1)"
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
