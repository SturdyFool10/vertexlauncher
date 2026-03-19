#!/usr/bin/env fish

set -g script_dir (path dirname (status filename))
set -g repo_root (path resolve $script_dir/..)

set -g package vertexlauncher
set -g release_dir $repo_root/target/release
set -g linux_glibc_version 2.17
if set -q VERTEX_LINUX_GLIBC_VERSION
    set -g linux_glibc_version $VERTEX_LINUX_GLIBC_VERSION
end
set -g windows_targets x86_64-pc-windows-msvc aarch64-pc-windows-msvc
set -g linux_targets x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
set -g macos_targets aarch64-apple-darwin
set -g build_failures
set -g staged_artifacts \
    vertexlauncher-windowsx86-64.exe \
    vertexlauncher-windowsarm64.exe \
    vertexlauncher-linuxx86-64 \
    vertexlauncher-linuxarm64 \
    vertexlauncher-macosarm64 \
    vertexlauncher-windows-x86-64.exe \
    vertexlauncher-windows-arm64.exe \
    vertexlauncher-linux-arm64 \
    vertexlauncher-macos-aarch64 \
    vertexlauncher-windows-x86_64.exe \
    vertexlauncher-linux-x86_64 \
    vertexlauncher-macos-x86_64

function require_command
    set -l command_name $argv[1]
    set -l install_hint $argv[2]
    if not command -sq $command_name
        echo "Missing $command_name. $install_hint" >&2
        exit 1
    end
end

function artifact_name
    set -l platform $argv[1]
    set -l arch $argv[2]
    set -l ext $argv[3]
    printf "%s/vertexlauncher-%s%s%s\n" $release_dir $platform $arch $ext
end

function copy_artifact
    set -l source_path $argv[1]
    set -l staged_path $argv[2]
    if not test -f $source_path
        echo "Missing built artifact: $source_path" >&2
        return 1
    end
    cp -f $source_path $staged_path
    or return $status
end

function note_failure
    set -g build_failures $build_failures $argv[1]
end

function clear_staged_artifacts
    for artifact in $staged_artifacts
        rm -f $release_dir/$artifact
    end
end

function has_cross_pkg_config
    if set -q PKG_CONFIG
        return 0
    end
    if set -q PKG_CONFIG_ALLOW_CROSS
        if set -q PKG_CONFIG_SYSROOT_DIR; or set -q PKG_CONFIG_LIBDIR; or set -q PKG_CONFIG_PATH
            return 0
        end
    end
    return 1
end

function has_macos_sdk
    if test -n (resolve_macos_sdk)
        return 0
    end
    return 1
end

function resolve_macos_sdk
    if set -q SDKROOT
        test -d $SDKROOT
        and echo $SDKROOT
        and return 0
    end
    if set -q DEVELOPER_DIR
        test -d $DEVELOPER_DIR
        and echo $DEVELOPER_DIR/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
        and return 0
    end
    if command -sq xcrun
        set -l xcrun_sdk (xcrun --sdk macosx --show-sdk-path 2>/dev/null)
        if test $status -eq 0 -a -n "$xcrun_sdk"
            echo $xcrun_sdk
            return 0
        end
    end

    for candidate in $HOME/.local/share/macos-sdk/MacOSX.sdk $HOME/.local/share/macos-sdk/MacOSX*.sdk
        if test -d $candidate
            echo $candidate
            return 0
        end
    end

    return 1
end

function build_windows_target
    set -l target $argv[1]
    set -l arch $argv[2]
    set -l source_path $repo_root/target/$target/release/$package.exe
    set -l staged_path (artifact_name windows $arch .exe)

    echo "Building Windows $arch release binary..."
    env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
        cargo xwin build --release --target $target --cross-compiler clang -p $package
    or return $status

    copy_artifact $source_path $staged_path
    echo "  Staged: $staged_path"
end

