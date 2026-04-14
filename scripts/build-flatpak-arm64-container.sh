#!/usr/bin/env bash
set -euo pipefail

# Build an aarch64 Flatpak from an x86_64 host/container.
#
# STRATEGY: two-phase build for maximum performance.
#
# Phase 1 — cross-compile (native speed, no QEMU):
#   Run x86_64 rustc with --target aarch64-unknown-linux-gnu.
#   QEMU is not involved at all during Rust compilation.
#
# Phase 2 — flatpak packaging (trivial, minimal QEMU use):
#   flatpak-builder --arch=aarch64 only runs shell `install` commands inside
#   bwrap; the heavy Rust compilation is already done.  QEMU overhead is
#   negligible for file-copy operations.
#
# WHY SYSROOT APPROACH:
#   In Debian bookworm, libglib2.0-dev:arm64 pulls in gobject-introspection:arm64
#   which requires python3:arm64 + build-essential:arm64 — unavailable for cross-
#   compilation.  We bypass apt's dependency checker by using `apt-get download`
#   (single-package, no dep check) and extracting into a sysroot directory.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CONTAINER_IMAGE="${CONTAINER_IMAGE:-docker.io/library/debian:bookworm}"
WORK_ROOT="${REPO_ROOT}/.cache/flatpak-arm64-container"

bash "${REPO_ROOT}/scripts/compile-slang-shaders.sh"

