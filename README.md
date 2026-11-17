Come to our Discord: [![Discord](https://img.shields.io/discord/1480105103414530190?label=Discord%20Members&logo=discord)](https://discord.gg/EJGUFeuGrN)

# Vertex Launcher

Native Minecraft launcher written in Rust.

Vertex Launcher is a multi-crate desktop launcher with:

- native desktop UI built on [SturdyEngine5](https://github.com/sturdyfool10/sturdyengine5), an immediate-mode UI on a GPU renderer
- Microsoft and Minecraft account sign-in
- multi-instance management
- runtime/bootstrap setup for Minecraft and Java
- in-app Modrinth and CurseForge browsing
- quick-launch CLI flows for packs, worlds, and servers

## Building

A plain `git clone` builds with no extra steps. `Cargo.lock` is tracked, so everyone resolves the same dependency versions. The first build also compiles the SturdyEngine5 C++ engine (it is fetched as a git dependency), which takes a while and needs the tools listed below.

```sh
git clone <repo-url> vertexlauncher
cd vertexlauncher
cargo build --release --locked
```

### Requirements on every platform

- Rust 1.92 or newer (edition 2024; `rustup` is the easiest way to get it)
- Git
- a C compiler and linker (several crates compile bundled C code: `ring`, `onig_sys`, `lzma-sys`, vendored `libdbus`)
- Git, CMake 3.28 or newer, Ninja, and Clang/Clang++ with C++26 support plus a libstdc++ that ships `libstdc++.a` (the SturdyEngine5 engine is built from source by its `sturdy-sys` crate; its build script documents the `STURDY_RS_*` environment knobs, e.g. `STURDY_RS_DEPS_DIR` to share downloaded third-party sources and `STURDY_RS_OFFLINE=1` for offline builds). At run time the launcher needs only a Vulkan driver (a Direct3D 12 driver on Windows) and, on Linux, `libwayland-client`.

TLS is done with rustls, so OpenSSL is not needed.

### Linux

Native builds need `pkg-config`. Wayland and X11 libraries are loaded at runtime and are not build dependencies.

Debian/Ubuntu (22.04+ / Debian 12+):

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config curl git \
  libglib2.0-dev libgtk-3-dev libgdk-pixbuf-2.0-dev libpango1.0-dev \
  libatk1.0-dev libcairo2-dev libsoup-3.0-dev \
  libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libudev-dev
```

Fedora:

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config git \
  glib2-devel gtk3-devel gdk-pixbuf2-devel pango-devel atk-devel cairo-devel \
  libsoup3-devel webkit2gtk4.1-devel javascriptcoregtk4.1-devel systemd-devel
```

Arch:

```sh
sudo pacman -S --needed base-devel pkgconf git gtk3 webkit2gtk-4.1 libsoup3 systemd-libs
```

The binary also needs a working GPU driver at runtime (Vulkan or OpenGL). If your distribution does not ship `webkit2gtk-4.1`, build with the containerized scripts below or use the Flatpak.

### Windows

Install the Visual Studio Build Tools (the "Desktop development with C++" workload, which includes the Windows SDK) and Rust's `x86_64-pc-windows-msvc` / `aarch64-pc-windows-msvc` target. `windows-gnu` is not part of the supported release matrix.

### macOS

Install the Xcode Command Line Tools (`xcode-select --install`). Only Apple Silicon (`aarch64-apple-darwin`) is part of the release matrix.

### Portable Linux builds

Linux release binaries should not be built on a rolling distro with plain `cargo build`, because that inherits the host glibc baseline. The x86-64 release scripts build in a CentOS 7 container so the binary links against a `glibc 2.17` baseline, and print the highest required glibc symbol version after each build (enforcing `VERTEX_MAX_GLIBC_VERSION=2.17` by default). Dependency versions are fixed by `Cargo.lock`; native builds need no source patching.

If you want the portable Linux deliverables instead of a host-linked binary, use the dedicated helper:

```sh
bash scripts/build-linux-portables.sh
```

On Linux x86-64, that helper defaults to building both `x86_64` and `aarch64` AppImage + Flatpak artifacts. On Linux ARM64, it defaults to `aarch64` only. The first run now builds reusable local Podman images for the CentOS 7 and Debian packaging environments and caches the x86-64 Rust toolchain plus the ARM64 CentOS sysroot, so repeat builds avoid most of the previous setup cost.

Windows MSVC example:

```sh
cargo build --release --target x86_64-pc-windows-msvc
```

## Release Matrix

The current supported release artifact matrix is:

- Windows x86-64: `x86_64-pc-windows-msvc`
- Windows ARM64: `aarch64-pc-windows-msvc`
- Linux x86-64: `x86_64-unknown-linux-gnu`
- Linux ARM64: `aarch64-unknown-linux-gnu`
- macOS ARM64: `aarch64-apple-darwin`

Installed Rust targets intentionally not used for release artifacts:

- `x86_64-pc-windows-gnu`
- `armv7-unknown-linux-gnueabihf`
- `x86_64-apple-darwin`

To build the staged release artifacts:

Linux/macOS:

```sh
fish scripts/build-release-artifacts.fish
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-release-artifacts.ps1
```

## Flatpak

Linux users who want a sandboxed portable package can use the Flatpak bundle. Vertex now packages Flatpak around the same CentOS 7-built Linux AppDir used for AppImage, so the launcher ELF staged into the bundle is checked against a `GLIBC_2.17` ceiling before Flatpak export. The Flatpak still runs against its selected runtime at launch time, but the packaged launcher binary itself stays aligned with the Linux `glibc 2.17+` target.

The Flatpak manifest grants home-directory filesystem access so the launcher can detect and import existing Minecraft / Modrinth / launcher install folders by path instead of being limited to its sandbox-private storage. It also explicitly grants access to the native Linux Vertex storage roots used in code: `~/.config/vertexlauncher`, `~/.local/share/vertexlauncher`, `~/.cache/vertexlauncher`, plus the legacy config root `~/.config/vertex-launcher`. It also exposes both Wayland and X11 sockets and sets `GDK_BACKEND=wayland,x11`, so the launcher prefers Wayland when available while still allowing Minecraft clients and Java mods to fall back to X11 when needed. For GPU compute, the manifest keeps the existing device permissions and adds Flatpak GL driver extension wiring, explicit OpenCL ICD library discovery for ROCm/NVIDIA/Mesa, read-only host OS driver-library access, read-only ROCm (`/opt/rocm`) and CUDA (`/opt/cuda`) toolkit access, plus udev metadata for OpenCL, HIP/ROCm, and CUDA-capable Java mods or native libraries.

To build the Flatpak bundle automatically:

```sh
bash scripts/build-flatpak.sh
```

For the full portable Linux matrix, prefer:

```sh
bash scripts/build-linux-portables.sh --formats flatpak
```

This script will:

- build or reuse the matching prebuilt Linux AppDir from the CentOS 7 packaging pipeline
- reject the build if the staged launcher binary exceeds `VERTEX_MAX_GLIBC_VERSION` (defaults to `2.17`)
- stage the AppDir into a Flatpak build directory and export an arch-specific bundle
- export an arch-specific local Flatpak repo under `flatpak/repo/<arch>`
- emit distributable bundles under `target/release/io.github.SturdyFool10.VertexLauncher-<arch>.flatpak`

By default the standalone helper builds the current Flatpak host architecture. Override `VERTEX_FLATPAK_ARCHES` with a comma-separated list such as `x86_64,aarch64` to request specific arches. Flatpak only allows builds for host-compatible arches unless the ARM64 container helper is used.

The release scripts also invoke the Flatpak helper. On Linux x86-64 they now default to staging both `x86_64` and `aarch64` Flatpak bundles, enabling the ARM64 container helper automatically when needed. Override that with `VERTEX_RELEASE_FLATPAK_ARCHES=<comma-separated arches>`.

The Flatpak app id is `io.github.SturdyFool10.VertexLauncher`.

The checked-in manifest at `flatpak/io.github.SturdyFool10.VertexLauncher.yaml` is now a reference/template for the staged source layout. Native host exports still generate arch-specific manifests under `flatpak/generated/`, while the ARM64 emulated export path stages the build directory directly to avoid `flatpak-builder` sandbox issues under emulation.

If you only have an x86-64 Linux builder but want an ARM64 Flatpak, set `VERTEX_FLATPAK_ARCHES=aarch64` and `VERTEX_ENABLE_ARM64_EMULATION=1`. That path first cross-compiles the ARM64 launcher on the host against a cached CentOS 7 ARM64 sysroot, then reuses the generated AppDir and performs only the final Flatpak export under emulation. It still depends on working `podman` plus host `binfmt_misc` / QEMU user emulation, but it is much faster than emulating the full Rust build.

## AppImage

Linux users who want a single-file portable launcher can also build an AppImage. Vertex AppImages bundle the launcher-adjacent GTK/WebKit support files, but intentionally do **not** bundle glibc or the dynamic loader; those are provided by the host because glibc private ABI mismatches can prevent AppImages from starting.

When Vertex runs from an AppImage, it now defaults to portable storage beside the AppImage itself unless `VERTEX_CONFIG_LOCATION` is already set. For example, running `VertexLauncher-x86_64.AppImage` will use `VertexLauncher-x86_64.AppImage.data/` for config, instances, cache, logs, and themes.

To build the AppImage bundle automatically on Linux:

```sh
bash scripts/build-appimage.sh
```

For the full portable Linux matrix, prefer:

```sh
bash scripts/build-linux-portables.sh --formats appimage
```

This helper expects:

- `linuxdeploy` on `PATH`, or `VERTEX_LINUXDEPLOY=/path/to/linuxdeploy*.AppImage`
- `appimagetool` on `PATH`, or `VERTEX_APPIMAGETOOL=/path/to/appimagetool*.AppImage`
- `linuxdeploy-plugin-gtk` on `PATH` is recommended for the GTK/WebKit runtime bundle, or `VERTEX_LINUXDEPLOY_GTK_PLUGIN=/path/to/linuxdeploy-plugin-gtk*`
- a matching native Linux release binary under `target/<triple>/release/vertexlauncher`

If `linuxdeploy` or `appimagetool` are missing, `build-appimage.sh` now downloads arch-matched AppImage builds into `.cache/appimage-tools/`. Override those download URLs with `VERTEX_LINUXDEPLOY_URL` and `VERTEX_APPIMAGETOOL_URL` if you need pinned mirrors or a private cache.

The staged AppImage artifact is written to `target/release` as one of:

- `vertexlauncher-linuxx86-64.AppImage`
- `vertexlauncher-linuxarm64.AppImage`

On Linux x86-64, the AppImage helper now prefers packaging inside the project’s Linux container when `podman` is available. Set `VERTEX_APPIMAGE_USE_CONTAINER=0` if you need to force host packaging.

During staging, the AppImage helper now also copies the WebKitGTK helper binaries, injected bundle, GTK IM modules, GDK Pixbuf loaders, schemas, and default theme assets into the AppDir, then patches the bundled WebKitGTK library to resolve those helper paths from inside the AppImage instead of the original system locations.

If you only have an x86-64 Linux builder but want an ARM64 AppImage, set `VERTEX_APPIMAGE_ARCH=aarch64`. The helper cross-compiles the ARM64 launcher on the host against a cached ARM64 sysroot, then runs only the final AppImage packaging step inside an emulated ARM64 Podman container. It will reuse or download ARM64-compatible `linuxdeploy` / `appimagetool` automatically.

The AppImage/Linux helpers reuse a shared cached Rust toolchain under `.cache/linux-x86_64-toolchain/`, so rebuilding x86-64 artifacts no longer has to bootstrap rustup inside the container every time.

The release scripts now default to building both `vertexlauncher-linuxx86-64.AppImage` and `vertexlauncher-linuxarm64.AppImage` on Linux x86-64. Override that with `VERTEX_RELEASE_APPIMAGE_ARCHES=<comma-separated arches>` or the legacy single-arch `VERTEX_RELEASE_APPIMAGE_ARCH`.

The release scripts still build raw Linux binaries under `target/<triple>/release/vertexlauncher` as packaging intermediates, but they no longer stage those host-linked binaries into `target/release`. The intended Linux release outputs are the AppImage and Flatpak artifacts.

Staged artifacts are written to `target/release` as:

- `vertexlauncher-windowsx86-64.exe`
- `vertexlauncher-windowsarm64.exe`
- `vertexlauncher-linuxx86-64.AppImage`
- `vertexlauncher-linuxarm64.AppImage`
- `vertexlauncher-macosarm64`

## Cross-Build Notes

- Windows cross-builds use `cargo xwin` with the `clang` backend and scrub host-specific compiler flags.
- Linux x86-64 release builds use a cached CentOS 7 Podman image so release binaries and portable bundles stay at a `glibc 2.17` floor without reinstalling the full GTK/WebKit stack on every run.
- Linux ARM64 release builds now cross-compile on the host against a cached CentOS 7 ARM64 sysroot, then use emulation only for the final AppImage/Flatpak packaging steps.
- The new `scripts/build-linux-portables.sh` helper is the fastest path for the Linux portable matrix because it reuses the local container images, the cached x86-64 Rust toolchain, and the cached ARM64 sysroot across runs.
- macOS ARM64 release builds require a usable Apple SDK. The scripts detect `SDKROOT`, `DEVELOPER_DIR`, `xcrun`, and `~/.local/share/macos-sdk/MacOSX*.sdk`.

## What The Launcher Can Do

- Create, import, edit, delete, and launch Minecraft instances
- Track favorites and usage metadata per instance
- Sign in with Microsoft accounts and switch between cached accounts
- Auto-provision compatible OpenJDK runtimes when needed
- Resolve and install Minecraft assets, libraries, and version metadata
- Install and update Fabric, Forge, NeoForge, and Quilt content
- Browse Modrinth and CurseForge content inside the launcher
- Filter and install mods, resource packs, shaders, and data packs per instance
- Support direct quick-launch into packs, worlds, and servers from the CLI
- Show notifications, logs, settings, skins, legal/privacy views, and themed UI configuration

## Workspace Layout

- `crates/vertexlauncher`: desktop app entrypoint, app shell, CLI dispatch
- `crates/vertex_shell`: the SturdyEngine5 window/runtime shell and live settings appliers
- `crates/launcher_ui`: toolkit-free screen state, background workers and view APIs (home, instance, discover, content browser, skins, game settings), notifications, desktop helpers
- `crates/vertex_theme`: themes and the Oklch palette
- the SE5 screens, widget kit and dialogs live in `crates/vertexlauncher/src/app/se5`
- `crates/installation`: Minecraft setup, dependency resolution, Java/runtime provisioning, launch orchestration
- `crates/auth`: Microsoft/Minecraft auth and account state
- `crates/instances`: persisted instance records and related metadata
- `crates/config`: launcher configuration and serialization
- `crates/modprovider`, `crates/modrinth`, `crates/curseforge`: content provider integration
- `crates/runtime_bootstrap`, `crates/launcher_runtime`: async runtime creation and task execution
- `crates/fontloader`: font lookup and script coverage (which installed fonts can draw emoji or CJK)

## CLI

Quick-launch commands run without opening the full desktop UI.

Launch an instance:

```sh
vertexlauncher --quick-launch-pack --instance <instance-id-or-name> --user <profile-id-or-username>
```

Launch directly into a world:

```sh
vertexlauncher --quick-launch-world --instance <instance-id-or-name> --world <world-folder-name> --user <profile-id-or-username>
```

Launch directly into a server:

```sh
vertexlauncher --quick-launch-server --instance <instance-id-or-name> --server <server-name-or-address> --user <profile-id-or-username>
```

Show quick-launch help:

```sh
vertexlauncher --quick-launch-help
```

List quick-launch targets for an instance:

```sh
vertexlauncher --list-quick-launch-targets --instance <instance-id-or-name>
```

Build launch arguments for scripts or external launchers:

```sh
vertexlauncher --build-quick-launch-args --mode <pack|world|server> --instance <instance-id-or-name> --user <profile-id-or-username> [--world <world-folder-name>] [--server <server-name-or-address>]
```
