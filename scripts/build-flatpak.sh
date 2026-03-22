#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

APP_ID="io.github.SturdyFool10.VertexLauncher"
BRANCH="${VERTEX_FLATPAK_BRANCH:-stable}"
ARCH="${VERTEX_FLATPAK_ARCH:-$(flatpak --default-arch 2>/dev/null || uname -m)}"

RUNTIME="org.gnome.Platform"
RUNTIME_VERSION="49"
SDK="org.gnome.Sdk"
RUST_EXT="org.freedesktop.Sdk.Extension.rust-stable"
RUST_EXT_TAG="25.08"

GEN_DIR="${REPO_ROOT}/flatpak/generated"
SOURCE_TREE="${GEN_DIR}/source-tree"
BUILD_DIR="${REPO_ROOT}/flatpak/build/${ARCH}"
REPO_DIR="${REPO_ROOT}/flatpak/repo/${ARCH}"
DIST_DIR="${REPO_ROOT}/target/release"
MANIFEST_PATH="${GEN_DIR}/${APP_ID}-${ARCH}.yaml"
CARGO_SOURCES_PATH="${GEN_DIR}/cargo-sources.json"
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
    log "Preparing clean Flatpak source tree"
    rm -rf "${SOURCE_TREE}"
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

    log "Generating cargo-sources.json"
    python3 "${GENERATOR_PATH}" "${SOURCE_TREE}/Cargo.lock" -o "${CARGO_SOURCES_PATH}"
    [[ -f "${CARGO_SOURCES_PATH}" ]] || die "Failed to generate cargo-sources.json"
}

write_manifest() {
    mkdir -p "${GEN_DIR}"

    cat > "${MANIFEST_PATH}" <<MANIFEST
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
  - --socket=fallback-x11
  - --socket=pulseaudio
  - --device=dri
  - --filesystem=home
  - --talk-name=org.freedesktop.secrets
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
    rm -rf "${BUILD_DIR}" "${REPO_DIR}"
    mkdir -p "${BUILD_DIR}" "${REPO_DIR}" "${DIST_DIR}"

    log "Building Flatpak from clean source tree"
    flatpak-builder \
        --user \
        --force-clean \
        --arch="${ARCH}" \
        --repo="${REPO_DIR}" \
        "${BUILD_DIR}" \
        "${MANIFEST_PATH}"

    log "Updating local repo metadata"
    flatpak build-update-repo "${REPO_DIR}" >/dev/null

    local app_ref
    app_ref="$(
        ostree --repo="${REPO_DIR}" refs 2>/dev/null \
            | awk -v app="${APP_ID}" -v arch="${ARCH}" '
                $0 ~ "^app/" app "/" arch "/" { print; exit }
            '
    )"

    [[ -n "${app_ref}" ]] || {
        ostree --repo="${REPO_DIR}" refs >&2 || true
        die "No exported Flatpak ref found after build"
    }

    local exported_branch
    exported_branch="${app_ref##*/}"

    log "Bundling ${app_ref}"
    flatpak build-bundle \
        "${REPO_DIR}" \
        "${DIST_DIR}/${APP_ID}-${ARCH}.flatpak" \
        "${APP_ID}" \
        "${exported_branch}"
}

main() {
    ARCH="$(normalize_arch "${ARCH}")"

    need_cmd bash
    need_cmd flatpak
    need_cmd flatpak-builder
    need_cmd ostree
    need_cmd curl
    need_cmd python3
    need_cmd awk
    need_cmd grep
    need_cmd rsync

    [[ -f "${REPO_ROOT}/Cargo.toml" ]] || die "Missing Cargo.toml"
    [[ -f "${REPO_ROOT}/flatpak/${APP_ID}.desktop" ]] || die "Missing desktop file"
    [[ -f "${REPO_ROOT}/flatpak/${APP_ID}.metainfo.xml" ]] || die "Missing metainfo file"
    [[ -f "${REPO_ROOT}/Vertex.svg" ]] || die "Missing Vertex.svg"

    ensure_flathub_remote
    ensure_runtime_bits
    prepare_clean_source_tree
    generate_cargo_sources
    write_manifest
    build_flatpak

    log "Done"
    log "Bundle: ${DIST_DIR}/${APP_ID}-${ARCH}.flatpak"
    log "Repo:   ${REPO_DIR}"
}

main "$@"
