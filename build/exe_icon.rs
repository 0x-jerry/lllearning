//! Puts the app's icon into the executable itself.
//!
//! Cargo links no resources of its own, so without this the exe carries the Rust
//! logo: Explorer, the taskbar button and a portable copy all show an icon that
//! is not the app. An icon is a Windows resource, and the only way into the
//! binary is to compile a `.rc` into the link — which is what
//! `assets/lllearning.rc` is for. It names `assets/icon.ico`, the artwork
//! `assets/build-icon.ts` rasterises from `icon.svg` — the same file the window
//! (`app/title_bar.rs`) and the tray embed — at 16 to 256 pixels, so Explorer has
//! a crisp one at every zoom level.
//!
//! This is the one build step that is not about fonts, and it only applies to a
//! Windows target: elsewhere there is no resource to link, and a `.lib` named on
//! the link line would be a hard error.

use std::path::Path;

/// Compile the icon resource and add it to the binaries' link line.
pub fn embed(manifest: &Path) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // `rc.exe` resolves `icon.ico` from the `.rc`'s own directory, so the icon
    // needs no path here; these are what cargo must rerun the script on.
    println!("cargo:rerun-if-changed=assets/lllearning.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // A toolchain without `rc.exe` is not an error here — the binary still
    // links, just with the default icon — but a `.rc` that fails to compile is,
    // because the result is exactly the iconless exe this replaces.
    let resource = manifest.join("assets/lllearning.rc");
    match embed_resource::compile(&resource, embed_resource::NONE).manifest_optional() {
        Ok(()) => {}
        Err(failure) => panic!("the executable's icon resource did not compile: {failure}"),
    }
}
