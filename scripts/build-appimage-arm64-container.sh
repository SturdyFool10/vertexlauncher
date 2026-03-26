#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CONTAINER_IMAGE="${CONTAINER_IMAGE:-docker.io/library/rust:1-bookworm}"
WORK_ROOT="${REPO_ROOT}/.cache/appimage-arm64-container"
SOURCE_BINARY="${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/vertexlauncher"

mkdir -p "${WORK_ROOT}"

if [[ ! -f "${SOURCE_BINARY}" ]]; then
  if [[ -x "${REPO_ROOT}/scripts/build-linux-arm64-container.sh" ]]; then
    echo "[appimage-arm64] building missing aarch64 Linux binary first..."
    bash "${REPO_ROOT}/scripts/build-linux-arm64-container.sh"
  else
    echo "Missing built Linux binary: ${SOURCE_BINARY}" >&2
    exit 1
  fi
fi

declare -A MOUNTED_DIRS=()
echo "[appimage] arm64 container dependency preflight..."
podman run --rm --arch=arm64 "${CONTAINER_IMAGE}" bash -lc '
set -e
echo "glib-2.0: $(pkg-config --modversion glib-2.0)"
echo "webkit2gtk-4.1: $(pkg-config --modversion webkit2gtk-4.1)"
echo "javascriptcoregtk-4.1: $(pkg-config --modversion javascriptcoregtk-4.1)"
echo "libsoup-2.4: $(pkg-config --modversion libsoup-2.4)"
pkg-config --exists "glib-2.0 >= 2.70"
'

PODMAN_ARGS=(
  run
  --rm
  --arch=amd64
  -v "${REPO_ROOT}:/workspace"
  -v "${WORK_ROOT}:/cache"
  -w /workspace
  -e VERTEX_APPIMAGE_ARCH=aarch64
  -e VERTEX_APPIMAGE_TARGET=aarch64-unknown-linux-gnu
  -e VERTEX_IN_APPIMAGE_CONTAINER=1
  -e VERTEX_APPIMAGE_TOOL_RUNNER=qemu-aarch64-static
)

mount_external_tool() {
  local env_name="$1"
  local tool_path="${!env_name:-}"
  local tool_dir

  if [[ -z "${tool_path}" ]]; then
    return 0
  fi
  if [[ ! -f "${tool_path}" ]]; then
    echo "Configured ${env_name} path does not exist: ${tool_path}" >&2
    exit 1
  fi

  tool_dir="$(dirname -- "${tool_path}")"
  if [[ -z "${MOUNTED_DIRS[${tool_dir}]:-}" ]]; then
    PODMAN_ARGS+=(-v "${tool_dir}:${tool_dir}:ro")
    MOUNTED_DIRS["${tool_dir}"]=1
  fi
  PODMAN_ARGS+=(-e "${env_name}=${tool_path}")
}

mount_external_tool VERTEX_LINUXDEPLOY
mount_external_tool VERTEX_APPIMAGETOOL
mount_external_tool VERTEX_LINUXDEPLOY_GTK_PLUGIN

podman "${PODMAN_ARGS[@]}" \
  "${CONTAINER_IMAGE}" \
  bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    export HOME=/cache/home
    export XDG_CACHE_HOME=/cache/xdg-cache
    export XDG_DATA_HOME=/cache/xdg-data
    mkdir -p "${HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}"

    echo "[appimage-arm64] installing packaging dependencies..."
    apt-get update >/dev/null
    dpkg --add-architecture arm64
    apt-get update >/dev/null
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      qemu-user-static \
      patchelf \
      file \
      desktop-file-utils \
      libglib2.0-0:arm64 \
      libgtk-3-0:arm64 \
      libgdk-pixbuf-2.0-0:arm64 \
      libpango-1.0-0:arm64 \
      libatk1.0-0:arm64 \
      libcairo2:arm64 \
      libsysprof-4:arm64 \
      libsoup-3.0-0:arm64 \
      libwebkit2gtk-4.1-0:arm64 \
      libjavascriptcoregtk-4.1-0:arm64 >/dev/null

    echo "[appimage-arm64] packaging AppImage via qemu-aarch64-static..."
    bash /workspace/scripts/build-appimage.sh
  '
