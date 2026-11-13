//! The bundled libheif (libheif-rs `embedded-libheif`: built from source and
//! linked statically; it uses the system libde265) is the one in use.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use libheif_rs::{HeifContext, LibHeif};

#[test]
fn embedded_libheif_is_in_use() {
    let lib = LibHeif::new();
    let v = lib.version();
    // 1.17 (Ubuntu 24.04's system copy) failed on some iPhone HEICs; the
    // embedded build is 1.23 or newer (PLAN §4).
    assert!(v[0] == 1 && v[1] >= 23, "unexpected libheif version {v:?}");
}

#[test]
fn rejects_garbage_input() {
    assert!(HeifContext::read_from_bytes(b"not a heic file").is_err());
}
