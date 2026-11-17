//! Compiles `src/shim.c`: definitions of every libudev function this crate declares, which open `libudev.so.1` on first use. The real
//! `libudev-sys` links libudev through pkg-config, which puts a shared library in the finished program's NEEDED list (and makes the
//! program refuse to start on a machine without it). With this crate patched in (see the workspace manifest's `[patch.crates-io]`), a
//! missing libudev instead makes the calls fail the way libudev itself fails, and the gamepad layer above reports "no gamepads".

fn main() {
    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-changed=generate_shim.py");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        cc::Build::new().file("src/shim.c").warnings(true).compile("udev_shim");
        // `udev` (the crate above this one) checks this to decide whether it may expose the hardware database; the shim declares none of it.
    }
    println!("cargo:hwdb=false");
}
