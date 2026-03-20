#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
APP_ID="io.github.SturdyFool10.VertexLauncher"
APP_COMMAND="vertexlauncher"
RUNTIME_ID="org.gnome.Platform"
SDK_ID="org.gnome.Sdk"
RUNTIME_BRANCH="${VERTEX_FLATPAK_RUNTIME_BRANCH:-49}"
BUILD_ROOT="${REPO_ROOT}/flatpak/build-dir"
REPO_ROOT_DIR="${REPO_ROOT}/flatpak/repo"
DESKTOP_FILE="${REPO_ROOT}/flatpak/${APP_ID}.desktop"
METAINFO_FILE="${REPO_ROOT}/flatpak/${APP_ID}.metainfo.xml"
ICON_SOURCE="${REPO_ROOT}/Vertex.svg"
BRANCH="${VERTEX_FLATPAK_BRANCH:-stable}"
TARGET_ARCHES_RAW="${VERTEX_FLATPAK_ARCHES:-}"

can_delegate_arm64_container_build() {
  local arch

  if [[ "${VERTEX_ENABLE_ARM64_EMULATION:-}" != "1" ]]; then
    return 1
  fi
  if [[ "${VERTEX_IN_ARM64_CONTAINER:-}" == "1" ]]; then
    return 1
  fi
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 1
  fi
  if ! command -v podman >/dev/null 2>&1; then
    return 1
  fi
  if [[ ! -x "${REPO_ROOT}/scripts/build-flatpak-arm64-container.sh" ]]; then
    return 1
  fi

  for arch in "$@"; do
    if [[ -z "${arch}" ]]; then
      continue
    fi
    if [[ "${arch}" != "aarch64" ]]; then
      return 1
    fi
  done

  return 0
}

run_arm64_container_build() {
  echo "[flatpak] host cannot export aarch64 directly; delegating to ARM64 container..."
  bash "${REPO_ROOT}/scripts/build-flatpak-arm64-container.sh"
}

require_command() {
  local command_name="$1"
  local install_hint="$2"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Missing ${command_name}. ${install_hint}" >&2
    exit 1
  fi
}

find_source_binary() {
  local requested_arch="$1"
  local candidate
  local -a candidates=()

  case "${requested_arch}" in
    x86_64)
      candidates=(
        "${REPO_ROOT}/target/release/vertexlauncher-linuxx86-64"
        "${REPO_ROOT}/target/x86_64-unknown-linux-gnu/release/vertexlauncher"
        "${REPO_ROOT}/target/release/vertexlauncher"
      )
      ;;
    aarch64)
      candidates=(
        "${REPO_ROOT}/target/release/vertexlauncher-linuxarm64"
        "${REPO_ROOT}/target/aarch64-unknown-linux-gnu/release/vertexlauncher"
      )
      ;;
    *)
      echo "Unsupported Flatpak architecture: ${requested_arch}" >&2
      exit 1
      ;;
  esac

  for candidate in "${candidates[@]}"; do
    if [[ -f "${candidate}" ]]; then
      printf "%s\n" "${candidate}"
      return 0
    fi
  done

  return 1
}

ensure_release_source_binary() {
  local source_binary="$1"

  case "${source_binary}" in
    "${REPO_ROOT}/target/"*/release/"${APP_COMMAND}" \
    | "${REPO_ROOT}/target/release/${APP_COMMAND}" \
    | "${REPO_ROOT}/target/release/${APP_COMMAND}-linux"*)
      return 0
      ;;
  esac

  echo "Flatpak packaging only accepts release binaries from target/*/release or target/release." >&2
  echo "Refusing source path: ${source_binary}" >&2
  exit 1
}

require_source_binary() {
  local requested_arch="$1"
  local source_binary=""

  if ! source_binary="$(find_source_binary "${requested_arch}")"; then
    echo "Missing built Linux binary for Flatpak arch '${requested_arch}'." >&2
    echo "Build the matching Linux release binary first, then rerun this script." >&2
    exit 1
  fi

  ensure_release_source_binary "${source_binary}"
  printf "%s\n" "${source_binary}"
}

ensure_runtime_refs() {
  local requested_arch="$1"

  echo "[flatpak] ensuring ${requested_arch} runtime refs exist..."
  flatpak install \
    --user \
    -y \
    --noninteractive \
    --or-update \
    --arch="${requested_arch}" \
    flathub \
    "${RUNTIME_ID}//${RUNTIME_BRANCH}" \
    "${SDK_ID}//${RUNTIME_BRANCH}" >/dev/null
}

stage_build_tree() {
  local build_dir="$1"
  local source_binary="$2"

  install -Dm755 "${source_binary}" "${build_dir}/files/bin/${APP_COMMAND}"
  install -Dm644 "${DESKTOP_FILE}" "${build_dir}/files/share/applications/${APP_ID}.desktop"
  install -Dm644 "${METAINFO_FILE}" "${build_dir}/files/share/metainfo/${APP_ID}.metainfo.xml"
  install -Dm644 "${ICON_SOURCE}" "${build_dir}/files/share/icons/hicolor/scalable/apps/${APP_ID}.svg"
}

