#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=aarch64-apple-darwin
STAGED="${REPO_ROOT}/target/release/vertexlauncher-macos-arm64"

resolve_macos_sdk() {
    if [[ -n "${SDKROOT:-}" ]] && [[ -d "${SDKROOT}" ]]; then
        printf '%s\n' "${SDKROOT}"; return 0
    fi
    if [[ -n "${DEVELOPER_DIR:-}" ]] && [[ -d "${DEVELOPER_DIR}" ]]; then
        printf '%s\n' "${DEVELOPER_DIR}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk"; return 0
    fi
    if command -v xcrun >/dev/null 2>&1; then
        local sdk
        sdk="$(xcrun --sdk macosx --show-sdk-path 2>/dev/null)" && [[ -n "${sdk}" ]] && printf '%s\n' "${sdk}" && return 0
    fi
    for candidate in "${HOME}/.local/share/macos-sdk/MacOSX.sdk" "${HOME}/.local/share/macos-sdk"/MacOSX*.sdk; do
        [[ -d "${candidate}" ]] && printf '%s\n' "${candidate}" && return 0
    done
    return 1
}

SDK_ROOT="$(resolve_macos_sdk)" || {
    echo "No Apple SDK found. Set SDKROOT, DEVELOPER_DIR, have xcrun on PATH," \
         "or place SDK at ~/.local/share/macos-sdk/MacOSX*.sdk" >&2
    exit 1
}

env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
    SDKROOT="${SDK_ROOT}" cargo zigbuild --release --target "${TARGET}" -p vertexlauncher

mkdir -p "${REPO_ROOT}/target/release"
cp "${REPO_ROOT}/target/${TARGET}/release/vertexlauncher" "${STAGED}"
echo "Staged: ${STAGED}"
