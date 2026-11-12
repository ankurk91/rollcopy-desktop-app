//! M0 check: libheif-rs links against the system libheif and can decode HEIC.

use libheif_rs::{HeifContext, LibHeif};

#[test]
fn system_libheif_is_usable() {
    let lib = LibHeif::new();
    let v = lib.version();
    assert!(v[0] == 1 && v[1] >= 17, "unexpected libheif version {v:?}");
}

#[test]
fn rejects_garbage_input() {
    assert!(HeifContext::read_from_bytes(b"not a heic file").is_err());
}
