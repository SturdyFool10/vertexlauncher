#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

APP_ID="io.github.SturdyFool10.VertexLauncher"
BRANCH="${VERTEX_FLATPAK_BRANCH:-stable}"
ARCHES="${VERTEX_FLATPAK_ARCHES:-${VERTEX_FLATPAK_ARCH:-$(flatpak --default-arch 2>/dev/null || uname -m)}}"
INCREMENTAL="${VERTEX_FLATPAK_INCREMENTAL:-0}"

RUNTIME="org.gnome.Platform"
RUNTIME_VERSION="49"
SDK="org.gnome.Sdk"
RUST_EXT="org.freedesktop.Sdk.Extension.rust-stable"
RUST_EXT_TAG="25.08"

GEN_DIR="${REPO_ROOT}/flatpak/generated"
SOURCE_TREE="${GEN_DIR}/source-tree"
DIST_DIR="${REPO_ROOT}/target/release"
CARGO_SOURCES_PATH="${GEN_DIR}/cargo-sources.json"
CARGO_SOURCES_LOCK_SNAPSHOT="${GEN_DIR}/cargo-sources.lock"
GENERATOR_PATH="${GEN_DIR}/flatpak-cargo-generator.py"

log() {
    printf '[build-flatpak] %s\n' "$*"
}

die() {
    printf '[build-flatpak] ERROR: %s\n' "$*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"
}

normalize_arch() {
    case "${1:-}" in
        x86_64|amd64|x86-64) printf 'x86_64\n' ;;
        aarch64|arm64) printf 'aarch64\n' ;;
        *) die "Unsupported arch: $1" ;;
    esac
}

ensure_flathub_remote() {
    if flatpak remotes --columns=name | grep -qx flathub; then
        return 0
    fi

    log "Adding Flathub remote"
    flatpak remote-add --if-not-exists --user flathub \
        https://dl.flathub.org/repo/flathub.flatpakrepo
}

ensure_runtime_bits() {
    log "Ensuring runtime, SDK, and Rust extension are installed"
    flatpak install --user -y flathub \
        "${RUNTIME}//${RUNTIME_VERSION}" \
        "${SDK}//${RUNTIME_VERSION}" \
        "${RUST_EXT}//${RUST_EXT_TAG}"
}

prepare_clean_source_tree() {
    if [[ "${INCREMENTAL}" == "1" ]]; then
        log "Refreshing incremental Flatpak source tree"
    else
        log "Preparing clean Flatpak source tree"
        rm -rf "${SOURCE_TREE}"
        mkdir -p "${SOURCE_TREE}"
    fi

    mkdir -p "${SOURCE_TREE}"

    rsync -a --delete \
        --exclude '.git' \
        --exclude '.github' \
        --exclude '.zed' \
        --exclude '.idea' \
        --exclude '.vscode' \
        --exclude '.venv' \
        --exclude '.direnv' \
        --exclude '.cache' \
        --exclude '.flatpak-builder' \
        --exclude 'flatpak/build' \
        --exclude 'flatpak/repo' \
        --exclude 'flatpak/generated' \
        --exclude 'target' \
        --exclude 'dist' \
        --exclude 'node_modules' \
        --exclude '__pycache__' \
        --exclude '*.pyc' \
        --exclude '*.pyo' \
        "${REPO_ROOT}/" "${SOURCE_TREE}/"

    [[ -f "${SOURCE_TREE}/Cargo.toml" ]] || die "Clean source tree missing Cargo.toml"
    [[ -f "${SOURCE_TREE}/Cargo.lock" ]] || die "Clean source tree missing Cargo.lock"
}

