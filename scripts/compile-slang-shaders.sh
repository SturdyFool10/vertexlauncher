#!/usr/bin/env bash
# Pre-compile Slang shaders to WGSL on the host before container entry.
# The precompiled/ files are the build.rs fallback when slangc is absent inside containers.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
SHADER_SRC="${REPO_ROOT}/crates/launcher_ui/src/screens/shaders"
PRECOMPILED="${SHADER_SRC}/precompiled"

SHADERS=(
    skin_preview_post_scene
    skin_preview_accumulate
    skin_preview_fxaa
    skin_preview_smaa
    skin_preview_taa
    skin_preview_present
    skin_preview_ssao
)

if ! command -v slangc >/dev/null 2>&1; then
    echo "[shaders] slangc not found; checking pre-compiled WGSL files are present..."
    missing=()
    for name in "${SHADERS[@]}"; do
        [[ -f "${PRECOMPILED}/${name}.wgsl" ]] || missing+=("${name}.wgsl")
    done
    if (( ${#missing[@]} > 0 )); then
        printf '[shaders] Missing pre-compiled fallbacks (install slangc to generate them):\n' >&2
        printf '          %s\n' "${missing[@]}" >&2
        exit 1
    fi
    echo "[shaders] using existing pre-compiled WGSL files."
    exit 0
fi

mkdir -p "${PRECOMPILED}"
changed=0
for name in "${SHADERS[@]}"; do
    src="${SHADER_SRC}/${name}.slang"
    dst="${PRECOMPILED}/${name}.wgsl"

    # Recompile if source is newer than the precompiled output, or output missing.
    if [[ ! -f "${dst}" ]] || [[ "${src}" -nt "${dst}" ]]; then
        slangc "${src}" -target wgsl -o "${dst}"
        echo "[shaders] compiled ${name}.wgsl"
        changed=$((changed + 1))
    fi
done

if (( changed == 0 )); then
    echo "[shaders] all WGSL files up to date."
fi
