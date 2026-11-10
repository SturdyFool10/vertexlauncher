#!/usr/bin/env bash
set -euo pipefail

# Cross compiles the aarch64 Linux binary on an x86_64 host (no emulation while compiling), in the same image as the
# x86_64 build: clang targets aarch64 against the arm64 sysroot baked into the image (/opt/sysroot-aarch64), and the
# engine's C++ goes through a CMake toolchain file written here (picked up by sturdy-sys through
# CMAKE_TOOLCHAIN_FILE_<target>).

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
WORK_ROOT="${REPO_ROOT}/.cache/linux-arm64-container"
TOOLCHAIN_CACHE_ROOT="${REPO_ROOT}/.cache/linux-x86_64-toolchain"
CARGO_HOME_DIR="${TOOLCHAIN_CACHE_ROOT}/cargo-home"
RUSTUP_HOME_DIR="${TOOLCHAIN_CACHE_ROOT}/rustup"
CONTAINER_DIR="${REPO_ROOT}/containers"

source "${REPO_ROOT}/scripts/lib/portable-linux-common.sh"

CONTAINER_IMAGE="${CONTAINER_IMAGE:-$(ensure_podman_image \
  linux-build \
  x86_64 \
  "${CONTAINER_DIR}/vertexlauncher-linux-build.Dockerfile" \
  "${CONTAINER_DIR}")}"

# Host-compiled build-script executables would not run against the container's glibc; remove them (from the host, as
# deleting from inside the container can fail on some mounts) so the container rebuilds them.
find "${REPO_ROOT}/target" -name "build-script-build" -delete 2>/dev/null || true
rm -rf "${REPO_ROOT}/target/release/build" 2>/dev/null || true
rm -f "${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/vertexlauncher" 2>/dev/null || true

mkdir -p "${WORK_ROOT}" "${CARGO_HOME_DIR}" "${RUSTUP_HOME_DIR}"

# While crates/vertex_shell points at a local SturdyEngine5 checkout by path (engine work in progress), mount it where
# that relative path lands inside the container. No effect once the dependency is a git revision again.
ENGINE_MOUNT=()
if [[ -d "${REPO_ROOT}/../SturdyEngine5" ]]; then
  ENGINE_MOUNT=(-v "$(cd "${REPO_ROOT}/../SturdyEngine5" && pwd):/SturdyEngine5")
fi

podman run --rm \
  --arch=amd64 \
  -v "${REPO_ROOT}:/workspace" \
  "${ENGINE_MOUNT[@]}" \
  -v "${WORK_ROOT}:/cache" \
  -v "${CARGO_HOME_DIR}:/usr/local/cargo" \
  -v "${RUSTUP_HOME_DIR}:/usr/local/rustup" \
  -w /workspace \
  "${CONTAINER_IMAGE}" \
  bash -lc '
    set -euo pipefail
    export PATH="/usr/local/cargo/bin:${PATH}"
    export CARGO_HOME=/usr/local/cargo
    export RUSTUP_HOME=/usr/local/rustup
    export HOME=/root
    export XDG_CACHE_HOME=/cache/xdg-cache
    # Separate from .cache/sturdy-deps on the host: CMake sub-build state there records absolute host paths.
    export STURDY_RS_DEPS_DIR=/workspace/.cache/sturdy-deps-container
    export XDG_DATA_HOME=/cache/xdg-data
    mkdir -p "${CARGO_HOME}" "${RUSTUP_HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}"

    echo "[linux-arm64] ensuring Rust toolchain..."
    if ! command -v rustup >/dev/null 2>&1; then
      curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
    fi
    if ! rustup toolchain list | grep -Eq "^stable($|-)"; then
      rustup toolchain install stable --profile minimal >/dev/null
    fi
    rustup default stable >/dev/null
    rustup target add aarch64-unknown-linux-gnu >/dev/null

    SYSROOT=/opt/sysroot-aarch64
    TRIPLE=aarch64-linux-gnu
    CROSS_FLAGS="--target=${TRIPLE} --sysroot=${SYSROOT}"

    # C/C++ built by cc-rs, and the final link.
    export CC_aarch64_unknown_linux_gnu=clang
    export CXX_aarch64_unknown_linux_gnu=clang++
    export AR_aarch64_unknown_linux_gnu=llvm-ar
    export CFLAGS_aarch64_unknown_linux_gnu="${CROSS_FLAGS}"
    export CXXFLAGS_aarch64_unknown_linux_gnu="${CROSS_FLAGS}"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=clang
    # --allow-shlib-undefined: the transitive libraries of WebKitGTK resolve at run time on the target.
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=--target=${TRIPLE} -C link-arg=--sysroot=${SYSROOT} -C link-arg=-fuse-ld=lld -C link-arg=-Wl,--allow-shlib-undefined"

    # pkg-config (GTK/WebKit for wry) against the sysroot.
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"
    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/${TRIPLE}/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    unset PKG_CONFIG_PATH

    # The engine (CMake) through a toolchain file.
    TOOLCHAIN_FILE=/cache/aarch64-linux-gnu.cmake
    cat > "${TOOLCHAIN_FILE}" <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_SYSROOT ${SYSROOT})
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_C_COMPILER_TARGET ${TRIPLE})
set(CMAKE_CXX_COMPILER_TARGET ${TRIPLE})
set(CMAKE_ASM_COMPILER clang)
set(CMAKE_ASM_COMPILER_TARGET ${TRIPLE})
set(CMAKE_AR llvm-ar CACHE FILEPATH "")
set(CMAKE_RANLIB llvm-ranlib CACHE FILEPATH "")
set(CMAKE_LIBRARY_ARCHITECTURE ${TRIPLE})
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_FIND_ROOT_PATH ${SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
set(ENV{PKG_CONFIG_SYSROOT_DIR} ${SYSROOT})
set(ENV{PKG_CONFIG_LIBDIR} ${SYSROOT}/usr/lib/${TRIPLE}/pkgconfig:${SYSROOT}/usr/share/pkgconfig)
EOF
    export CMAKE_TOOLCHAIN_FILE_aarch64_unknown_linux_gnu="${TOOLCHAIN_FILE}"

    bash /workspace/scripts/patch-wry-source.sh

    echo "[linux-arm64] cleaning stale workspace build-script artifacts..."
    cargo clean --package vertexlauncher --package launcher_ui || true

    echo "[linux-arm64] building release artifact..."
    cargo build --release --target aarch64-unknown-linux-gnu -p vertexlauncher

    echo "[linux-arm64] Built packaging input: /workspace/target/aarch64-unknown-linux-gnu/release/vertexlauncher"
  '
