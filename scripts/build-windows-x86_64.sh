#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=x86_64-pc-windows-msvc
STAGED="${REPO_ROOT}/target/release/vertexlauncher-windows-x86_64.exe"

env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
    cargo xwin build --release --target "${TARGET}" --cross-compiler clang -p vertexlauncher

mkdir -p "${REPO_ROOT}/target/release"
cp "${REPO_ROOT}/target/${TARGET}/release/vertexlauncher.exe" "${STAGED}"
echo "Staged: ${STAGED}"
