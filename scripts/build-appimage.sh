#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
APP_ID="io.github.SturdyFool10.VertexLauncher"
PACKAGE="vertexlauncher"
DESKTOP_FILE="${REPO_ROOT}/flatpak/${APP_ID}.desktop"
ICON_SOURCE="${REPO_ROOT}/Vertex.svg"
APPIMAGE_TOOL_CACHE_ROOT="${VERTEX_APPIMAGE_TOOL_CACHE_ROOT:-${REPO_ROOT}/.cache/appimage-tools}"
APPIMAGE_TOOL_CHANNEL="${VERTEX_APPIMAGE_TOOL_CHANNEL:-continuous}"

should_use_container_packaging() {
  case "${VERTEX_APPIMAGE_USE_CONTAINER:-1}" in
    0|false|FALSE|False|no|NO)
      return 1
      ;;
  esac

  return 0
}

maybe_delegate_container_build() {
  local requested_arch="$1"
  local helper_script=""

  if [[ "${VERTEX_IN_APPIMAGE_CONTAINER:-}" == "1" ]]; then
    return 1
  fi
  if ! should_use_container_packaging; then
    return 1
  fi
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 1
  fi
  if ! command -v podman >/dev/null 2>&1; then
    return 1
  fi

  case "${requested_arch}" in
    x86_64)
      helper_script="${REPO_ROOT}/scripts/build-appimage-x86_64-container.sh"
      ;;
    aarch64)
      helper_script="${REPO_ROOT}/scripts/build-appimage-arm64-container.sh"
      ;;
    *)
      return 1
      ;;
  esac

  if [[ ! -x "${helper_script}" ]]; then
    return 1
  fi

  echo "[appimage] packaging ${requested_arch} AppImage inside Debian container..."
  exec bash "${helper_script}"
}

normalize_arch() {
  local value="${1:-}"
  case "${value}" in
    x86_64|amd64|x86-64)
      printf "x86_64\n"
      ;;
    aarch64|arm64)
      printf "aarch64\n"
      ;;
    *)
      return 1
      ;;
  esac
}

stage_arch_name() {
  local value="$1"
  case "${value}" in
    x86_64)
      printf "x86-64\n"
      ;;
    aarch64)
      printf "arm64\n"
      ;;
    *)
      return 1
      ;;
  esac
}

ensure_tool_executable() {
  local tool_path="$1"
  if [[ -x "${tool_path}" ]]; then
    return 0
  fi

  chmod +x "${tool_path}" 2>/dev/null || true
  [[ -x "${tool_path}" ]]
}

resolve_tool() {
  local env_name="$1"
  local command_name="$2"
  local install_hint="$3"
  local requested_arch="$4"
  local configured_path="${!env_name:-}"

  if [[ -n "${configured_path}" ]]; then
    if [[ ! -f "${configured_path}" ]]; then
      echo "Configured ${env_name} path does not exist: ${configured_path}" >&2
      exit 1
    fi
    if ! ensure_tool_executable "${configured_path}"; then
      echo "Configured ${env_name} path is not executable: ${configured_path}" >&2
      exit 1
    fi
    printf "%s\n" "${configured_path}"
    return 0
  fi

  if command -v "${command_name}" >/dev/null 2>&1; then
    command -v "${command_name}"
    return 0
  fi

  local candidate
  for candidate in \
    "${REPO_ROOT}/tools/${command_name}" \
    "${REPO_ROOT}/tools/${command_name}.AppImage" \
    "${REPO_ROOT}/tools/${command_name}"-*".AppImage" \
    "${REPO_ROOT}/${command_name}.AppImage" \
    "${REPO_ROOT}/${command_name}"-*".AppImage"
  do
    if [[ -f "${candidate}" ]] && ensure_tool_executable "${candidate}"; then
      printf "%s\n" "${candidate}"
      return 0
    fi
  done

  if download_candidate="$(download_tool_if_missing "${command_name}" "${requested_arch}")"; then
    printf "%s\n" "${download_candidate}"
    return 0
  fi

  echo "Missing ${command_name}. ${install_hint}" >&2
  echo "You can also set ${env_name} to a downloaded tool path." >&2
  exit 1
}

download_with_available_client() {
  local url="$1"
  local destination="$2"

  if command -v curl >/dev/null 2>&1; then
    curl --fail --location --retry 3 --output "${destination}" "${url}"
    return 0
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -O "${destination}" "${url}"
    return 0
  fi

  echo "Missing curl/wget. Install one of them or set VERTEX_LINUXDEPLOY / VERTEX_APPIMAGETOOL explicitly." >&2
  exit 1
}

