# A Vertex that needs nothing installed (Linux)

`scripts/build-linux-self-contained.sh` builds `target/release/vertexlauncher` as one executable that asks the machine it runs on for
nothing beyond the C library every glibc Linux install has (`libc`, `libm`, the dynamic loader) and glibc 2.34 or newer. It then runs
`scripts/check-linux-self-contained.sh` on the result, which fails if the binary names any other shared library or needs a newer glibc.

A plain `cargo build --release` produces the same self-contained executable (the linker steps are in `crates/vertexlauncher/build.rs` and
always apply on Linux with glibc); the script only adds the strip and the check.

```
scripts/build-linux-self-contained.sh            # build, strip and check
scripts/check-linux-self-contained.sh <binary>   # check any Linux executable
```

## What each piece does

| Dependency | How it is handled |
| --- | --- |
| C++ engine, libstdc++, SDL3, HarfBuzz, spdlog, fmt, codecs, Slang, ... | static archives, linked in by `sturdy-sys` |
| OpenSSL (the engine's TLS) | built from source and linked statically: `sturdy-sys` feature `bundled-openssl` (engine option `STURDY_BUNDLE_OPENSSL`) |
| liblzma | `xz2` feature `static` builds it from its bundled source |
| libgcc (`libgcc_s.so.1`, the unwinder) | `build.rs` puts a `libgcc_s.so` linker script that points at `libgcc_eh.a` / `libgcc.a` first on the search path |
| glibc newer than 2.34 | the engine's `cmake/linux/glibc_compat.c` is compiled into the binary: the affected libm/libc functions (`sqrtf`, `hypot`, `cosh`, the C23 `__isoc23_*` family, `strlcpy`, `arc4random`, `_dl_find_object`, ...) are defined in the executable and forward to the old symbol versions, so the binary records 2.34 instead of the build machine's glibc |
| libudev (gamepads, via gilrs) | `crates/libudev-shim` replaces `libudev-sys` (workspace `[patch.crates-io]`) with a version that opens `libudev.so.1` with `dlopen` on first use. Without libudev, gamepads are reported as unavailable |
| Vulkan loader, X11, Wayland, audio libraries | opened with `dlopen` at run time by the engine / SDL3, as before: they belong to the GPU driver and the desktop |
| WebKitGTK | Vertex has no embedded sign-in webview any more (WebKitGTK cannot be carried in an executable); Microsoft sign-in uses the system browser or a device code |

The glibc floor is a property of the *symbols*, not of the build machine: the check reports the newest version the binary needs. A machine
older than 2.34 would need the build done in an older-glibc container (`scripts/build-linux-x86_64-container.sh`); the symbol pinning
then has nothing to do.

## Other platforms (best effort)

Windows and macOS binaries are not changed by any of this: they link only what the operating system provides, and the Linux-only
linker steps do not apply there.
Whether a Windows build needs the Visual C++ runtime on the machine depends on the CRT choice the engine's CMake makes; that was not
looked into here.
