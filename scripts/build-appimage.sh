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

  if [[ ! -f "${helper_script}" ]]; then
    return 1
  fi

  echo "[appimage] packaging ${requested_arch} AppImage inside container..."
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

find_first_existing_path() {
  local candidate=""

  for candidate in "$@"; do
    if [[ -e "${candidate}" ]]; then
      printf "%s\n" "${candidate}"
      return 0
    fi
  done

  return 1
}

copy_tree_contents_if_present() {
  local source_path="$1"
  local dest_path="$2"

  if [[ ! -d "${source_path}" ]]; then
    return 1
  fi

  mkdir -p "${dest_path}"
  cp -a "${source_path}/." "${dest_path}/"
}

set_rpath_if_elf() {
  local target_path="$1"
  local rpath_value="$2"

  if ! command -v patchelf >/dev/null 2>&1; then
    return 0
  fi
  if [[ ! -f "${target_path}" ]]; then
    return 0
  fi
  if ! file -b "${target_path}" | grep -q "ELF"; then
    return 0
  fi

  patchelf --set-rpath "${rpath_value}" "${target_path}"
}

patch_binary_path_literal() {
  local binary_path="$1"
  local old_value="$2"
  local new_value="$3"

  local pybin="${PYTHON:-}"
  if [[ -z "${pybin}" ]]; then
    if command -v python3 >/dev/null 2>&1; then
      pybin="$(command -v python3)"
    elif command -v python >/dev/null 2>&1; then
      pybin="$(command -v python)"
    else
      echo "Missing python3/python for patch_binary_path_literal" >&2
      return 1
    fi
  fi
  "${pybin}" - "$binary_path" "$old_value" "$new_value" <<'PY'
import sys

binary_path = sys.argv[1]
old = sys.argv[2].encode("utf-8")
new = sys.argv[3].encode("utf-8")

if len(new) > len(old):
    raise SystemExit(
        "replacement path '{}' is longer than '{}'".format(sys.argv[3], sys.argv[2])
    )

with open(binary_path, "rb") as handle:
    payload = handle.read()
index = payload.find(old)
if index == -1:
    raise SystemExit(0)

replacement = new + (b"\x00" * (len(old) - len(new)))
payload = payload.replace(old, replacement)
with open(binary_path, "wb") as handle:
    handle.write(payload)
PY
}

