#!/bin/bash
set -e

# Define the isolated workspace
WORKSPACE="/tmp/vertex_flatpak_workspace"

echo "--- 1. Creating Clean Workspace in /tmp ---"
rm -rf "$WORKSPACE"
mkdir -p "$WORKSPACE"

# Copy your project to the workspace, strictly ignoring broken cache/build folders
# (Requires rsync to be installed on your system)
rsync -a \
    --exclude='.git' \
    --exclude='.cache' \
    --exclude='.flatpak-builder' \
    --exclude='build-dir' \
    --exclude='target' \
    --exclude='.venv-flatpak' \
    "$(dirname "$0")/../" "$WORKSPACE/"

# Move into the clean workspace
cd "$WORKSPACE"

echo "--- 2. Preparing Python Environment ---"
python3 -m venv .venv-flatpak
source .venv-flatpak/bin/activate
pip install --upgrade pip
pip install aiohttp tomli tomlkit

echo "--- 3. Generating Cargo sources ---"
if [ ! -f "flatpak-cargo-generator.py" ]; then
    curl -L -O https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
deactivate

echo "--- 4. Building Flatpak ---"
# Because we are in /tmp, flatpak-builder uses the native tmpfs filesystem
# and avoids all permission/container conflicts from your mounted drive.
flatpak-builder --user --install --force-clean build-dir org.sturdyfool10.VertexLauncher.yml

echo "--- Done! ---"
echo "Your app has been installed successfully."
echo "Run it anywhere with: flatpak run org.sturdyfool10.VertexLauncher"