if [[ -n "${TARGET_ARCHES_RAW}" ]]; then
  IFS=',' read -r -a TARGET_ARCHES <<< "${TARGET_ARCHES_RAW}"
  if can_delegate_arm64_container_build "${TARGET_ARCHES[@]}"; then
    run_arm64_container_build
    exit 0
  fi
fi

require_command flatpak "Install Flatpak first."

if [[ ! -f "${DESKTOP_FILE}" ]]; then
  echo "Missing desktop file: ${DESKTOP_FILE}" >&2
  exit 1
fi
if [[ ! -f "${METAINFO_FILE}" ]]; then
  echo "Missing metainfo file: ${METAINFO_FILE}" >&2
  exit 1
fi
if [[ ! -f "${ICON_SOURCE}" ]]; then
  echo "Missing icon file: ${ICON_SOURCE}" >&2
  exit 1
fi

mapfile -t SUPPORTED_ARCHES < <(flatpak --supported-arches)
DEFAULT_ARCH="$(flatpak --default-arch)"

if [[ -z "${TARGET_ARCHES_RAW}" ]]; then
  TARGET_ARCHES_RAW="${DEFAULT_ARCH}"
fi

IFS=',' read -r -a TARGET_ARCHES <<< "${TARGET_ARCHES_RAW}"

arch_is_supported() {
  local candidate="$1"
  local supported_arch
  for supported_arch in "${SUPPORTED_ARCHES[@]}"; do
    if [[ "${supported_arch}" == "${candidate}" ]]; then
      return 0
    fi
  done
  return 1
}

mkdir -p "${REPO_ROOT}/target/release"
declare -a NATIVE_TARGET_ARCHES=()
declare -a EMULATED_TARGET_ARCHES=()

requested_arch=""
for requested_arch in "${TARGET_ARCHES[@]}"; do
  if [[ -z "${requested_arch}" ]]; then
    continue
  fi

  if ! arch_is_supported "${requested_arch}"; then
    if [[ "${requested_arch}" == "aarch64" ]] && can_delegate_arm64_container_build "${requested_arch}"; then
      EMULATED_TARGET_ARCHES+=("${requested_arch}")
      continue
    fi
    echo "Flatpak host does not support architecture '${requested_arch}'." >&2
    echo "Supported arches on this machine: ${SUPPORTED_ARCHES[*]}" >&2
    echo "Run this script on a host that supports '${requested_arch}', or set VERTEX_ENABLE_ARM64_EMULATION=1 to use the ARM64 container helper." >&2
    exit 1
  fi

  NATIVE_TARGET_ARCHES+=("${requested_arch}")
done

echo "[flatpak] ensuring Flathub remote exists..."
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo >/dev/null 2>&1 || true

declare -a BUNDLE_PATHS=()
for requested_arch in "${NATIVE_TARGET_ARCHES[@]}"; do
  if [[ -z "${requested_arch}" ]]; then
    continue
  fi

  source_binary="$(require_source_binary "${requested_arch}")"
  build_dir="${BUILD_ROOT}/${requested_arch}"
  repo_dir="${REPO_ROOT_DIR}/${requested_arch}"
  bundle_path="${REPO_ROOT}/target/release/${APP_ID}-${requested_arch}.flatpak"

  ensure_runtime_refs "${requested_arch}"
  rm -rf "${build_dir}" "${repo_dir}"

  echo "[flatpak] assembling ${requested_arch} build tree from ${source_binary}..."
  flatpak build-init \
    --arch="${requested_arch}" \
    "${build_dir}" \
    "${APP_ID}" \
    "${SDK_ID}" \
    "${RUNTIME_ID}" \
    "${RUNTIME_BRANCH}" >/dev/null

  stage_build_tree "${build_dir}" "${source_binary}"

  echo "[flatpak] finalizing metadata for ${requested_arch}..."
  flatpak build-finish \
    "${build_dir}" \
    --command="${APP_COMMAND}" \
    --share=network \
    --share=ipc \
    --socket=wayland \
    --socket=x11 \
    --socket=pulseaudio \
    --device=dri \
    --filesystem=home \
    --talk-name=org.freedesktop.secrets \
    --env=GDK_BACKEND=wayland,x11 >/dev/null

  echo "[flatpak] exporting repository for ${requested_arch}..."
  flatpak build-export \
    --disable-sandbox \
    --arch="${requested_arch}" \
    "${repo_dir}" \
    "${build_dir}" \
    "${BRANCH}" >/dev/null

  echo "[flatpak] generating repository metadata for ${requested_arch}..."
  flatpak build-update-repo --generate-static-deltas "${repo_dir}" >/dev/null

  echo "[flatpak] bundling portable flatpak for ${requested_arch}..."
  rm -f "${bundle_path}"
  flatpak build-bundle \
    --arch="${requested_arch}" \
    "${repo_dir}" \
    "${bundle_path}" \
    "${APP_ID}" \
    "${BRANCH}" >/dev/null

  BUNDLE_PATHS+=("${bundle_path}")
done

if (( ${#EMULATED_TARGET_ARCHES[@]} > 0 )); then
  run_arm64_container_build
  BUNDLE_PATHS+=("${REPO_ROOT}/target/release/${APP_ID}-aarch64.flatpak")
fi

echo
echo "Flatpak artifacts ready:"
for bundle_path in "${BUNDLE_PATHS[@]}"; do
  echo "  ${bundle_path}"
done