bundle_runtime_support_assets() {
  local appdir="$1"
  local webkit_helper_dir=""
  local webkit_bundle_dir=""
  local gdk_pixbuf_dir=""
  local gtk_modules_dir=""
  local schema_dir="/usr/share/glib-2.0/schemas"
  local default_theme_dir="/usr/share/themes/Default/gtk-3.0"
  local helper_file=""

  webkit_helper_dir="$(find_first_existing_path \
    /usr/libexec/webkit2gtk-4.1 \
    /usr/lib/x86_64-linux-gnu/webkit2gtk-4.1 \
    /usr/lib/aarch64-linux-gnu/webkit2gtk-4.1 || true)"
  webkit_bundle_dir="$(find_first_existing_path \
    /usr/lib64/webkit2gtk-4.1 \
    /usr/lib/x86_64-linux-gnu/webkit2gtk-4.1 \
    /usr/lib/aarch64-linux-gnu/webkit2gtk-4.1 || true)"
  gdk_pixbuf_dir="$(find_first_existing_path \
    /usr/lib64/gdk-pixbuf-2.0/2.10.0 \
    /usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0 \
    /usr/lib/aarch64-linux-gnu/gdk-pixbuf-2.0/2.10.0 || true)"
  gtk_modules_dir="$(find_first_existing_path \
    /usr/lib64/gtk-3.0/3.0.0 \
    /usr/lib/x86_64-linux-gnu/gtk-3.0/3.0.0 \
    /usr/lib/aarch64-linux-gnu/gtk-3.0/3.0.0 || true)"

  if [[ -n "${webkit_helper_dir}" ]]; then
    copy_tree_contents_if_present "${webkit_helper_dir}" "${appdir}/usr/libexec/webkit2gtk-4.1"
  fi
  if [[ -n "${webkit_bundle_dir}" ]]; then
    copy_tree_contents_if_present "${webkit_bundle_dir}" "${appdir}/usr/lib64/webkit2gtk-4.1"
  fi
  if [[ -n "${gdk_pixbuf_dir}" ]]; then
    copy_tree_contents_if_present "${gdk_pixbuf_dir}" "${appdir}/usr/lib64/gdk-pixbuf-2.0/2.10.0"
  fi
  if [[ -n "${gtk_modules_dir}" ]]; then
    copy_tree_contents_if_present "${gtk_modules_dir}" "${appdir}/usr/lib64/gtk-3.0/3.0.0"
  fi
  copy_tree_contents_if_present "${schema_dir}" "${appdir}/usr/share/glib-2.0/schemas" || true
  copy_tree_contents_if_present "${default_theme_dir}" "${appdir}/usr/share/themes/Default/gtk-3.0" || true

  if command -v glib-compile-schemas >/dev/null 2>&1 && [[ -d "${appdir}/usr/share/glib-2.0/schemas" ]]; then
    glib-compile-schemas "${appdir}/usr/share/glib-2.0/schemas"
  fi

  for helper_file in \
    "${appdir}/usr/libexec/webkit2gtk-4.1/WebKitNetworkProcess" \
    "${appdir}/usr/libexec/webkit2gtk-4.1/WebKitPluginProcess" \
    "${appdir}/usr/libexec/webkit2gtk-4.1/WebKitWebProcess" \
    "${appdir}/usr/libexec/webkit2gtk-4.1/jsc" \
    "${appdir}/usr/lib64/webkit2gtk-4.1/injected-bundle/libwebkit2gtkinjectedbundle.so"
  do
    set_rpath_if_elf "${helper_file}" '$ORIGIN/../../lib:$ORIGIN/../../lib64/webkit2gtk-4.1'
  done

  if [[ -d "${appdir}/usr/lib64/gdk-pixbuf-2.0/2.10.0/loaders" ]]; then
    while IFS= read -r -d "" helper_file; do
      set_rpath_if_elf "${helper_file}" '$ORIGIN/../../../../lib'
    done < <(find "${appdir}/usr/lib64/gdk-pixbuf-2.0/2.10.0/loaders" -type f -print0)
  fi

  if [[ -d "${appdir}/usr/lib64/gtk-3.0/3.0.0/immodules" ]]; then
    while IFS= read -r -d "" helper_file; do
      set_rpath_if_elf "${helper_file}" '$ORIGIN/../../../../lib'
    done < <(find "${appdir}/usr/lib64/gtk-3.0/3.0.0/immodules" -type f -print0)
  fi

  local arch_lib_webkit_dir=""
  if [[ -d "${appdir}/usr/libexec/webkit2gtk-4.1" ]]; then
    arch_lib_webkit_dir="${appdir}/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1"
    mkdir -p "${arch_lib_webkit_dir}"
    cp -a "${appdir}/usr/libexec/webkit2gtk-4.1/." "${arch_lib_webkit_dir}/"
    mkdir -p "${appdir}/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1"
    cp -a "${appdir}/usr/libexec/webkit2gtk-4.1/." "${appdir}/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1/"
  fi

  if [[ -d "${appdir}/usr/lib64/webkit2gtk-4.1/injected-bundle" ]]; then
    mkdir -p "${appdir}/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/injected-bundle"
    cp -a "${appdir}/usr/lib64/webkit2gtk-4.1/injected-bundle/." "${appdir}/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/injected-bundle/"
    mkdir -p "${appdir}/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1/injected-bundle"
    cp -a "${appdir}/usr/lib64/webkit2gtk-4.1/injected-bundle/." "${appdir}/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1/injected-bundle/"
  fi

  local patch_target=""
  for patch_target in     "${appdir}/usr/bin/vertexlauncher"     "${appdir}/usr/lib/libwebkit2gtk-4.1.so.0"     "${appdir}/usr/lib/libjavascriptcoregtk-4.1.so.0"     "${appdir}/usr/lib/x86_64-linux-gnu/libwebkit2gtk-4.1.so.0"     "${appdir}/usr/lib/x86_64-linux-gnu/libjavascriptcoregtk-4.1.so.0"     "${appdir}/usr/lib/aarch64-linux-gnu/libwebkit2gtk-4.1.so.0"     "${appdir}/usr/lib/aarch64-linux-gnu/libjavascriptcoregtk-4.1.so.0"
  do
    [[ -f "${patch_target}" ]] || continue
    patch_binary_path_literal "${patch_target}" "/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1" "/tmp/webkit2gtk-4.1"
    patch_binary_path_literal "${patch_target}" "/usr/lib/aarch64-linux-gnu/webkit2gtk-4.1" "/tmp/webkit2gtk-4.1"
  done

  #-----------------------------------------------------------------------
  # Copy GLib GIO modules into the bundle so that networking functions
  # (including TLS via glib-networking) work correctly.  Without the
  # `libgio` modules, WebKitGTK will display an error such as
  # "TLS/SSL support not available; install glib-networking" when
  # attempting to load HTTPS content.  We search for the system's
  # `gio/modules` directory across several common locations and copy
  # everything into our AppDir.  Afterwards we run `gio-querymodules`
  # to generate the module cache; this ensures GLib picks up the
  # bundled modules at runtime.
  # Discover GIO module directories for glib-networking (TLS support).  Copy
  # them into both lib and lib64 destinations so that GLib can find
  # modules regardless of architecture-specific directory.  We prefer the
  # first existing directory but may fall back to others.
  local gio_modules_dir=""
  gio_modules_dir="$(find_first_existing_path \
    /usr/lib/gio/modules \
    /usr/lib64/gio/modules \
    /usr/lib/x86_64-linux-gnu/gio/modules \
    /usr/lib/aarch64-linux-gnu/gio/modules || true)"
  if [[ -n "${gio_modules_dir}" ]]; then
    # Copy into /usr/lib/gio/modules
    copy_tree_contents_if_present "${gio_modules_dir}" "${appdir}/usr/lib/gio/modules"
    # Also copy into /usr/lib64/gio/modules for completeness on multiarch systems
    copy_tree_contents_if_present "${gio_modules_dir}" "${appdir}/usr/lib64/gio/modules"
  fi
  # Generate the module cache for each destination; this ensures GLib uses
  # the bundled modules instead of system modules.  Only run the
  # querymodules tool if it exists.
  if command -v gio-querymodules >/dev/null 2>&1; then
    if [[ -d "${appdir}/usr/lib/gio/modules" ]]; then
      gio-querymodules "${appdir}/usr/lib/gio/modules"
    fi
    if [[ -d "${appdir}/usr/lib64/gio/modules" ]]; then
      gio-querymodules "${appdir}/usr/lib64/gio/modules"
    fi
  fi

  # Copy WebKitGTK runtime libraries into the bundle when using the
  # Freedesktop runtime.  The FreeDesktop base does not ship WebKitGTK,
  # so our Flatpak will fail to start unless we bundle the core
  # libwebkit2gtk library (and its JavaScriptCore companion).  Search for
  # the appropriate shared libraries on the build system and install
  # them into `usr/lib` within the AppDir so that the wrapper can find
  # them via LD_LIBRARY_PATH.  Different distributions name the files
  # differently, so probe several common patterns.
  local webkit_lib=""
  for candidate in \
    /usr/lib64/libwebkit2gtk-4.0.so.37 \
    /usr/lib/libwebkit2gtk-4.0.so.37 \
    /usr/lib64/libwebkit2gtk-4.0.so \
    /usr/lib/libwebkit2gtk-4.0.so; do
    if [[ -f "${candidate}" ]]; then
      webkit_lib="${candidate}"
      break
    fi
  done
  if [[ -n "${webkit_lib}" ]]; then
    install -Dm755 "${webkit_lib}" "${appdir}/usr/lib/$(basename "${webkit_lib}")"
  fi

  # Also bundle JavaScriptCore GTK if present; this is often a direct
  # dependency of WebKitGTK.  Without it the launcher may still fail
  # to locate its JSC dependency.
  local jsc_lib=""
  for candidate in \
    /usr/lib64/libjavascriptcoregtk-4.0.so.18 \
    /usr/lib/libjavascriptcoregtk-4.0.so.18 \
    /usr/lib64/libjavascriptcoregtk-4.0.so \
    /usr/lib/libjavascriptcoregtk-4.0.so; do
    if [[ -f "${candidate}" ]]; then
      jsc_lib="${candidate}"
      break
    fi
  done
  if [[ -n "${jsc_lib}" ]]; then
    install -Dm755 "${jsc_lib}" "${appdir}/usr/lib/$(basename "${jsc_lib}")"
  fi

  if [[ -f "${appdir}/usr/lib/libwebkit2gtk-4.0.so.37" ]]; then
    patch_binary_path_literal "${appdir}/usr/lib/libwebkit2gtk-4.0.so.37" \
      "/usr/libexec/webkit2gtk-4.1" \
      "libexec/webkit2gtk-4.0"
    patch_binary_path_literal "${appdir}/usr/lib/libwebkit2gtk-4.0.so.37" \
      "/usr/lib64/webkit2gtk-4.1/injected-bundle/" \
      "lib64/webkit2gtk-4.0/injected-bundle/"
  fi
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

prepare_only=0
case "${VERTEX_APPIMAGE_PREPARE_ONLY:-}" in
  1|true|TRUE|True|yes|YES)
    prepare_only=1
    ;;
