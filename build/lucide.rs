//! Which lucide glyphs the app draws, and how to find them.
//!
//! The UI names icons as `Icon::Variant` (`lucide_icons::Icon`), so the set is
//! whatever `src/` mentions. Scanning rather than listing keeps the subset from
//! going stale: adding an icon to a view is enough, and `src/icons.rs`' coverage
//! test fails if the font was not rebuilt.
//!
//! `lucide_icons::Icon` also exposes the crate's own name-to-code-point table,
//! so a lucide upgrade that moves a glyph is picked up here rather than pinned
//! as a literal.

use std::{collections::BTreeSet, fs, path::Path};

use crate::font;

/// Every lucide glyph the app draws, as code points.
pub fn drawn_glyphs(manifest: &Path) -> Vec<u32> {
    let mut glyphs = BTreeSet::new();
    for variant in variants(manifest) {
        let name = kebab_case(&variant);
        let icon = lucide_icons::Icon::try_from(name.as_str())
            .unwrap_or_else(|_| panic!("`Icon::{variant}` in src/ is not a lucide icon"));
        glyphs.insert(char::from(icon) as u32);
    }
    glyphs.into_iter().collect()
}

/// Every `Icon::Variant` named under `src/`.
///
/// `Icon::new` and `Icon::from` are GPUI Component's icon builder, which shares
/// the `Icon` name; neither is a variant, and both are dropped here. Everything
/// else is resolved against lucide, where an unknown one is a build error.
fn variants(manifest: &Path) -> BTreeSet<String> {
    const NOT_VARIANTS: [&str; 2] = ["new", "from"];
    let mut variants = BTreeSet::new();
    for file in font::rust_sources(&manifest.join("src")) {
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        for (at, _) in source.match_indices("Icon::") {
            let rest = &source[at + "Icon::".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            // `Icon::from` and a bare `Icon::` in a doc comment both land here.
            if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            if !NOT_VARIANTS.contains(&name.as_str()) {
                variants.insert(name);
            }
        }
    }
    assert!(
        !variants.is_empty(),
        "no `Icon::Variant` uses found under src/; the scan is broken"
    );
    variants
}

/// `FolderOpen` → `folder-open`, which is how lucide names its icons.
fn kebab_case(variant: &str) -> String {
    let mut out = String::with_capacity(variant.len() + 4);
    let chars: Vec<char> = variant.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let previous = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|next| next.is_lowercase());
            if previous.is_lowercase() || previous.is_numeric() || next_is_lower {
                out.push('-');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
