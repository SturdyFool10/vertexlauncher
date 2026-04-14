#!/usr/bin/env bash

require_command() {
  local command_name="$1"
  local install_hint="$2"

  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Missing ${command_name}. ${install_hint}" >&2
    exit 1
  fi
}

normalize_arch() {
  local value="${1:-}"

  case "${value}" in
    x86_64|amd64|x86-64)
      printf 'x86_64\n'
      ;;
    aarch64|arm64)
      printf 'aarch64\n'
      ;;
    *)
      return 1
      ;;
  esac
}

podman_arch_name() {
  local value

  value="$(normalize_arch "${1:-}")" || return 1
  case "${value}" in
    x86_64)
      printf 'amd64\n'
      ;;
    aarch64)
      printf 'arm64\n'
      ;;
  esac
}

stage_arch_name() {
  local value

  value="$(normalize_arch "${1:-}")" || return 1
  case "${value}" in
    x86_64)
      printf 'x86-64\n'
      ;;
    aarch64)
      printf 'arm64\n'
      ;;
  esac
}

normalize_glibc_version() {
  local value="$1"
  value="${value#GLIBC_}"
  printf '%s\n' "${value}"
}

glibc_floor_exceeds_limit() {
  local glibc_floor="$1"
  local max_glibc="$2"
  local normalized_floor
  local normalized_max

  normalized_floor="$(normalize_glibc_version "${glibc_floor}")"
  normalized_max="$(normalize_glibc_version "${max_glibc}")"

  [[ "$(printf '%s\n%s\n' "${normalized_max}" "${normalized_floor}" | sort -V | tail -n 1)" != "${normalized_max}" ]]
}

default_linux_portable_arches() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 1
  fi

  case "$(uname -m)" in
    x86_64|amd64)
      printf 'x86_64\n'
      printf 'aarch64\n'
      ;;
    aarch64|arm64)
      printf 'aarch64\n'
      ;;
    *)
      return 1
      ;;
  esac
}

sha256_short() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$@" | sha256sum | awk '{print substr($1, 1, 16)}'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$@" | shasum -a 256 | awk '{print substr($1, 1, 16)}'
    return 0
  fi

  echo "Missing sha256sum/shasum for container image cache keys." >&2
  exit 1
}

run_with_lock() {
  local lock_path="$1"
  shift

  mkdir -p "$(dirname -- "${lock_path}")"

  if command -v flock >/dev/null 2>&1; then
    local lock_fd
    exec {lock_fd}> "${lock_path}"
    flock "${lock_fd}"
    "$@"
    local _ret=$?
    exec {lock_fd}>&-
    return "${_ret}"
  fi

  local lock_dir="${lock_path}.dirlock"
  while ! mkdir "${lock_dir}" 2>/dev/null; do
    sleep 1
  done

  "$@"
  local _ret=$?
  rmdir "${lock_dir}" 2>/dev/null || true
  return "${_ret}"
}

build_cached_podman_image() {
  local image_tag="$1"
  local podman_arch="$2"
  local containerfile="$3"
  local context_dir="$4"

  if podman image exists "${image_tag}" >/dev/null 2>&1; then
    return 0
  fi

  echo "[container] building cached image ${image_tag}..." >&2
  podman build \
    --pull=missing \
    --arch="${podman_arch}" \
    --tag "${image_tag}" \
    --file "${containerfile}" \
    "${context_dir}" >/dev/null
}

ensure_podman_image() {
  local image_name="$1"
  local requested_arch="$2"
  local containerfile="$3"
  local context_dir="${4:-$(dirname -- "${containerfile}")}"
  local podman_arch
  local cache_key
  local image_tag
  local lock_path

  require_command podman "Install Podman so the portable Linux containers can be used."

  podman_arch="$(podman_arch_name "${requested_arch}")"
  cache_key="$(sha256_short "${containerfile}")"
  image_tag="localhost/vertexlauncher-${image_name}:${requested_arch}-${cache_key}"

  if podman image exists "${image_tag}" >/dev/null 2>&1; then
    printf '%s\n' "${image_tag}"
    return 0
  fi

  lock_path="${PORTABLE_LINUX_CACHE_ROOT:-${REPO_ROOT}/.cache}/podman-image-locks/${image_name}-${requested_arch}-${cache_key}.lock"
  run_with_lock "${lock_path}" \
    build_cached_podman_image "${image_tag}" "${podman_arch}" "${containerfile}" "${context_dir}" || return $?

  printf '%s\n' "${image_tag}"
}