function build_linux_target
    set -l target $argv[1]
    set -l arch $argv[2]
    set -l build_target $target
    set -l staged_path (artifact_name linux $arch "")

    echo "Building Linux $arch release binary..."
    if test $target = x86_64-unknown-linux-gnu
        set build_target $target.$linux_glibc_version
    end

    set -l source_path $repo_root/target/$build_target/release/$package

    if test $target = aarch64-unknown-linux-gnu
        if test -x $repo_root/scripts/build-linux-arm64-container.sh
            if not command -sq podman
                echo "Skipping Linux $arch: podman is required for the containerized cross-build helper." >&2
                return 2
            end

            bash $repo_root/scripts/build-linux-arm64-container.sh
            or return $status

            copy_artifact $source_path $staged_path
            echo "  Staged: $staged_path"
            return 0
        end

        if not has_cross_pkg_config
            echo "Skipping Linux $arch: configure pkg-config for cross-compilation first." >&2
            echo "  Required: PKG_CONFIG_ALLOW_CROSS=1 plus either PKG_CONFIG=<wrapper> or PKG_CONFIG_SYSROOT_DIR with PKG_CONFIG_PATH/PKG_CONFIG_LIBDIR." >&2
            return 2
        end
    end

    if test $target = x86_64-unknown-linux-gnu
        env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
            cargo zigbuild --release --target $build_target -p $package
    else
        env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
            cargo zigbuild --release --target $target -p $package
    end
    or return $status

    copy_artifact $source_path $staged_path
    echo "  Staged: $staged_path"
end

function build_macos_target
    set -l target $argv[1]
    set -l arch $argv[2]
    set -l source_path $repo_root/target/$target/release/$package
    set -l staged_path (artifact_name macos $arch "")

    echo "Building macOS $arch release binary..."
    if not has_macos_sdk
        echo "Skipping macOS $arch: no Apple SDK found." >&2
        echo "  Required: SDKROOT=<MacOSX.sdk path>, DEVELOPER_DIR=<Xcode path>, xcrun on PATH, or ~/.local/share/macos-sdk/MacOSX*.sdk." >&2
        return 2
    end

    set -l sdk_root (resolve_macos_sdk)
    env -u CFLAGS -u CXXFLAGS -u LDFLAGS -u CC -u CXX -u AR -u RANLIB -u RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS \
        SDKROOT=$sdk_root cargo zigbuild --release --target $target -p $package
    or return $status

    copy_artifact $source_path $staged_path
    echo "  Staged: $staged_path"
end

cd $repo_root; or exit 1
mkdir -p $release_dir
or exit $status
clear_staged_artifacts

require_command cargo "Install Rust/Cargo first."

if not cargo xwin --version >/dev/null 2>&1
    echo "Missing cargo-xwin. Install it with: cargo install --locked cargo-xwin" >&2
    exit 1
end

if not cargo zigbuild --help >/dev/null 2>&1
    echo "Missing cargo-zigbuild. Install it with: cargo install --locked cargo-zigbuild" >&2
    exit 1
end

for target in $windows_targets
    switch $target
        case x86_64-pc-windows-msvc
            build_windows_target $target x86-64
            or note_failure "Windows x86-64 build failed."
        case aarch64-pc-windows-msvc
            build_windows_target $target arm64
            or note_failure "Windows arm64 build failed."
    end
end

for target in $linux_targets
    switch $target
        case x86_64-unknown-linux-gnu
            build_linux_target $target x86-64
            or note_failure "Linux x86-64 build failed."
        case aarch64-unknown-linux-gnu
            build_linux_target $target arm64
            or note_failure "Linux arm64 build requires a cross pkg-config sysroot/wrapper."
    end
end

for target in $macos_targets
    switch $target
        case aarch64-apple-darwin
            build_macos_target $target arm64
            or note_failure "macOS arm64 build requires an Apple SDK via SDKROOT, DEVELOPER_DIR, or xcrun."
    end
end

echo ""
echo "Artifacts ready:"
echo "  "(artifact_name windows x86-64 .exe)
echo "  "(artifact_name windows arm64 .exe)
echo "  "(artifact_name linux x86-64 "")
echo "  "(artifact_name linux arm64 "")
echo "  "(artifact_name macos arm64 "")

if test (count $build_failures) -gt 0
    echo ""
    echo "Build matrix incomplete:" >&2
    for failure in $build_failures
        echo "  - $failure" >&2
    end
    exit 1
end