default_tool_download_url() {
  local command_name="$1"
  local requested_arch="$2"

  case "${command_name}" in
    linuxdeploy)
      printf "https://github.com/linuxdeploy/linuxdeploy/releases/download/%s/linuxdeploy-%s.AppImage\n" "${APPIMAGE_TOOL_CHANNEL}" "${requested_arch}"
      ;;
    appimagetool)
      printf "https://github.com/AppImage/appimagetool/releases/download/%s/appimagetool-%s.AppImage\n" "${APPIMAGE_TOOL_CHANNEL}" "${requested_arch}"
      ;;
    *)
      return 1
      ;;
  esac
}

download_tool_if_missing() {
  local command_name="$1"
  local requested_arch="$2"
  local configured_url=""
  local download_url=""
  local cache_dir="${APPIMAGE_TOOL_CACHE_ROOT}/${requested_arch}"
  local destination=""
  local tmp_path=""

  case "${command_name}" in
    linuxdeploy)
      configured_url="${VERTEX_LINUXDEPLOY_URL:-}"
      ;;
    appimagetool)
      configured_url="${VERTEX_APPIMAGETOOL_URL:-}"
      ;;
    *)
      return 1
      ;;
  esac

  download_url="${configured_url:-$(default_tool_download_url "${command_name}" "${requested_arch}")}"
  mkdir -p "${cache_dir}"
  destination="${cache_dir}/${command_name}-${requested_arch}.AppImage"

  if [[ -f "${destination}" ]] && ensure_tool_executable "${destination}"; then
    printf "%s\n" "${destination}"
    return 0
  fi

  echo "[appimage] downloading ${command_name} for ${requested_arch}..." >&2
  tmp_path="${destination}.tmp.$$"
  rm -f "${tmp_path}"
  download_with_available_client "${download_url}" "${tmp_path}" >&2
  chmod +x "${tmp_path}"
  mv -f "${tmp_path}" "${destination}"
  printf "%s\n" "${destination}"
}

resolve_optional_tool() {
  local env_name="$1"
  shift
  local configured_path="${!env_name:-}"
  local candidate

  if [[ -n "${configured_path}" ]]; then
    if [[ -f "${configured_path}" ]] && ensure_tool_executable "${configured_path}"; then
      printf "%s\n" "${configured_path}"
    fi
    return 0
  fi

  for candidate in "$@"; do
    if command -v "${candidate}" >/dev/null 2>&1; then
      command -v "${candidate}"
      return 0
    fi
  done

  for candidate in \
    "${REPO_ROOT}/tools/linuxdeploy-plugin-gtk" \
    "${REPO_ROOT}/tools/linuxdeploy-plugin-gtk.sh" \
    "${REPO_ROOT}/tools/linuxdeploy-plugin-gtk"*.AppImage \
    "${REPO_ROOT}/linuxdeploy-plugin-gtk" \
    "${REPO_ROOT}/linuxdeploy-plugin-gtk.sh" \
    "${REPO_ROOT}/linuxdeploy-plugin-gtk"*.AppImage
  do
    if [[ -f "${candidate}" ]] && ensure_tool_executable "${candidate}"; then
      printf "%s\n" "${candidate}"
      return 0
    fi
  done
}

run_tool() {
  local tool_path="$1"
  shift
  local tool_runner="${VERTEX_APPIMAGE_TOOL_RUNNER:-}"

  if [[ "${tool_path}" == *.AppImage ]]; then
    if [[ -n "${tool_runner}" ]]; then
      APPIMAGE_EXTRACT_AND_RUN=1 "${tool_runner}" "${tool_path}" "$@"
    else
      APPIMAGE_EXTRACT_AND_RUN=1 "${tool_path}" "$@"
    fi
  else
    if [[ -n "${tool_runner}" ]]; then
      "${tool_runner}" "${tool_path}" "$@"
    else
      "${tool_path}" "$@"
    fi
  fi
}

ensure_release_source_binary() {
  local source_binary="$1"

  case "${source_binary}" in
    "${REPO_ROOT}/target/"*/release/"${PACKAGE}" \
    | "${REPO_ROOT}/target/release/${PACKAGE}" \
    | "${REPO_ROOT}/target/release/${PACKAGE}-linux"*)
      return 0
      ;;
  esac

  echo "AppImage packaging only accepts release binaries from target/*/release or target/release." >&2
  echo "Refusing source path: ${source_binary}" >&2
  exit 1
}

