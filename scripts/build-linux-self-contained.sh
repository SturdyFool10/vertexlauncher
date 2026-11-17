#!/usr/bin/env bash
# Builds Vertex as one Linux executable that needs only the C library of the machine it runs on, then checks that it really does.
#
# What makes that so (details in crates/vertexlauncher/build.rs and the engine's cmake/SturdySelfContained.cmake):
#   * the C++ engine, its libstdc++ and its third-party libraries are static archives linked into the binary;
#   * OpenSSL is built from source and linked statically (sturdy-sys feature `bundled-openssl`);
#   * liblzma is built from its bundled source (xz2 feature `static`);
#   * libgcc is linked statically instead of through libgcc_s.so.1;
#   * libudev (gamepads) is opened at run time (crates/libudev-shim), like the Vulkan loader and the X11/Wayland libraries;
#   * glibc symbols newer than the floor are pinned to their old versions (the engine's glibc_compat.c), so the binary starts on any
#     distribution with glibc 2.34 or newer rather than only on the one that built it;
#   * there is no embedded sign-in webview (WebKitGTK cannot be carried in an executable); Microsoft sign-in uses the system browser or a
#     device code.
#
# usage: scripts/build-linux-self-contained.sh [extra cargo arguments]
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

cargo build --release -p vertexlauncher "$@"

# The release profile keeps debug info (`debug = true`) for profiling and backtraces, which makes the executable about 1 GB. The
# shippable copy is stripped; the unstripped one stays next to it for symbolication.
BUILT="${REPO_ROOT}/target/release/vertexlauncher"
DIST="${REPO_ROOT}/target/release/vertexlauncher-self-contained"
strip --strip-all -o "${DIST}" "${BUILT}"
echo "stripped: $(du -h "${BUILT}" | cut -f1) -> $(du -h "${DIST}" | cut -f1)  (${DIST})"

bash "${SCRIPT_DIR}/check-linux-self-contained.sh" "${DIST}"
