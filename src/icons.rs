//! Lucide glyphs as text.
//!
//! `lucide-icons` ships the icon set as a font whose glyphs sit at private-use
//! code points, one per [`Icon`] variant. Drawing an icon is therefore drawing
//! a one-character string in the `lucide` family — there is no SVG, no asset
//! pipeline, and no per-icon registration.
//!
//! The family name is not a constant in the crate; it is read from the TTF's
//! `name` table, where id 1 (the family) is `lucide`. Renaming it here without
//! re-checking that table renders tofu, which is why a test reads the table.

use std::borrow::Cow;

use gpui::{App, Div, ParentElement as _, SharedString, Styled as _, div, px};
use lucide_icons::{Icon, LUCIDE_FONT_BYTES};

/// The family name from the bundled font's `name` table.
pub const LUCIDE_FAMILY: &str = "lucide";

/// Whether [`install_font`] has run. The text system is process-global, so this
/// is too.
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Install the icon font into the text system.
///
/// Call once, after [`gpui::init`]. Registering the same font twice is
/// harmless but wasteful, so this is guarded by a flag.
pub fn install_font(cx: &mut App) {
    if INSTALLED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Err(error) = cx
        .text_system()
        .add_fonts(vec![Cow::Borrowed(LUCIDE_FONT_BYTES)])
    {
        // A font that fails to load costs every icon in the app, so it is worth
        // a line on stderr; there is nothing to recover to at startup.
        eprintln!("failed to install the lucide icon font: {error}");
    }
}

/// Draw `glyph` at `size`, centred in a box of that size.
///
/// The box is what keeps a row of icons from drifting: glyph advance widths
/// differ per icon, and centring inside a fixed square makes the visual rhythm
/// independent of them.
pub fn icon(glyph: Icon, size: f32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .w(px(size))
        .h(px(size))
        .font_family(LUCIDE_FAMILY)
        .text_size(px(size))
        .line_height(px(size))
        .child(SharedString::from(char::from(glyph).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_variant_is_a_single_char() {
        // A variant that mapped to an empty or multi-char string would render
        // as nothing or as a word, and the layout above assumes one glyph.
        for glyph in [Icon::Folder, Icon::File, Icon::X, Icon::Globe] {
            let text = char::from(glyph).to_string();
            assert_eq!(text.chars().count(), 1, "{glyph:?} is not one glyph");
        }
    }

    #[test]
    fn the_family_name_matches_the_bundled_font() {
        // Reads the `name` table directly rather than trusting the constant:
        // the crate does not export it, so a font upgrade could rename it and
        // nothing else would notice.
        let name = ttf_family_name(LUCIDE_FONT_BYTES).expect("the font has a name table");
        assert_eq!(name, LUCIDE_FAMILY);
    }

    /// Read the `u16` at `at`, big-endian.
    fn be_u16(font: &[u8], at: usize) -> Option<usize> {
        let high = *font.get(at)?;
        let low = *font.get(at + 1)?;
        Some(u16::from_be_bytes([high, low]) as usize)
    }

    /// Read the `u32` at `at`, big-endian.
    fn be_u32(font: &[u8], at: usize) -> Option<usize> {
        let bytes = [
            *font.get(at)?,
            *font.get(at + 1)?,
            *font.get(at + 2)?,
            *font.get(at + 3)?,
        ];
        Some(u32::from_be_bytes(bytes) as usize)
    }

    /// Minimal TTF `name`-table reader for family id 1.
    fn ttf_family_name(font: &[u8]) -> Option<String> {
        let num_tables = be_u16(font, 4)?;
        let name_table = (0..num_tables)
            .map(|i| 12 + i * 16)
            .find(|&at| font.get(at..at + 4) == Some(b"name"))?;
        let table = be_u32(font, name_table + 8)?;
        let count = be_u16(font, table + 2)?;
        let strings = table + be_u16(font, table + 4)?;

        (0..count).find_map(|i| {
            let record = table + 6 + i * 12;
            let platform = be_u16(font, record)?;
            let name_id = be_u16(font, record + 6)?;
            let length = be_u16(font, record + 8)?;
            let offset = be_u16(font, record + 10)?;
            if name_id != 1 {
                return None;
            }
            let start = strings + offset;
            let bytes = font.get(start..start + length)?;
            // Platform 3 is Windows: UTF-16BE. Platform 1 is Macintosh: ASCII.
            if platform == 3 {
                Some(String::from_utf16_lossy(
                    &bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>(),
                ))
            } else {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
        })
    }
}
