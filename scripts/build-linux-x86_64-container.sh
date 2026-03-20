#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CONTAINER_IMAGE="${CONTAINER_IMAGE:-docker.io/library/rust:1-bookworm}"
MAX_GLIBC_VERSION="${VERTEX_MAX_GLIBC_VERSION:-2.42}"
WORK_ROOT="${REPO_ROOT}/.cache/linux-x86_64-container"
CARGO_REGISTRY_DIR="${WORK_ROOT}/cargo-registry"
CARGO_GIT_DIR="${WORK_ROOT}/cargo-git"

mkdir -p "${WORK_ROOT}" "${CARGO_REGISTRY_DIR}" "${CARGO_GIT_DIR}"

podman run --rm \
  --arch=amd64 \
  -v "${REPO_ROOT}:/workspace" \
  -v "${WORK_ROOT}:/cache" \
  -v "${CARGO_REGISTRY_DIR}:/usr/local/cargo/registry" \
  -v "${CARGO_GIT_DIR}:/usr/local/cargo/git" \
  -w /workspace \
  -e MAX_GLIBC_VERSION="${MAX_GLIBC_VERSION}" \
  "${CONTAINER_IMAGE}" \
  bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    export PATH="/usr/local/cargo/bin:${PATH}"
    export HOME=/cache/home
    export XDG_CACHE_HOME=/cache/xdg-cache
    export XDG_DATA_HOME=/cache/xdg-data
    mkdir -p "${HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}"

    normalize_glibc_version() {
      local value="$1"
      value="${value#GLIBC_}"
      printf "%s\n" "${value}"
    }

    echo "[linux-x86_64] installing native build dependencies..."
    apt-get update >/dev/null
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      build-essential \
      pkg-config \
      libglib2.0-dev \
      libgtk-3-dev \
      libgdk-pixbuf-2.0-dev \
      libpango1.0-dev \
      libatk1.0-dev \
      libcairo2-dev \
      libsysprof-4-dev \
      libsoup-3.0-dev \
      libwebkit2gtk-4.1-dev \
      libjavascriptcoregtk-4.1-dev \
      binutils >/dev/null

    echo "[linux-x86_64] ensuring Rust toolchain..."
    if ! command -v rustup >/dev/null 2>&1; then
      echo "[linux-x86_64] bootstrapping rustup..."
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

    echo "[linux-x86_64] building release artifact..."
    cargo build --release --target x86_64-unknown-linux-gnu -p vertexlauncher

    echo "[linux-x86_64] inspecting glibc symbol floor..."
    glibc_floor="$(bash /workspace/scripts/report-linux-glibc-floor.sh /workspace/target/x86_64-unknown-linux-gnu/release/vertexlauncher)"
    echo "[linux-x86_64] highest required glibc: ${glibc_floor}"

    if [ -n "${MAX_GLIBC_VERSION}" ]; then
      normalized_max_glibc="$(normalize_glibc_version "${MAX_GLIBC_VERSION}")"
      normalized_glibc_floor="$(normalize_glibc_version "${glibc_floor}")"

      if [ "$(printf "%s\n%s\n" "${normalized_max_glibc}" "${normalized_glibc_floor}" | sort -V | tail -n 1)" != "${normalized_max_glibc}" ]; then
        echo "[linux-x86_64] glibc floor ${glibc_floor} exceeds allowed maximum ${MAX_GLIBC_VERSION}" >&2
        exit 1
      fi
    fi
  '
