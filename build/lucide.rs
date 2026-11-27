//! Which lucide glyphs the app draws: whatever `Icon::Variant` the sources name.
//! Scanning keeps the subset from going stale, and the crate's own name-to-code
//! table resolves each variant.

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

/// Every `Icon::Variant` named under `src/`. `Icon::new` and `Icon::from` are
/// GPUI Component's builder and are dropped; anything else must be a lucide icon.
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

/// `FolderOpen` → `folder-open`, and `Columns2` → `columns-2`, which is how
/// lucide names its icons.
fn kebab_case(variant: &str) -> String {
    let mut out = String::with_capacity(variant.len() + 4);
    let chars: Vec<char> = variant.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 {
            let previous = chars[i - 1];
            // A capital starts a new word, except in a run of capitals:
            // `Columns2Cog` is `columns-2-cog`, `SquarePen` is `square-pen`.
            let word_break = c.is_uppercase()
                && (previous.is_lowercase()
                    || previous.is_numeric()
                    || chars.get(i + 1).is_some_and(|next| next.is_lowercase()));
            // lucide spells the numbered variants with a dash (`columns-2`).
            let number_break = c.is_ascii_digit() && previous.is_ascii_alphabetic();
            if word_break || number_break {
                out.push('-');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
