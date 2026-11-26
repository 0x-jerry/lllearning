//! Which nerd-font glyphs the explorer can draw.
//!
//! File-type glyphs come from `devicons`, which keeps its dark and light tables
//! private, so the code points are read out of the crate source. That way a
//! table update reaches the font without this file knowing what changed, and
//! `src/icons.rs`' coverage test catches the case where it does not.

use std::{collections::BTreeSet, fs};

use crate::font;

/// The nerd font's closed and open folder glyphs, which `src/icons.rs` spells
/// out itself: `devicons` names only the open one, and only through the fallback
/// its table returns for a directory.
const FOLDER_GLYPHS: [u32; 2] = [0xf114, 0xf115];

/// Every private-use glyph `devicons` can return, plus the folder pair above.
pub fn drawn_glyphs() -> Vec<u32> {
    let mut glyphs = BTreeSet::from(FOLDER_GLYPHS);
    for theme in ["dark", "light"] {
        let source = font::crate_file("devicons", &format!("src/icons/{theme}.rs"));
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));
        for code_point in code_points(&text) {
            if is_private_use(code_point) {
                glyphs.insert(code_point);
            }
        }
    }
    assert!(
        glyphs.len() > FOLDER_GLYPHS.len(),
        "no devicons glyphs found; the scan is broken"
    );
    glyphs.into_iter().collect()
}

/// The code points of the `icon:` literals in one table.
///
/// Each entry is a char literal: one literal character, or an escaped code point
/// written as `\u{hex}`. The hex is read as text, so this needs no unescaping.
fn code_points(table: &str) -> Vec<u32> {
    let mut code_points = Vec::new();
    for (at, _) in table.match_indices("icon: '") {
        let rest = &table[at + "icon: '".len()..];
        let Some(end) = rest.find('\'') else { continue };
        let literal = &rest[..end];
        let code_point = match literal.find('{') {
            None => literal.chars().next().map(|c| c as u32),
            Some(brace) => literal[brace + 1..]
                .find('}')
                .and_then(|end| u32::from_str_radix(&literal[brace + 1..brace + 1 + end], 16).ok()),
        };
        code_points.extend(code_point);
    }
    code_points
}

/// Whether `code_point` is in a Unicode private-use area, which is where both
/// icon fonts put their glyphs.
///
/// A few `devicons` entries name an ordinary symbol instead (λ, ∞, a gear);
/// those are not in the nerd font and not what the explorer draws, so they are
/// skipped rather than asserted against. The table's ASCII fallback for unknown
/// files is skipped for the same reason — any font draws it.
fn is_private_use(code_point: u32) -> bool {
    matches!(
        code_point,
        0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd
    )
}
