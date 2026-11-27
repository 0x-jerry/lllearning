//! Cuts the two icon fonts down to the glyphs this app draws.
//!
//! Both fonts ship whole in their crates: lucide's is 1 MB and the nerd font's
//! 2.3 MB. The app draws a dozen-odd lucide glyphs and the ~225 private-use
//! glyphs in `devicons`' table, so almost all of both files would be
//! unreachable data in the binary. This subsets them into `OUT_DIR`, and
//! `src/icons.rs` embeds the results.
//!
//! Each font's glyph set is derived rather than listed: [`lucide`] scans `src/`
//! for the icons the UI names, and [`devicons`] reads the file-type table out of
//! the crate. `src/icons.rs`' coverage tests fail if either goes stale.
//!
//! Generation is skipped when the inputs are unchanged: a release build with
//! LTO takes minutes, and re-subsetting on every build would add to it for no
//! change in output. A changed glyph list counts as a changed input, so adding
//! an `Icon::Variant` still reaches the font.
//!
//! The third call is not about fonts: [`exe_icon`] compiles the executable's own
//! icon into the link, because cargo has no other way to put a resource in the
//! binary.

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
    // The staleness check covers both a changed font and a changed glyph list;
    // it also covers an output missing because a previous run was interrupted.
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