is_blacklisted_bundle_library() {
  local library_name="$1"

  case "${library_name}" in
    ld-linux-*.so*|libc.so.*|libdl.so.*|libm.so.*|libpthread.so.*|librt.so.*|libresolv.so.*|libutil.so.*|libnss_*.so.*)
      return 0
      ;;
  esac

  return 1
}

bundle_foreign_arch_libraries() {
  local source_binary="$1"
  local appdir="$2"
  local tool_runner="${VERTEX_APPIMAGE_TOOL_RUNNER:-}"
  local interpreter=""
  local library_path=""
  local library_name=""
  local bundled_count=0
  local -a resolved_library_paths=()

  if [[ -z "${tool_runner}" ]]; then
    echo "Missing VERTEX_APPIMAGE_TOOL_RUNNER for foreign-arch AppImage packaging." >&2
    exit 1
  fi
  if ! command -v patchelf >/dev/null 2>&1; then
    echo "Missing patchelf. Install patchelf before building the AppImage." >&2
    exit 1
  fi

  interpreter="$(patchelf --print-interpreter "${source_binary}")"
  if [[ -z "${interpreter}" ]] || [[ ! -e "${interpreter}" ]]; then
    echo "Unable to locate ELF interpreter for ${source_binary}." >&2
    exit 1
  fi

  mapfile -t resolved_library_paths < <(
    "${tool_runner}" "${interpreter}" --list "${source_binary}" | \
      awk '{
        for (i = 1; i <= NF; ++i) {
          if ($i ~ /^\//) {
            print $i
            break
          }
        }
      }' | sort -u
  )

  mkdir -p "${appdir}/usr/lib"
  for library_path in "${resolved_library_paths[@]}"; do
    if [[ -z "${library_path}" ]] || [[ ! -f "${library_path}" ]]; then
      continue
    fi

    library_name="$(basename -- "${library_path}")"
    if is_blacklisted_bundle_library "${library_name}"; then
      continue
    fi

    install -Dm755 "${library_path}" "${appdir}/usr/lib/${library_name}"
    if file -b "${appdir}/usr/lib/${library_name}" | grep -q 'ELF'; then
      patchelf --set-rpath '$ORIGIN' "${appdir}/usr/lib/${library_name}"
    fi
    bundled_count="$((bundled_count + 1))"
  done

  if (( bundled_count == 0 )); then
    echo "Failed to bundle any foreign-arch shared libraries for ${source_binary}." >&2
    exit 1
  fi

  patchelf --set-rpath '$ORIGIN/../lib' "${appdir}/usr/bin/${PACKAGE}"
}

machine_arch="$(uname -m)"
host_arch="$(normalize_arch "${machine_arch}")" || {
  echo "Unsupported AppImage host architecture: ${machine_arch}" >&2
  exit 1
}
requested_arch="${host_arch}"
if [[ -n "${VERTEX_APPIMAGE_ARCH:-}" ]]; then
  requested_arch="$(normalize_arch "${VERTEX_APPIMAGE_ARCH}")" || {
    echo "Unsupported requested AppImage architecture: ${VERTEX_APPIMAGE_ARCH}" >&2
    exit 1
  }
fi

maybe_delegate_container_build "${requested_arch}" || true

if [[ "${requested_arch}" != "${host_arch}" ]]; then
  if [[ -z "${VERTEX_APPIMAGE_TOOL_RUNNER:-}" ]]; then
    echo "Skipping AppImage: requested arch ${requested_arch} does not match host arch ${host_arch}." >&2
    exit 2
  fi
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Skipping AppImage: Linux is required." >&2
  exit 2
fi

appimagetool_tool="$(resolve_tool VERTEX_APPIMAGETOOL appimagetool "Install appimagetool first." "${requested_arch}")"
manual_library_bundle=0
linuxdeploy_tool=""
plugin_args=()

if [[ "${requested_arch}" != "${host_arch}" ]] && [[ -n "${VERTEX_APPIMAGE_TOOL_RUNNER:-}" ]]; then
  manual_library_bundle=1
else
  linuxdeploy_tool="$(resolve_tool VERTEX_LINUXDEPLOY linuxdeploy "Install linuxdeploy first." "${requested_arch}")"
  gtk_plugin_tool="$(resolve_optional_tool VERTEX_LINUXDEPLOY_GTK_PLUGIN linuxdeploy-plugin-gtk linuxdeploy-plugin-gtk.sh)"

  if [[ -n "${gtk_plugin_tool:-}" ]]; then
    plugin_shim_dir="${REPO_ROOT}/target/appimage/tool-shims"
    mkdir -p "${plugin_shim_dir}"
    cat > "${plugin_shim_dir}/linuxdeploy-plugin-gtk" <<EOF
