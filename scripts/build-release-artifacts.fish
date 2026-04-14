#!/usr/bin/env fish

set -g script_dir (path dirname (status filename))
set -g failures

function run_target
    set -l name $argv[1]
    set -l script $argv[2]
    echo ""
    echo "=== $name ==="
    if bash $script
        return 0
    else
        set -g failures $failures $name
    end
end

run_target windows-x86_64  $script_dir/build-windows-x86_64.sh
run_target windows-arm64   $script_dir/build-windows-arm64.sh
run_target linux-x86_64    $script_dir/build-linux-x86_64-container.sh
run_target linux-arm64     $script_dir/build-linux-arm64-container.sh
run_target macos-arm64     $script_dir/build-macos-arm64.sh
run_target flatpak-x86_64  $script_dir/build-flatpak-x86_64-container.sh
run_target flatpak-arm64   $script_dir/build-flatpak-arm64-container.sh
run_target appimage-x86_64 $script_dir/build-appimage-x86_64-container.sh
run_target appimage-arm64  $script_dir/build-appimage-arm64-container.sh

if test (count $failures) -gt 0
    echo ""
    echo "Build matrix incomplete:" >&2
    for f in $failures
        echo "  - $f" >&2
    end
    exit 1
end

echo ""
echo "All targets built."
