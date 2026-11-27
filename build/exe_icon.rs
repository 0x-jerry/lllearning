//! Puts the app's icon into the executable: an icon is a Windows resource, and
//! the only way in is compiling `assets/lllearning.rc` into the link. Without it
//! the exe carries the Rust logo.

use std::path::Path;

/// Compile the icon resource and add it to the binaries' link line.
pub fn embed(manifest: &Path) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // `rc.exe` resolves `icon.ico` from the `.rc`'s own directory.
    println!("cargo:rerun-if-changed=assets/lllearning.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // A toolchain without `rc.exe` still links, with the default icon; a `.rc`
    // that fails to compile does not, because the result is the iconless exe.
    let resource = manifest.join("assets/lllearning.rc");
    match embed_resource::compile(&resource, embed_resource::NONE).manifest_optional() {
        Ok(()) => {}
        Err(failure) => panic!("the executable's icon resource did not compile: {failure}"),
    }
}