esac

appimagetool_tool=""
if (( ! prepare_only )); then
  appimagetool_tool="$(resolve_tool VERTEX_APPIMAGETOOL appimagetool "Install appimagetool first." "${requested_arch}")"
fi
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
tmp_output_path="${output_path}.tmp.$$"

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
  linuxdeploy_args=(
    --appdir "${appdir}"
    --desktop-file "${appdir}/${APP_ID}.desktop"
    --icon-file "${icon_path}"
    --executable "${appdir}/usr/bin/${PACKAGE}"
  )
  # Only append plugin arguments when the array is defined and non-empty.  When
  # using `set -u`, referencing an unset array variable will cause an
  # unbound-variable error.  Guard the check so that we skip adding the
  # plugin arguments entirely if the `plugin_args` array was never defined.
  if [[ -n "${plugin_args+x}" ]] && (( ${#plugin_args[@]} > 0 )); then
    linuxdeploy_args+=("${plugin_args[@]}")
  fi
  run_tool "${linuxdeploy_tool}" "${linuxdeploy_args[@]}"
fi

bundle_runtime_support_assets "${appdir}"

install -Dm755 "${SCRIPT_DIR}/resources/AppRun" "${appdir}/AppRun"

if (( prepare_only )); then
  echo "AppDir prepared:"
  echo "  ${appdir}"
  exit 0
fi

rm -f "${tmp_output_path}"
ARCH="${requested_arch}" run_tool "${appimagetool_tool}" "${appdir}" "${tmp_output_path}"
mv -f "${tmp_output_path}" "${output_path}"

echo "AppImage artifact ready:"
echo "  ${output_path}"