# Remove host-compiled build-script executables and the aarch64 binary before
# entering the container so that the container recompiles them against its glibc.
find "${REPO_ROOT}/target" -name "build-script-build" -delete 2>/dev/null || true
rm -f "${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/vertexlauncher" 2>/dev/null || true

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
    export HOME=/cache/home
    export XDG_CACHE_HOME=/cache/xdg-cache
    export XDG_DATA_HOME=/cache/xdg-data
    export CARGO_HOME=/cache/cargo
    export RUSTUP_HOME=/cache/rustup
    export SYSROOT=/cache/arm64-sysroot
    mkdir -p "${HOME}" "${XDG_CACHE_HOME}" "${XDG_DATA_HOME}" "${CARGO_HOME}" "${RUSTUP_HOME}"

    # ------------------------------------------------------------------ #
    # Phase 1 setup: amd64 tools
    # ------------------------------------------------------------------ #
    echo "[flatpak-arm64] installing amd64 toolchain..."

    dpkg --add-architecture arm64
    apt-get update -qq

    apt-get install -y --no-install-recommends \
      ca-certificates curl \
      gcc libc6-dev gcc-aarch64-linux-gnu libc6-dev-arm64-cross binutils-aarch64-linux-gnu \
      pkg-config \
      >/dev/null

    apt-get install -y --no-install-recommends \
      elfutils appstream-compose \
      flatpak flatpak-builder ostree \
      python3 python3-aiohttp python3-tomlkit \
      qemu-user-static rsync xz-utils zstd \
      librsvg2-2 librsvg2-common librsvg2-bin \
      >/dev/null

    # Ensure gdk-pixbuf SVG loader is registered so flatpak build-export
    # can validate the Vertex.svg icon.  In minimal containers the dpkg
    # trigger may not run automatically after apt-get install.
    gdk-pixbuf-query-loaders --update-cache 2>/dev/null || true

    # ------------------------------------------------------------------ #
    # Build arm64 sysroot by downloading .deb files directly.
    # We bypass apt dependency resolution because gobject-introspection:arm64
    # needs python3:arm64 + build-essential:arm64 (not available cross-host).
    # ------------------------------------------------------------------ #
    if [[ ! -f "${SYSROOT}/.stamp" ]]; then
      echo "[flatpak-arm64] building arm64 sysroot (cached for future runs)..."
      mkdir -p "${SYSROOT}" /tmp/arm64-debs
      cd /tmp/arm64-debs

      # All packages needed for headers + link stubs.
      # Include both the runtime (.so) and dev (headers + .so symlinks) packages.
      ARM64_PKGS=(
        # webkit2gtk / wry
        libwebkit2gtk-4.1-0 libwebkit2gtk-4.1-dev
        libjavascriptcoregtk-4.1-0 libjavascriptcoregtk-4.1-dev
        libsoup-3.0-0 libsoup-3.0-dev
        # gtk3 / pango / cairo / gdk-pixbuf / atk
        libgtk-3-0 libgtk-3-dev
        libpango-1.0-0 libpangocairo-1.0-0 libpangoft2-1.0-0 libpango1.0-dev
        libfribidi0 libfribidi-dev
        libthai0 libthai-dev
        libxft2 libxft-dev
        libcairo2 libcairo-gobject2 libcairo2-dev
        libgdk-pixbuf-2.0-0 libgdk-pixbuf-2.0-dev
        libatk1.0-0 libatk1.0-dev
        libepoxy0 libepoxy-dev
        # glib / gio / gobject
        libglib2.0-0 libglib2.0-dev
        libgio-2.0-0
        # SSL / crypto
        libssl3 libssl-dev
        libcrypto3
        # udev / dbus / secret
        libudev1 libudev-dev
        libdbus-1-3 libdbus-1-dev
        libsecret-1-0 libsecret-1-dev
        libgcrypt20 libgcrypt20-dev
        libgpg-error0 libgpg-error-dev
        # xkbcommon / wayland
        libxkbcommon0 libxkbcommon-dev
        libwayland-client0 libwayland-server0 libwayland-cursor0 libwayland-egl1 libwayland-dev
        # vulkan
        libvulkan1 libvulkan-dev
        # common transitive deps
        libffi8 libffi-dev
        zlib1g zlib1g-dev
        libpcre2-8-0 libpcre2-dev
        libmount1 libmount-dev
        libblkid1 libblkid-dev
        libselinux1 libselinux1-dev
        libpixman-1-0 libpixman-1-dev
        libfreetype6 libfreetype-dev
        libfontconfig1 libfontconfig-dev
        libpng16-16 libpng-dev
        libjpeg62-turbo libjpeg62-turbo-dev
        libtiff6 libtiff-dev
        libwebp7 libwebp-dev
        libenchant-2-2
        libhyphen0
        libwoff1
        # gstreamer (required by WebKit media pipeline)
        libgstreamer1.0-0 libgstreamer1.0-dev
        libgstreamer-plugins-base1.0-0 libgstreamer-plugins-base1.0-dev
        # xml / http2 / sqlite (WebKit transitive deps)
        libxml2 libxml2-dev
        libnghttp2-14 libnghttp2-dev
        libsqlite3-0 libsqlite3-dev
        libdrm2 libdrm-dev
        libx11-6 libx11-dev
        libxext6 libxext-dev
        libxcb1 libxcb-dev libxcb1-dev
        libxcb-render0 libxcb-render0-dev
        libxcb-shm0 libxcb-shm0-dev
        libxcb-xfixes0 libxcb-xfixes0-dev
        libxrender1 libxrender-dev
        libxi6 libxi-dev
        libxinerama1 libxinerama-dev
        libxrandr2 libxrandr-dev
        libxcursor1 libxcursor-dev
        libxfixes3 libxfixes-dev
        libxcomposite1 libxcomposite-dev
        libxdamage1 libxdamage-dev
        libxtst6
        libasound2 libasound2-dev
        libpulse0
        libice6
        libsm6
        libgio-fam0
        libglib2.0-bin
        libglib2.0-data
        libgtk-3-bin
        libgdk-pixbuf2.0-bin
        librsvg2-2
        libsoup2.4-1
        libsqlite3-0
        libgnutls30
        libgnutls-dane0
        libharfbuzz0b libharfbuzz-dev
        libgraphite2-3 libgraphite2-dev
        libgirepository-1.0-1
        libatk-bridge2.0-0 libatk-bridge2.0-dev
        libatspi2.0-0
        libdw1
        libelf1
        libunwind8
        libgdbm6
        libseccomp2
        libcap2
        libnsl2
        # Additional dependencies for complete cross-compilation
        libbrotli1 libbrotli-dev
        libexpat1 libexpat-dev
        libxau6 libxdmcp6
        libxcb-glx0 libxcb-keysyms1 libxcb-image0 libxcb-render-util0
        libxcb-shape0 libxcb-sync1 libxcb-xkb1
        libxcvt0 libxxf86vm1 libxxf86dga1
        libxshmfence1 libxfixes3
        libgl1-mesa-glx libglapi-mesa libegl1-mesa libgbm1
        libwayland-bin
        # ICU (required by WebKit / JavaScriptCore)
        libicu72 libicu-dev
        # systemd (udev runtime dep)
        libsystemd0 libsystemd-dev
        # XSLT (WebKit transitive dep)
        libxslt1.1 libxslt1-dev
        # Color management (WebKit transitive dep)
        liblcms2-2 liblcms2-dev
        # Flite TTS (WebKit accessibility dep)
        libflite1
        # WebP mux (additional WebP codec)
        libwebpmux3
        # Additional transitive deps
        libsepol2 libsepol-dev
        libpcre3 libpcre3-dev
      )

      for pkg in "${ARM64_PKGS[@]}"; do
        apt-get download "${pkg}:arm64" 2>/dev/null \
          && echo "  downloaded ${pkg}:arm64" \
          || echo "  WARNING: skipped ${pkg}:arm64 (not found)"
      done

      # Architecture-independent packages that provide .pc files (no :arm64 suffix)
      ARCHALL_PKGS=(
        shared-mime-info
        glib-networking
        libglib2.0-doc
        x11-common
        xtrans-dev
      )
      for pkg in "${ARCHALL_PKGS[@]}"; do
        apt-get download "${pkg}" 2>/dev/null \
          && echo "  downloaded ${pkg}" \
          || echo "  WARNING: skipped ${pkg} (not found)"
      done

      # Extract all downloaded debs into sysroot
      for deb in /tmp/arm64-debs/*.deb; do
        dpkg -x "${deb}" "${SYSROOT}/" 2>/dev/null || true
      done

      # Fix .so symlinks: for each lib.so.X.Y create intermediate symlinks
      # lib.so.X -> lib.so.X.Y and lib.so -> lib.so.X so the linker can
      # resolve both the soname (-lxxx) and the versioned name.
      # Process both /usr/lib and /lib paths (some packages install to /lib).
      for sodir in \
          "${SYSROOT}/usr/lib/aarch64-linux-gnu" \
          "${SYSROOT}/lib/aarch64-linux-gnu"; do
        find "${sodir}" -maxdepth 1 -name "*.so.*" 2>/dev/null \
          | while IFS= read -r versioned; do
              name="${versioned}"
              while true; do
                shorter="${name%.[0-9]*}"
                [[ "${shorter}" == "${name}" ]] && break
                [[ -e "${shorter}" ]] || ln -sf "$(basename "${name}")" "${shorter}" 2>/dev/null || true
                name="${shorter}"
              done
            done
      done

      # Mirror /lib/aarch64-linux-gnu into /usr/lib/aarch64-linux-gnu so the
      # linker (which only searches the sysroot-relative usr/lib path) can find
      # libraries that Debian installed under /lib rather than /usr/lib.
      if [[ -d "${SYSROOT}/lib/aarch64-linux-gnu" ]]; then
        find "${SYSROOT}/lib/aarch64-linux-gnu" -maxdepth 1 \( -name "*.so*" -o -name "*.a" \) \
          | while IFS= read -r src; do
              dst="${SYSROOT}/usr/lib/aarch64-linux-gnu/$(basename "${src}")"
              [[ -e "${dst}" ]] || ln -sf "${src}" "${dst}" 2>/dev/null || true
            done
      fi

      # Strip Requires.private from all .pc files.
      # pkgconf (Debian bookworm pkg-config) validates ALL Requires.private
      # transitively even for dynamic linking, causing failures for missing
      # transitive deps (e.g. libbrotlidec, xproto, expat, libsepol).
      # We only need dynamic linking here so Requires.private are irrelevant.
      find "${SYSROOT}/usr/lib/aarch64-linux-gnu/pkgconfig" -name "*.pc" \
        -exec sed -i "/^Requires\.private/d" {} \; 2>/dev/null || true
      find "${SYSROOT}/usr/share/pkgconfig" -name "*.pc" \
        -exec sed -i "/^Requires\.private/d" {} \; 2>/dev/null || true

      touch "${SYSROOT}/.stamp"
      echo "[flatpak-arm64] arm64 sysroot ready at ${SYSROOT}"
    else
      echo "[flatpak-arm64] using cached arm64 sysroot at ${SYSROOT}"
    fi

    # ------------------------------------------------------------------ #
    # Install Rust via rustup (cached in /cache/rustup)
    # ------------------------------------------------------------------ #
    if [[ ! -x "${CARGO_HOME}/bin/rustup" ]]; then
      echo "[flatpak-arm64] installing rustup..."
      curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --default-toolchain stable
    fi
    export PATH="${CARGO_HOME}/bin:${PATH}"

    # ------------------------------------------------------------------ #
    # Phase 1: cross-compile with native x86_64 rustc → aarch64 binary
    # ------------------------------------------------------------------ #
    echo "[flatpak-arm64] adding aarch64 Rust target..."
    rustup target add aarch64-unknown-linux-gnu

    echo "[flatpak-arm64] cross-compiling vertexlauncher for aarch64..."
    cd /workspace

    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    # Force host (x86_64) build scripts to use system gcc, not the rustup bundled lld.
    # Rust >= 1.79 defaults to rust-lld on Linux which does not know Debian library paths.
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"
    export PKG_CONFIG_PATH="${SYSROOT}/usr/lib/aarch64-linux-gnu/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
    export PKG_CONFIG_LIBDIR="${SYSROOT}/usr/lib/aarch64-linux-gnu/pkgconfig:${SYSROOT}/usr/share/pkgconfig"

    # Allow unresolved symbols from shared libraries (transitive deps of webkit2gtk,
    # gstreamer, etc.) during cross-compilation. These are all runtime-resolved by
    # the Flatpak org.gnome.Platform runtime — we do not need to verify them at link
    # time. Without this flag ld fails on hundreds of ICU, GStreamer, X11, etc. refs
    # that are transitively imported through the stub .so files in the sysroot.
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined"

    # Additional environment for cross-compilation stability
    export CARGO_BUILD_TARGET=aarch64-unknown-linux-gnu

    cargo build --release --locked \
      --target aarch64-unknown-linux-gnu \
      -p vertexlauncher

    PREBUILT=/workspace/flatpak/generated/prebuilt-aarch64
    mkdir -p "${PREBUILT}"
    cp /workspace/target/aarch64-unknown-linux-gnu/release/vertexlauncher \
       "${PREBUILT}/vertexlauncher"
    echo "[flatpak-arm64] cross-compiled binary: $(file ${PREBUILT}/vertexlauncher)"

    # ------------------------------------------------------------------ #
    # Phase 2: flatpak packaging (installs the prebuilt binary, no cargo)
    # ------------------------------------------------------------------ #
    echo "[flatpak-arm64] packaging aarch64 Flatpak..."

    if [ -d /proc/sys/fs/binfmt_misc ]; then
      update-binfmts --enable qemu-aarch64 2>/dev/null || true
    fi

    export VERTEX_IN_ARM64_CONTAINER=1
    export VERTEX_FLATPAK_ARCHES=aarch64
    export VERTEX_PREBUILT_AARCH64="${PREBUILT}/vertexlauncher"
    bash /workspace/scripts/build-flatpak.sh
  '