generate_cargo_sources() {
    mkdir -p "${GEN_DIR}"

    [[ -f "${SOURCE_TREE}/Cargo.lock" ]] || die "Cargo.lock not found in clean source tree"

    if [[ ! -f "${GENERATOR_PATH}" ]]; then
        log "Downloading flatpak-cargo-generator.py"
        curl -L --fail --retry 3 \
            -o "${GENERATOR_PATH}" \
            https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
    fi

    if [[ "${INCREMENTAL}" == "1" ]] \
        && [[ -f "${CARGO_SOURCES_PATH}" ]] \
        && [[ -f "${CARGO_SOURCES_LOCK_SNAPSHOT}" ]] \
        && cmp -s "${SOURCE_TREE}/Cargo.lock" "${CARGO_SOURCES_LOCK_SNAPSHOT}"; then
        log "Reusing existing cargo-sources.json because Cargo.lock is unchanged"
        return
    fi

    if ! python3 -c "import aiohttp" 2>/dev/null; then
        log "Installing aiohttp for flatpak-cargo-generator..."
        python3 -m pip install aiohttp --break-system-packages --quiet \
            || python3 -m pip install aiohttp --quiet \
            || die "Failed to install aiohttp — run: pip3 install aiohttp"
    fi

    log "Generating cargo-sources.json"
    python3 "${GENERATOR_PATH}" "${SOURCE_TREE}/Cargo.lock" -o "${CARGO_SOURCES_PATH}"
    [[ -f "${CARGO_SOURCES_PATH}" ]] || die "Failed to generate cargo-sources.json"
    cp "${SOURCE_TREE}/Cargo.lock" "${CARGO_SOURCES_LOCK_SNAPSHOT}"
}

write_manifest() {
    local arch="$1"
    local manifest_path="${GEN_DIR}/${APP_ID}-${arch}.yaml"
    mkdir -p "${GEN_DIR}"

    cat > "${manifest_path}" <<MANIFEST
app-id: ${APP_ID}
runtime: ${RUNTIME}
runtime-version: "${RUNTIME_VERSION}"
sdk: ${SDK}
sdk-extensions:
  - ${RUST_EXT}
command: vertexlauncher
branch: ${BRANCH}
separate-locales: false

finish-args:
  - --share=network

  - --share=ipc

  - --socket=wayland

  - --socket=x11

  - --socket=pulseaudio

  - --device=all

  - --filesystem=home

  - --filesystem=/run/media

  - --filesystem=xdg-config/vertexlauncher

  - --filesystem=xdg-data/vertexlauncher

  - --filesystem=xdg-cache/vertexlauncher

  - --filesystem=xdg-config/vertex-launcher

  - --filesystem=xdg-run/gvfs

  - --filesystem=xdg-run/discord-ipc-0

  - --filesystem=xdg-run/discord-ipc-1

  - --filesystem=xdg-run/discord-ipc-2

  - --filesystem=xdg-run/discord-ipc-3

  - --filesystem=xdg-run/discord-ipc-4

  - --filesystem=xdg-run/discord-ipc-5

  - --filesystem=xdg-run/discord-ipc-6

  - --filesystem=xdg-run/discord-ipc-7

  - --filesystem=xdg-run/discord-ipc-8

  - --filesystem=xdg-run/discord-ipc-9

  - --filesystem=xdg-run/app/com.discordapp.Discord

  - --filesystem=xdg-run/app/com.discordapp.DiscordCanary

  - --filesystem=xdg-run/app/com.discordapp.DiscordPTB

  - --filesystem=xdg-run/app/dev.vencord.Vesktop

  - --talk-name=org.freedesktop.secrets

  - --talk-name=org.freedesktop.Flatpak

  - --env=GDK_BACKEND=wayland,x11
build-options:
  append-path: /usr/lib/sdk/rust-stable/bin
  extension-tag: "${RUST_EXT_TAG}"

modules:
  - name: vertexlauncher
    buildsystem: simple
    build-options:
      env:
        CARGO_HOME: /run/build/vertexlauncher/cargo
        CARGO_TARGET_DIR: /run/build/vertexlauncher/target
        CARGO_NET_OFFLINE: 'true'
        PKG_CONFIG_ALLOW_SYSTEM_CFLAGS: '1'
        PKG_CONFIG_PATH: /app/lib/pkgconfig:/app/share/pkgconfig:/usr/lib/pkgconfig
        RUST_BACKTRACE: '1'
    build-commands:
      - cargo --version
      - rustc --version
      - cargo build --offline --release --locked
      - install -Dm755 target/release/vertexlauncher /app/bin/vertexlauncher
      - install -Dm644 flatpak/${APP_ID}.desktop /app/share/applications/${APP_ID}.desktop
      - install -Dm644 flatpak/${APP_ID}.metainfo.xml /app/share/metainfo/${APP_ID}.metainfo.xml
      - install -Dm644 Vertex.svg /app/share/icons/hicolor/scalable/apps/${APP_ID}.svg
    sources:
      - type: dir
        path: source-tree
      - cargo-sources.json
MANIFEST
}

