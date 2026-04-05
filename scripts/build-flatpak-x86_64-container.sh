#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CONTAINER_IMAGE="${CONTAINER_IMAGE:-docker.io/library/rust:1-bookworm}"
WORK_ROOT="${REPO_ROOT}/.cache/flatpak-x86_64-container"

mkdir -p "${WORK_ROOT}"

podman run --rm \
  --privileged \
  --arch=amd64 \
  -v "${REPO_ROOT}:/workspace" \
  -v "${WORK_ROOT}:/cache" \
  -w /workspace \
  -e VERTEX_FLATPAK_BRANCH="${VERTEX_FLATPAK_BRANCH:-stable}" \
  "${CONTAINER_IMAGE}" \
  bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    export PATH="/usr/local/cargo/bin:${PATH}"
    export HOME=/cache/home
    export XDG_CACHE_HOME=/cache/xdg-cache
    export XDG_DATA_HOME=/cache/xdg-data

    mkdir -p "${HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}"

    echo "[flatpak-x86_64] installing build dependencies..."
    apt-get update >/dev/null
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      flatpak \
      flatpak-builder \
      ostree \
      python3 \
      python3-aiohttp \
      rsync \
      xz-utils \
      zstd >/dev/null

    echo "[flatpak-x86_64] building x86_64 flatpak..."
    export VERTEX_IN_X86_64_CONTAINER=1
    export VERTEX_FLATPAK_ARCHES=x86_64
    bash /workspace/scripts/build-flatpak.sh
  '
