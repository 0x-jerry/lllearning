//! Subsets the two icon fonts into `OUT_DIR`, and compiles the exe's own icon
//! into the link. Each font's glyph set is derived from the source rather than
//! listed, and generation is skipped while the inputs are unchanged.

mod devicons;
mod exe_icon;
mod font;
mod lucide;

use std::{env, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let nerd_source = manifest.join("assets/SymbolsNerdFontMono-Regular.ttf");
    println!("cargo:rerun-if-changed=assets/SymbolsNerdFontMono-Regular.ttf");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build");

    let lucide_source = font::crate_file("lucide-icons", "lucide.ttf");
    let lucide_output = out_dir.join("lucide-subset.ttf");
    // Covers a changed font and a changed glyph list, and an output left missing
    // by an interrupted run.
    font::subset_if_changed(
        &lucide_source,
        &lucide_output,
        &lucide::drawn_glyphs(&manifest),
        font::Vendor::Lucide,
    );

    let nerd_output = out_dir.join("SymbolsNerdFontMono-subset.ttf");
    font::subset_if_changed(
        &nerd_source,
        &nerd_output,
        &devicons::drawn_glyphs(),
        font::Vendor::NerdFont,
    );

    exe_icon::embed(&manifest);
}