#!/usr/bin/env bash
set -euo pipefail
tool_path=$(printf '%q' "${gtk_plugin_tool}")
tool_runner=$(printf '%q' "${VERTEX_APPIMAGE_TOOL_RUNNER:-}")
if [[ -n "\${tool_runner}" ]]; then
  if [[ "\${tool_path}" == *.AppImage ]]; then
    APPIMAGE_EXTRACT_AND_RUN=1 "\${tool_runner}" "\${tool_path}" "\$@"
  else
    "\${tool_runner}" "\${tool_path}" "\$@"
  fi
else
  if [[ "\${tool_path}" == *.AppImage ]]; then
    APPIMAGE_EXTRACT_AND_RUN=1 "\${tool_path}" "\$@"
  else
    "\${tool_path}" "\$@"
  fi
fi
EOF
    chmod +x "${plugin_shim_dir}/linuxdeploy-plugin-gtk"
    PATH="${plugin_shim_dir}:${PATH}"
    plugin_args+=(--plugin gtk)
  else
    echo "AppImage packaging will continue without linuxdeploy GTK plugin; runtime theming/assets may be incomplete." >&2
  fi
fi

rust_target="${VERTEX_APPIMAGE_TARGET:-}"
if [[ -z "${rust_target}" ]]; then
  case "${requested_arch}" in
    x86_64)
      rust_target="x86_64-unknown-linux-gnu"
      ;;
    aarch64)
      rust_target="aarch64-unknown-linux-gnu"
      ;;
  esac
fi

source_binary="${VERTEX_APPIMAGE_SOURCE:-${REPO_ROOT}/target/${rust_target}/release/${PACKAGE}}"
if [[ ! -f "${source_binary}" ]]; then
  echo "Missing built Linux binary: ${source_binary}" >&2
  exit 1
fi
ensure_release_source_binary "${source_binary}"

if [[ ! -f "${DESKTOP_FILE}" ]]; then
  echo "Missing desktop file: ${DESKTOP_FILE}" >&2
  exit 1
fi

if [[ ! -f "${ICON_SOURCE}" ]]; then
  echo "Missing icon source: ${ICON_SOURCE}" >&2
  exit 1
fi

stage_arch="$(stage_arch_name "${requested_arch}")"
appdir_root="${REPO_ROOT}/target/appimage/${requested_arch}"
appdir="${appdir_root}/AppDir"
icon_path="${appdir_root}/${APP_ID}.svg"
output_path="${REPO_ROOT}/target/release/vertexlauncher-linux${stage_arch}.AppImage"

rm -rf "${appdir_root}"
mkdir -p "${appdir}/usr/bin" "${appdir}/usr/share/applications" "${appdir}/usr/share/icons/hicolor/scalable/apps"

install -Dm755 "${source_binary}" "${appdir}/usr/bin/${PACKAGE}"
install -Dm644 "${DESKTOP_FILE}" "${appdir}/usr/share/applications/${APP_ID}.desktop"
install -Dm644 "${DESKTOP_FILE}" "${appdir}/${APP_ID}.desktop"
install -Dm644 "${ICON_SOURCE}" "${appdir}/usr/share/icons/hicolor/scalable/apps/${APP_ID}.svg"
install -Dm644 "${ICON_SOURCE}" "${appdir}/${APP_ID}.svg"
install -Dm644 "${ICON_SOURCE}" "${icon_path}"
install -Dm644 "${ICON_SOURCE}" "${appdir}/.DirIcon"

if (( manual_library_bundle )); then
  echo "[appimage] bundling ${requested_arch} shared libraries without linuxdeploy auto-detection..."
  bundle_foreign_arch_libraries "${source_binary}" "${appdir}"
else
  run_tool "${linuxdeploy_tool}" \
    --appdir "${appdir}" \
    --desktop-file "${appdir}/${APP_ID}.desktop" \
    --icon-file "${icon_path}" \
    --executable "${appdir}/usr/bin/${PACKAGE}" \
    "${plugin_args[@]}"
fi

install -Dm755 "${SCRIPT_DIR}/resources/AppRun" "${appdir}/AppRun"

ARCH="${requested_arch}" run_tool "${appimagetool_tool}" "${appdir}" "${output_path}"

echo "AppImage artifact ready:"
echo "  ${output_path}"
