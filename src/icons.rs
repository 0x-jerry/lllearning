//! Icon glyphs as text.
//!
//! Both sets are fonts whose glyphs sit at private-use code points, so drawing
//! an icon is drawing a one-character string in the right family — there is no
//! SVG, no asset pipeline, and no per-icon registration.
//!
//! `lucide` is the UI's icon set: it has no file types, so the explorer's file
//! and folder glyphs come from a nerd font instead ([`file_icon`]).
//!
//! The family names are not constants in the crates; they are read from the
//! TTFs' `name` tables, where id 1 is `lucide` and `Symbols Nerd Font Mono`.
//! Renaming either here without re-checking that table renders tofu, which is
//! why a test reads both.

use std::borrow::Cow;

use gpui::component::ActiveTheme as _;
use gpui::{App, Div, Hsla, ParentElement as _, SharedString, Styled as _, div, px, rgb};
use gpui_rsx::rsx;
use lucide_icons::{Icon, LUCIDE_FONT_BYTES};

/// The family name from the bundled lucide font's `name` table.
pub const LUCIDE_FAMILY: &str = "lucide";

/// The nerd font that draws file-type glyphs, vendored in `assets/`.
///
/// It is 1.9 MB and bundled for the same reason lucide is: an icon that depends
/// on a font the user happens to have installed is an icon that renders as tofu
/// on the machine you ship to.
const DEVICONS_FONT_BYTES: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");

/// The family name from that font's `name` table.
pub const DEVICONS_FAMILY: &str = "Symbols Nerd Font Mono";

/// Whether [`install_font`] has run. The text system is process-global, so this
/// is too.
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Install the icon fonts into the text system.
///
/// Call once, after [`gpui::init`]. Registering the same font twice is
/// harmless but wasteful, so this is guarded by a flag.
pub fn install_font(cx: &mut App) {
    if INSTALLED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    for font in [LUCIDE_FONT_BYTES, DEVICONS_FONT_BYTES] {
        if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(font)]) {
            // A font that fails to load costs every icon in the app, so it is
            // worth a line on stderr; there is nothing to recover to at startup.
            eprintln!("failed to install an icon font: {error}");
        }
    }
}

/// Draw `glyph` at `size`, centred in a box of that size.
///
/// The box is what keeps a row of icons from drifting: glyph advance widths
/// differ per icon, and centring inside a fixed square makes the visual rhythm
/// independent of them.
pub fn icon(glyph: Icon, size: f32) -> Div {
    glyph_box(
        LUCIDE_FAMILY,
        SharedString::from(char::from(glyph)),
        size,
        None,
    )
}

/// One glyph centred in a square box, in `family` and tinted `color`.
///
/// Every icon in the app is this shape, so the three public helpers are names
/// for how the glyph and its colour are chosen, not for the layout.
fn glyph_box(family: &'static str, glyph: SharedString, size: f32, color: Option<Hsla>) -> Div {
    let el = rsx! {
        <div
            class="flex items-center justify-center flex-none"
            w={px(size)}
            h={px(size)}
            font_family={family}
            text_size={px(size)}
            line_height={px(size)}
        >
            {glyph}
        </div>
    };
    // An untinted icon inherits the text colour of whatever row it sits in.
    match color {
        Some(color) => el.text_color(color),
        None => el,
    }
}

/// Draw the glyph for `path` at `size`, tinted the way the file type is
/// conventionally drawn.
///
/// The colour comes from the `devicons` table, which ships a light and a dark
/// palette; the active theme picks between them.
pub fn file_icon(path: &std::path::Path, size: f32, cx: &App) -> Div {
    let theme = if cx.theme().mode.is_dark() {
        devicons::Theme::Dark
    } else {
        devicons::Theme::Light
    };
    let glyph = devicons::icon_for_file(path, &Some(theme));
    glyph_box(
        DEVICONS_FAMILY,
        SharedString::from(glyph.icon),
        size,
        Some(parse_hex(glyph.color)),
    )
}

/// Read `#rrggbb` as an `Hsla`.
fn parse_hex(hex: &str) -> Hsla {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x7e8ea8);
    rgb(value).into()
}

/// The nerd font's closed and open folder glyphs.
///
/// `devicons` names only the open one, and only through the fallback icon its
/// table returns for a directory; the closed twin sits one code point below,
/// so both are spelled out here rather than derived from the table.
const FOLDER_CLOSED: char = '\u{f114}';
const FOLDER_OPEN: char = '\u{f115}';

/// The folder colour, matching `devicons`' own neutral grey.
const FOLDER_COLOR: &str = "#7e8ea8";

/// Draw a directory, open or closed: the glyph is the expand/collapse
/// indicator as well as the row's icon.
pub fn folder_icon(expanded: bool, size: f32) -> Div {
    let glyph = if expanded { FOLDER_OPEN } else { FOLDER_CLOSED };
    glyph_box(
        DEVICONS_FAMILY,
        SharedString::from(glyph),
        size,
        Some(parse_hex(FOLDER_COLOR)),
    )
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
    fn the_family_names_match_the_bundled_fonts() {
        // Reads the `name` tables directly rather than trusting the constants:
        // neither crate exports them, so a font upgrade could rename one and
        // nothing else would notice.
        assert_eq!(
            ttf_family_name(LUCIDE_FONT_BYTES).expect("the font has a name table"),
            LUCIDE_FAMILY
        );
        assert_eq!(
            ttf_family_name(DEVICONS_FONT_BYTES).expect("the font has a name table"),
            DEVICONS_FAMILY
        );
    }

    #[test]
    fn a_known_extension_gets_a_glyph_outside_ascii() {
        // The default entry is `*`, which draws whether or not the nerd font
        // loaded; a real extension's glyph is what proves the table was read.
        let icon = devicons::icon_for_file("src/main.rs", &None);
        assert!(!icon.icon.is_ascii(), "{:?} is the fallback", icon.icon);
    }

    #[test]
    fn the_folder_glyphs_are_not_the_file_fallback() {
        // Both chevron replacements come from this table, so a rename in a
        // future `devicons` would silently give every directory a `*`.
        for path in ["x/", "x"] {
            let icon = devicons::icon_for_file(path, &None);
            assert!(!icon.icon.is_ascii(), "{path} fell back to {:?}", icon.icon);
        }
    }

    #[test]
    fn hex_colours_parse_and_fall_back() {
        assert_eq!(parse_hex("#000000"), rgb(0x000000).into());
        assert_eq!(parse_hex("#ffffff"), rgb(0xffffff).into());
        // A malformed colour is the table's problem, not the row's: grey beats
        // a panic in the middle of a render.
        assert_eq!(parse_hex("nonsense"), parse_hex("#7e8ea8"));
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
