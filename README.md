[![Discord](https://img.shields.io/discord/1480105103414530190?label=Discord%20Members&logo=discord)](https://discord.gg/EJGUFeuGrN)
# Vertex Launcher

Native Minecraft launcher written in Rust.

Vertex Launcher is a multi-crate workspace that provides a desktop launcher, a quick-launch CLI, account handling, instance management, runtime/bootstrap logic, and in-app content discovery for the current Minecraft mod ecosystem.

## Installation
over on the right of our github, we offer release builds for download, they are all come with a .sig file which was signed by one of our maintainers, ensuring the authenticity of the build, to learn more google how to verify a PGP signature, we recommend using Kleopatra.

if you choose to build from source, you will need to have a couple of things installed:
- Rust toolchain (stable)
    - **[NOTE]**: This requires a C/C++ linker to be installed. You can get this from the Visual Studio Community installer by checking `Build Desktop Applications in C++` and installing Visual Studio.
- Cargo (Rust package manager)
- Git (for cloning the repository)

## Linux Build Prerequisites

On Linux, you need native development libraries for `gtk`, `glib`, and `webkit` before `cargo build` will succeed. we use these to handle logins without being able to read your username or password, all we see is a refresh token and a active session token when we log in.

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

If your distro ships `4.0` instead of `4.1`, use:

- `libwebkit2gtk-4.0-dev`
- `libjavascriptcoregtk-4.0-dev`

once you have everything installed, clone the repository, open a shell inside the root of the repo, and trigger a build with:
```sh
cargo build --release
```
this should produce a release build of the application which should appear in the `target/release` directory. this binary is compiled to bundle everything it needs within itself, so you can move it around wherever you like and it should work without any issues.

On Wayland, the desktop app ID is `vertexlauncher`, this will allow you to set an icon and other desktop integration features, a copy of the svg icon we use is included in the repository to help. 


## Current Capabilities

- Native desktop application built with `eframe/egui` and `wgpu`
- Multi-instance library with create, import, delete, launch, and usage tracking flows
- Microsoft account sign-in, cached account management, token refresh, and multi-account switching
- Native quick-launch CLI for packs, worlds, and servers without opening the GUI
- Minecraft runtime/bootstrap setup, OpenJDK provisioning, asset/version resolution, and modloader install flows
- Modrinth and CurseForge content browsing inside the app
- Content-type filtering for mods, resource packs, shaders, and data packs
- In-app content detail/version browsing and per-instance content installation
- Home screen activity feed with world/server discovery, favorites, and server reachability checks
- Console/log surfaces, notifications, settings, skins, legal/privacy views, and theme/font customization
- Download throttling and frame limiter controls for lower-power systems

## Workspace Layout

- `crates/vertexlauncher`: desktop app entrypoint, app orchestration, CLI dispatch
- `crates/launcher_ui`: screens, widgets, notifications, desktop UI helpers
- `crates/installation`: game file resolution, modloader/runtime setup, launch orchestration
- `crates/auth`: Microsoft/Minecraft auth, account cache, secret store integration
- `crates/instances`: persisted instance records and usage/favorite metadata
- `crates/modprovider`, `crates/modrinth`, `crates/curseforge`: content discovery providers
- `crates/config`: persisted launcher configuration and config format handling
- `crates/runtime_bootstrap`, `crates/launcher_runtime`: runtime creation and async task execution
- `crates/textui`, `crates/fontloader`: text/layout/font support used by the UI

## Command Line

These commands run without opening the desktop UI.

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

List available quick-launch targets for one instance:

```sh
vertexlauncher --list-quick-launch-targets --instance <instance-id-or-name>
```

Build an argument string for scripts or external launchers:

```sh
vertexlauncher --build-quick-launch-args --mode <pack|world|server> --instance <instance-id-or-name> --user <profile-id-or-username> [--world <world-folder-name>] [--server <server-name-or-address>]
```

## Project Direction

The project is aimed at a practical native launcher with enough control for power users without turning into a browser shell or a piracy tool.

- Uses valid Minecraft account data and launch credentials
- Prefers native Rust codepaths over heavyweight web stacks
- Keeps launcher concerns separated into reusable crates
- Targets desktop environments where Minecraft itself can run

# What we will never do

Our goal is to provide a high-quality, secure, and reliable launcher for Minecraft. We will never:

- assist in the distribution or sale of cracked or pirated Minecraft content
- tell people how or where to obtain cracked or pirated Minecraft content
- attempt to steal credentials or data from Minecraft accounts
- ban people from using this launcher due to:
  - their beliefs
  - their speech so long as it is lawful in the United States
  - who they voted for
  - who they affiliate with
- try to allow people to continue using this launcher after they have been banned from Minecraft

We care about community trust, and above that as well, I as the owner of this project hate pirates, that is all anyone needs to know.
Free speech is a core value of this project, and we will never censor or ban people for their opinions.
