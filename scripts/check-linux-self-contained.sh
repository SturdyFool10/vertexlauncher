#!/usr/bin/env bash
# Checks that a Linux executable needs nothing from the machine it runs on except what every Linux install of its kind has:
#   * every NEEDED shared library is one of the C library's (libc, libm, libdl, libpthread, librt, libutil and the dynamic loader);
#   * no glibc symbol version newer than the floor is required (weak references to newer ones are tolerated: the program tests for them).
# Libraries the program opens itself with dlopen at run time (the Vulkan loader, X11/Wayland, audio, libudev) are not in the list and do
# not count: the program starts without them and treats their absence as "feature not available".
#
# usage: check-linux-self-contained.sh <elf-binary> [glibc-floor, default 2.34]
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <elf-binary> [glibc-floor]" >&2
  exit 2
fi
binary="$1"
floor="${2:-2.34}"
[[ -f "${binary}" ]] || { echo "missing binary: ${binary}" >&2; exit 2; }
command -v readelf >/dev/null 2>&1 || { echo "readelf is required" >&2; exit 2; }

allowed_libraries='^(libc\.so\.6|libm\.so\.6|libdl\.so\.2|libpthread\.so\.0|librt\.so\.1|libutil\.so\.1|ld-linux-x86-64\.so\.2|ld-linux-aarch64\.so\.1|ld-linux\.so\.2)$'
failed=0

echo "== shared libraries the binary needs"
while read -r library; do
  [[ -n "${library}" ]] || continue
  if [[ "${library}" =~ ${allowed_libraries} ]]; then
    echo "   ok      ${library}"
  else
    echo "   NOT ALLOWED  ${library}   <-- not part of the C library"
    failed=1
  fi
done < <(readelf -d "${binary}" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')

echo "== newest glibc symbol version required (floor ${floor})"
versions="$(
  readelf --dyn-syms -W "${binary}" \
    | awk '$7 == "UND" && $5 != "WEAK" { print $8 }' \
    | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/GLIBC_//' | sort -Vu || true
)"
newest="$(printf '%s\n' "${versions}" | tail -n 1)"
echo "   newest: ${newest:-none}"
if [[ -n "${versions}" ]]; then
  while read -r version; do
    [[ -n "${version}" ]] || continue
    if [[ "$(printf '%s\n%s\n' "${version}" "${floor}" | sort -V | tail -n 1)" != "${floor}" ]]; then
      offenders="$(readelf --dyn-syms -W "${binary}" | awk -v v="GLIBC_${version}" '$7 == "UND" && $5 != "WEAK" && index($8, v) { print "      " $8 }' | sort -u | head -8)"
      echo "   TOO NEW  GLIBC_${version}"
      echo "${offenders}"
      failed=1
    fi
  done <<<"${versions}"
fi

if [[ ${failed} -ne 0 ]]; then
  echo "NOT self-contained" >&2
  exit 1
fi
echo "self-contained: needs only the C library (glibc >= ${newest:-?})"
