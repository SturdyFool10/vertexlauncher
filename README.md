Come to our Discord: [![Discord](https://img.shields.io/discord/1480105103414530190?label=Discord%20Members&logo=discord)](https://discord.gg/EJGUFeuGrN)

# Vertex Launcher

Native Minecraft launcher written in Rust.

Vertex Launcher is a multi-crate desktop launcher with:

- native desktop UI built on `eframe`/`egui`
- Microsoft and Minecraft account sign-in
- multi-instance management
- runtime/bootstrap setup for Minecraft and Java
- in-app Modrinth and CurseForge browsing
- quick-launch CLI flows for packs, worlds, and servers

## Building

If you build from source, install:

- Rust toolchain
- Cargo
- Git
- a working C/C++ toolchain

Windows release artifacts must use the MSVC targets. `windows-gnu` is not part of the supported release matrix.

## Native Linux Prerequisites

On Linux, native launcher builds require GTK, GLib, Soup, and WebKit development packages.

For Debian/Ubuntu:

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  pkg-config \
  libglib2.0-dev \
  libgtk-3-dev \
  libgdk-pixbuf-2.0-dev \
  libpango1.0-dev \
  libatk1.0-dev \
  libcairo2-dev \
  libsoup-3.0-dev \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev
```

If your distro only ships the `4.0` WebKit packages, use:

- `libwebkit2gtk-4.0-dev`
- `libjavascriptcoregtk-4.0-dev`

Basic native builds:

```sh
cargo build --release
```

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

Staged artifacts are written to `target/release` as:

- `vertexlauncher-windowsx86-64.exe`
- `vertexlauncher-windowsarm64.exe`
- `vertexlauncher-linuxx86-64`
- `vertexlauncher-linuxarm64`
- `vertexlauncher-macosarm64`

## Cross-Build Notes

- Windows cross-builds use `cargo xwin` with the `clang` backend and scrub host-specific compiler flags.
- Linux ARM64 release builds use a cross sysroot path. The current helper script can assemble that sysroot for release builds.
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
- `crates/launcher_ui`: screens, widgets, notifications, desktop UI helpers
- `crates/installation`: Minecraft setup, dependency resolution, Java/runtime provisioning, launch orchestration
- `crates/auth`: Microsoft/Minecraft auth and account state
- `crates/instances`: persisted instance records and related metadata
- `crates/config`: launcher configuration and serialization
- `crates/modprovider`, `crates/modrinth`, `crates/curseforge`: content provider integration
- `crates/runtime_bootstrap`, `crates/launcher_runtime`: async runtime creation and task execution
- `crates/textui`, `crates/fontloader`: text, layout, and font support

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