build_flatpak() {
    local arch="$1"
    local build_dir="${REPO_ROOT}/flatpak/build/${arch}"
    local repo_dir="${REPO_ROOT}/flatpak/repo/${arch}"
    local manifest_path="${GEN_DIR}/${APP_ID}-${arch}.yaml"

    if [[ "${INCREMENTAL}" == "1" ]]; then
        rm -rf "${build_dir}"
        mkdir -p "${build_dir}" "${repo_dir}" "${DIST_DIR}"
    else
        rm -rf "${build_dir}" "${repo_dir}"
        mkdir -p "${build_dir}" "${repo_dir}" "${DIST_DIR}"
    fi

    if [[ "${INCREMENTAL}" == "1" ]]; then
        log "Building Flatpak incrementally (arch: ${arch})"
        flatpak-builder \
            --user \
            --arch="${arch}" \
            --repo="${repo_dir}" \
            "${build_dir}" \
            "${manifest_path}"
    else
        log "Building Flatpak from clean source tree (arch: ${arch})"
        flatpak-builder \
            --user \
            --force-clean \
            --arch="${arch}" \
            --repo="${repo_dir}" \
            "${build_dir}" \
            "${manifest_path}"
    fi

    log "Updating local repo metadata"
    flatpak build-update-repo "${repo_dir}" >/dev/null

    local app_ref
    app_ref="$(
        ostree --repo="${repo_dir}" refs 2>/dev/null \
            | awk -v app="${APP_ID}" -v arch="${arch}" '
                $0 ~ "^app/" app "/" arch "/" { print; exit }
            '
    )"

    [[ -n "${app_ref}" ]] || {
        ostree --repo="${repo_dir}" refs >&2 || true
        die "No exported Flatpak ref found after build"
    }

    local exported_branch
    exported_branch="${app_ref##*/}"

    log "Bundling ${app_ref}"
    flatpak build-bundle \
        "${repo_dir}" \
        "${DIST_DIR}/${APP_ID}-${arch}.flatpak" \
        "${APP_ID}" \
        "${exported_branch}"
}

main() {
    need_cmd bash
    need_cmd flatpak
    need_cmd flatpak-builder
    need_cmd ostree
    need_cmd curl
    need_cmd python3
    need_cmd awk
    need_cmd grep
    need_cmd rsync

    if [[ "${INCREMENTAL}" != "0" && "${INCREMENTAL}" != "1" ]]; then
        die "VERTEX_FLATPAK_INCREMENTAL must be 0 or 1"
    fi

    [[ -f "${REPO_ROOT}/Cargo.toml" ]] || die "Missing Cargo.toml"
    [[ -f "${REPO_ROOT}/flatpak/${APP_ID}.desktop" ]] || die "Missing desktop file"
    [[ -f "${REPO_ROOT}/flatpak/${APP_ID}.metainfo.xml" ]] || die "Missing metainfo file"
    [[ -f "${REPO_ROOT}/Vertex.svg" ]] || die "Missing Vertex.svg"

    ensure_flathub_remote

    # Parse comma-separated arch list
    IFS=',' read -ra ARCH_LIST <<< "${ARCHES}"

    local source_prepared=0
    for raw_arch in "${ARCH_LIST[@]}"; do
        local arch
        arch="$(normalize_arch "${raw_arch}")"

        # For aarch64 on a non-arm64 host, delegate to the container helper
        if [[ "${arch}" == "aarch64" && -z "${VERTEX_IN_ARM64_CONTAINER:-}" ]]; then
            log "Delegating aarch64 build to ARM64 container helper"
            VERTEX_FLATPAK_BRANCH="${BRANCH}" bash "${SCRIPT_DIR}/build-flatpak-arm64-container.sh"
            log "Bundle: ${DIST_DIR}/${APP_ID}-aarch64.flatpak"
            continue
        fi

        ensure_runtime_bits

        if [[ "${source_prepared}" -eq 0 ]]; then
            prepare_clean_source_tree
            generate_cargo_sources
            source_prepared=1
        fi

        write_manifest "${arch}"
        build_flatpak "${arch}"

        log "Bundle: ${DIST_DIR}/${APP_ID}-${arch}.flatpak"
    done

    log "Done"
}

main "$@"
