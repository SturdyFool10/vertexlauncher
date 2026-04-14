#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
WORK_ROOT="${REPO_ROOT}/.cache/appimage-x86_64-container"
TOOLCHAIN_CACHE_ROOT="${REPO_ROOT}/.cache/linux-x86_64-toolchain"
CARGO_HOME_DIR="${TOOLCHAIN_CACHE_ROOT}/cargo-home"
RUSTUP_HOME_DIR="${TOOLCHAIN_CACHE_ROOT}/rustup"
CONTAINER_DIR="${REPO_ROOT}/containers"

source "${REPO_ROOT}/scripts/lib/portable-linux-common.sh"

CONTAINER_IMAGE="${CONTAINER_IMAGE:-$(ensure_podman_image \
  centos7-webkit \
  x86_64 \
  "${CONTAINER_DIR}/vertexlauncher-centos7-webkit.Dockerfile" \
  "${CONTAINER_DIR}")}"

bash "${REPO_ROOT}/scripts/compile-slang-shaders.sh"

# Remove host-compiled build-script executables and the final binary before
# entering the container so that the container recompiles them against its glibc.
find "${REPO_ROOT}/target" -name "build-script-build" -delete 2>/dev/null || true
rm -f "${REPO_ROOT}/target/x86_64-unknown-linux-gnu/release/vertexlauncher" 2>/dev/null || true
rm -f "${REPO_ROOT}/target/release/vertexlauncher" 2>/dev/null || true

mkdir -p "${WORK_ROOT}" "${CARGO_HOME_DIR}" "${RUSTUP_HOME_DIR}"

declare -A MOUNTED_DIRS=()
echo "[appimage] container dependency preflight..."
podman run --rm --arch=amd64 "${CONTAINER_IMAGE}" bash -lc '
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
  -v "${CARGO_HOME_DIR}:/usr/local/cargo"
  -v "${RUSTUP_HOME_DIR}:/usr/local/rustup"
  -w /workspace
  -e VERTEX_APPIMAGE_ARCH=x86_64
  -e VERTEX_APPIMAGE_TARGET=x86_64-unknown-linux-gnu
  -e VERTEX_IN_APPIMAGE_CONTAINER=1
  # Forward pkg‑config environment so that the build script can find
  # libsoup and other development packages in the container.  Without
  # PKG_CONFIG_PATH and the *_ALLOW_SYSTEM_* flags `soup2‑sys` fails to
  # locate libsoup even though it is installed.
  -e PKG_CONFIG_PATH="/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig" \
  -e PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
  -e PKG_CONFIG_ALLOW_SYSTEM_LIBS=1
  -e PKG_CONFIG_LIBDIR=/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig \
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
    export PATH="/usr/local/cargo/bin:${PATH}"
    export CARGO_HOME=/usr/local/cargo
    export RUSTUP_HOME=/usr/local/rustup
    export HOME=/root
    export XDG_CACHE_HOME=/cache/xdg-cache
    export XDG_DATA_HOME=/cache/xdg-data
    mkdir -p "${CARGO_HOME}" "${RUSTUP_HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}"

    if [[ ! -f /workspace/target/x86_64-unknown-linux-gnu/release/vertexlauncher ]]; then
      echo "[appimage-x86_64] ensuring Rust toolchain..."
      if ! command -v rustup >/dev/null 2>&1; then
        echo "[appimage-x86_64] bootstrapping rustup..."
        curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
        if [ -f "${HOME}/.cargo/env" ]; then
          # shellcheck disable=SC1091
          . "${HOME}/.cargo/env"
        elif [ -f /usr/local/cargo/env ]; then
          # shellcheck disable=SC1091
          . /usr/local/cargo/env
        fi
      fi
      if ! rustup toolchain list | grep -Eq "^stable($|-)"; then
        rustup toolchain install stable --profile minimal >/dev/null
      fi
      rustup default stable >/dev/null
      rustup target add x86_64-unknown-linux-gnu >/dev/null

      bash /workspace/scripts/patch-wry-source.sh

      echo "[appimage-x86_64] building native x86_64 binary..."

      # Clean up any previously compiled artifacts to avoid glibc
      # mismatches.  Stale build script binaries linked against a newer
      # glibc on the host can end up in the shared `target` directory and
      # will fail when executed in this CentOS 7 container (which only
      # provides glibc 2.17).  Running `cargo clean` forces a fresh build
      # of all build scripts and dependencies against the container
      # (which provides glibc 2.17), preserving the portability of the resulting binary.
      echo "[appimage-x86_64] cleaning stale workspace build-script artifacts..."
      cargo clean --package vertexlauncher --package launcher_ui || true
      # Export pkg‑config hints within the container as well.  See
      # comment in the Podman arguments above.  Explicitly setting
      # PKG_CONFIG_PATH and allowing system CFLAGS/LIBS ensures that
      # pkg‑config can locate libsoup and other development files during
      # the build of `soup2‑sys` and related crates.
      export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig}"
      export PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
      export PKG_CONFIG_ALLOW_SYSTEM_LIBS=1

      cargo build --release --target x86_64-unknown-linux-gnu -p vertexlauncher
    fi

    echo "[appimage-x86_64] packaging AppImage inside CentOS 7 container..."
    bash /workspace/scripts/build-appimage.sh
  '
