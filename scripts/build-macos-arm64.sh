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
    # Newest SDK first: an old one cannot build the engine (see the libc++ check below).
    local candidate
    while IFS= read -r candidate; do
        [[ -d "${candidate}" ]] && printf '%s\n' "${candidate}" && return 0
    done < <(printf '%s\n' "${HOME}/.local/share/macos-sdk/MacOSX.sdk" "${HOME}/.local/share/macos-sdk"/MacOSX*.sdk | sort -rV)
    return 1
}

SDK_ROOT="$(resolve_macos_sdk)" || {
    echo "No Apple SDK found. Set SDKROOT, DEVELOPER_DIR, have xcrun on PATH," \
         "or place SDK at ~/.local/share/macos-sdk/MacOSX*.sdk" >&2
    exit 1
}

export STURDY_RS_DEPS_DIR="${STURDY_RS_DEPS_DIR:-${REPO_ROOT}/.cache/sturdy-deps}"

# The engine is C++26. Its C++ is compiled against the libc++ headers bundled with zig and linked (by cargo-zigbuild, below)
# against the libc++ zig builds from the same sources, so both sides agree; the SDK only provides the system headers and
# frameworks. (Mixing in the build machine's libc++ or the SDK's would leave symbols undefined at the final link.)
ZIG_LIB="$(zig env | sed -n 's/^[[:space:]]*\.lib_dir = "\([^"]*\)".*/\1/p')"
if [[ -z "${ZIG_LIB}" || ! -d "${ZIG_LIB}/libcxx/include" ]]; then
    echo "Could not find zig's bundled libc++ headers (looked in '${ZIG_LIB}/libcxx/include')." >&2
    exit 1
fi
# The defines zig itself compiles C++ with (see: zig c++ -### ...), so the headers select the same ABI as its libc++.
LIBCXX_FLAGS="-nostdinc++ -isystem ${ZIG_LIB}/libcxx/include -isystem ${ZIG_LIB}/libcxxabi/include"
for define in _LIBCPP_ABI_VERSION=1 _LIBCPP_ABI_NAMESPACE=__1 _LIBCPP_HAS_THREADS=1 _LIBCPP_HAS_MONOTONIC_CLOCK \
    _LIBCPP_HAS_TERMINAL _LIBCPP_HAS_MUSL_LIBC=0 _LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS \
    _LIBCPP_HAS_VENDOR_AVAILABILITY_ANNOTATIONS=0 _LIBCPP_HAS_FILESYSTEM=1 _LIBCPP_HAS_RANDOM_DEVICE _LIBCPP_HAS_LOCALIZATION \
    _LIBCPP_HAS_UNICODE _LIBCPP_HAS_WIDE_CHARACTERS _LIBCPP_HAS_NO_STD_MODULES _LIBCPP_PSTL_BACKEND_SERIAL \
    _LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_NONE _LIBCPP_ASSERTION_SEMANTIC_DEFAULT=_LIBCPP_ASSERTION_SEMANTIC_IGNORE; do
    LIBCXX_FLAGS+=" -D${define}"
done

# The engine's C++ (CMake) cross compiles with clang + ld64.lld against the SDK; sturdy-sys picks this file up through
# CMAKE_TOOLCHAIN_FILE_<target>. Rust's own link goes through cargo-zigbuild.
# Absolute compiler paths: a bare "clang" resolves for C/C++ but not when the engine enables Objective-C in a fresh build
# directory ("CMAKE_OBJC_COMPILER: clang is not a full path and was not found in the PATH").
CLANG_BIN="$(dirname "$(command -v clang)")"
MACOS_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
TOOLCHAIN_FILE="${REPO_ROOT}/.cache/macos-arm64-toolchain.cmake"
mkdir -p "$(dirname "${TOOLCHAIN_FILE}")"
cat > "${TOOLCHAIN_FILE}" <<EOF
set(CMAKE_SYSTEM_NAME Darwin)
set(CMAKE_SYSTEM_PROCESSOR arm64)
set(CMAKE_OSX_SYSROOT "${SDK_ROOT}" CACHE PATH "")
set(CMAKE_OSX_ARCHITECTURES arm64 CACHE STRING "")
set(CMAKE_OSX_DEPLOYMENT_TARGET ${MACOS_DEPLOYMENT_TARGET} CACHE STRING "")
set(CMAKE_C_COMPILER ${CLANG_BIN}/clang)
set(CMAKE_CXX_COMPILER ${CLANG_BIN}/clang++)
set(CMAKE_OBJC_COMPILER ${CLANG_BIN}/clang)
set(CMAKE_OBJCXX_COMPILER ${CLANG_BIN}/clang++)
set(CMAKE_C_COMPILER_TARGET arm64-apple-macos${MACOS_DEPLOYMENT_TARGET})
set(CMAKE_CXX_COMPILER_TARGET arm64-apple-macos${MACOS_DEPLOYMENT_TARGET})
set(CMAKE_OBJC_COMPILER_TARGET arm64-apple-macos${MACOS_DEPLOYMENT_TARGET})
set(CMAKE_OBJCXX_COMPILER_TARGET arm64-apple-macos${MACOS_DEPLOYMENT_TARGET})
# Assembly (Boost.Context's context switch) is assembled by clang too, and must be told the target or it assembles for this machine.
set(CMAKE_ASM_COMPILER ${CLANG_BIN}/clang)
set(CMAKE_ASM_COMPILER_TARGET arm64-apple-macos${MACOS_DEPLOYMENT_TARGET})
set(CMAKE_AR llvm-ar CACHE FILEPATH "")
set(CMAKE_RANLIB llvm-ranlib CACHE FILEPATH "")
set(CMAKE_INSTALL_NAME_TOOL llvm-install-name-tool CACHE FILEPATH "")
set(CMAKE_OBJCXX_FLAGS_INIT "${LIBCXX_FLAGS}")
# sturdy-sys passes -DCMAKE_CXX_FLAGS=..., which would shadow CMAKE_CXX_FLAGS_INIT, so append to the cache entry itself.
# The libc++ flags are always the tail of the entry, so a build directory left by an older version of this script (other defines)
# has them replaced rather than kept or doubled.
string(FIND "\${CMAKE_CXX_FLAGS}" "${LIBCXX_FLAGS}" _sturdy_libcxx_pos)
if(_sturdy_libcxx_pos EQUAL -1)
  string(REGEX REPLACE [=[ ?-nostdinc\+\+.*\$]=] "" _sturdy_cxx_base "\${CMAKE_CXX_FLAGS}")
  set(CMAKE_CXX_FLAGS "\${_sturdy_cxx_base} ${LIBCXX_FLAGS}" CACHE STRING "" FORCE)
endif()
set(CMAKE_EXE_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_MODULE_LINKER_FLAGS_INIT "-fuse-ld=lld")
set(CMAKE_FIND_ROOT_PATH "${SDK_ROOT}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
EOF
export CMAKE_TOOLCHAIN_FILE_aarch64_apple_darwin="${TOOLCHAIN_FILE}"
# zig's libc++ (a LLVM 22/23 snapshot) recurses in C++26 mode when a std::deque holds something built on std::expected
# ("satisfaction of constraint ... depends on itself" in <expected>), so the engine is built as C++23 for this target.
export STURDY_RS_CMAKE_ARGS="${STURDY_RS_CMAKE_ARGS:-} -DSTURDY_CPP_STANDARD=23"
export MACOSX_DEPLOYMENT_TARGET="${MACOS_DEPLOYMENT_TARGET}"

env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
    SDKROOT="${SDK_ROOT}" cargo zigbuild --release --target "${TARGET}" -p vertexlauncher

mkdir -p "${REPO_ROOT}/target/release"
cp "${REPO_ROOT}/target/${TARGET}/release/vertexlauncher" "${STAGED}"
echo "Staged: ${STAGED}"
