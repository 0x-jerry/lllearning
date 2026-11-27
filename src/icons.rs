//! Icon glyphs as text: both sets are fonts whose glyphs sit at private-use code
//! points, so drawing an icon is drawing one character in the right family.
//! `lucide` covers the UI; a nerd font covers file types.

use std::borrow::Cow;

use gpui::component::ActiveTheme as _;
use gpui::{App, Div, Hsla, ParentElement as _, SharedString, Styled as _, div, px, rgb};
use gpui_rsx::rsx;
use lucide_icons::Icon;

/// The family name from the bundled lucide font's `name` table; renaming it here
/// without re-checking that table renders tofu.
pub const LUCIDE_FAMILY: &str = "lucide";

/// The lucide font, subset to the glyphs this app draws by `build/lucide.rs`.
const LUCIDE_FONT_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lucide-subset.ttf"));

/// The nerd font that draws file-type glyphs, subset the same way. It is
/// vendored in `assets/` so the icons do not depend on a font the user has.
const DEVICONS_FONT_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/SymbolsNerdFontMono-subset.ttf"));

/// The family name from that font's `name` table.
pub const DEVICONS_FAMILY: &str = "Symbols Nerd Font Mono";

/// Whether [`install_font`] has run; the text system is process-global.
static INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Install the icon fonts into the text system, once, after [`gpui::init`].
pub fn install_font(cx: &mut App) {
    if INSTALLED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    for font in [LUCIDE_FONT_BYTES, DEVICONS_FONT_BYTES] {
        if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(font)]) {
            // A font that fails to load costs every icon, and there is nothing
            // to recover to at startup.
            log::error!("failed to install an icon font: {error}");
        }
    }
}

/// Draw `glyph` at `size`, centred in a box of that size: glyphs have different
/// advance widths, and the box keeps a row of icons from drifting.
pub fn icon(glyph: Icon, size: f32) -> Div {
    glyph_box(
        LUCIDE_FAMILY,
        SharedString::from(char::from(glyph)),
        size,
        None,
    )
}

/// One glyph centred in a square box, in `family` and tinted `color`.
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
/// conventionally drawn; the active theme picks the palette.
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

/// The nerd font's closed and open folder glyphs. `devicons` names only the open
/// one, so both are spelled out here.
const FOLDER_CLOSED: char = '\u{f114}';
const FOLDER_OPEN: char = '\u{f115}';

/// The folder colour, matching `devicons`' own neutral grey.
const FOLDER_COLOR: &str = "#7e8ea8";

/// Draw a directory: the glyph is the expand/collapse indicator too.
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
        for glyph in [Icon::Folder, Icon::File, Icon::X, Icon::Globe] {
            let text = char::from(glyph).to_string();
            assert_eq!(text.chars().count(), 1, "{glyph:?} is not one glyph");
        }
    }

    #[test]
    fn the_lucide_font_covers_every_icon_this_build_draws() {
        // The bundled font is a subset, so a new `Icon` in the UI only renders
        // after a rebuild regenerates it; this turns tofu into a failure.
        let coverage = Coverage::of(LUCIDE_FONT_BYTES);
        let missing: Vec<char> = DRAWN_ICONS
            .iter()
            .map(|glyph| char::from(*glyph))
            .filter(|glyph| !coverage.has(*glyph))
            .collect();
        assert!(
            missing.is_empty(),
            "the lucide subset has no glyph for {missing:?}; rebuild to regenerate it"
        );
    }

    /// Every lucide icon the app draws. `build/lucide.rs` derives the same set
    /// from the `Icon::` uses under `src/`; this test is what notices the two
    /// disagreeing.
    const DRAWN_ICONS: [Icon; 17] = [
        Icon::ArrowLeft,
        Icon::ArrowRight,
        Icon::Book,
        Icon::Columns2,
        Icon::Eye,
        Icon::File,
        Icon::Files,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Globe,
        Icon::ListTree,
        Icon::RefreshCw,
        Icon::RotateCw,
        Icon::SquarePen,
        Icon::TriangleAlert,
        Icon::WrapText,
        Icon::X,
    ];

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
    fn the_font_covers_every_glyph_the_devicons_table_uses() {
        // A missing private-use glyph draws tofu, and nothing else in the build
        // can see the disagreement between the table and the font.
        let coverage = Coverage::of(DEVICONS_FONT_BYTES);
        let missing: Vec<char> = COMMON_FILE_NAMES
            .iter()
            .map(|name| devicons::icon_for_file(*name, &Some(devicons::Theme::Dark)).icon)
            // An ASCII glyph is `devicons`' own fallback and renders in any font.
            .filter(|glyph| !glyph.is_ascii() && !coverage.has(*glyph))
            .collect();
        assert!(
            missing.is_empty(),
            "the nerd font has no glyph for {missing:?}; update the vendored font"
        );
    }

    /// A sample of the file types a workspace of notes and code is made of.
    const COMMON_FILE_NAMES: [&str; 16] = [
        "Cargo.toml",
        "main.rs",
        "README.md",
        "package.json",
        "index.ts",
        "index.js",
        "Cargo.lock",
        "ci.yml",
        "notes.txt",
        "index.html",
        "style.css",
        "main.py",
        "main.go",
        "run.sh",
        "Dockerfile",
        "Makefile",
    ];

    /// Which code points a font's `cmap` maps, for the coverage check above.
    struct Coverage(std::collections::HashSet<u32>);

    impl Coverage {
        fn of(font: &[u8]) -> Self {
            let mut mapped = std::collections::HashSet::new();
            let tables = be_u16(font, 4).unwrap_or(0);
            let Some(cmap) = (0..tables)
                .map(|i| 12 + i * 16)
                .find(|&at| font.get(at..at + 4) == Some(b"cmap"))
                .and_then(|at| be_u32(font, at + 8))
            else {
                return Self(mapped);
            };
            for record in 0..be_u16(font, cmap + 2).unwrap_or(0) {
                let at = cmap + 4 + record * 8;
                let subtable = cmap + be_u32(font, at + 4).unwrap_or(0);
                match be_u16(font, subtable).unwrap_or(0) {
                    4 => Self::format_4(font, subtable, &mut mapped),
                    // The icon fonts ship a format 12 subtable for the planes
                    // above the BMP.
                    12 => Self::format_12(font, subtable, &mut mapped),
                    _ => {}
                }
            }
            Self(mapped)
        }

        fn format_4(font: &[u8], subtable: usize, mapped: &mut std::collections::HashSet<u32>) {
            let segments = be_u16(font, subtable + 6).unwrap_or(0) / 2;
            let ends = subtable + 14;
            let starts = ends + segments * 2 + 2;
            let deltas = starts + segments * 2;
            let offsets = deltas + segments * 2;
            for segment in 0..segments {
                let end = be_u16(font, ends + segment * 2).unwrap_or(0xffff);
                let start = be_u16(font, starts + segment * 2).unwrap_or(0xffff);
                let delta = be_u16(font, deltas + segment * 2).unwrap_or(0);
                let offset = be_u16(font, offsets + segment * 2).unwrap_or(0);
                for code in start..=end {
                    if code == 0xffff {
                        break;
                    }
                    let glyph = if offset == 0 {
                        (code as u16).wrapping_add(delta as u16)
                    } else {
                        // The glyph index lives in `glyphIdArray`.
                        let at = offsets + segment * 2 + offset + (code - start) * 2;
                        match be_u16(font, at).unwrap_or(0) {
                            0 => 0,
                            id => (id as u16).wrapping_add(delta as u16),
                        }
                    };
                    if glyph != 0 {
                        mapped.insert(code as u32);
                    }
                }
            }
        }

        fn format_12(font: &[u8], subtable: usize, mapped: &mut std::collections::HashSet<u32>) {
            let groups = be_u32(font, subtable + 12).unwrap_or(0);
            for group in 0..groups {
                let at = subtable + 16 + group * 12;
                let start = be_u32(font, at).unwrap_or(0) as u32;
                let end = be_u32(font, at + 4).unwrap_or(0) as u32;
                // A group whose first glyph is 0 has no glyphs to record.
                if be_u32(font, at + 8).unwrap_or(0) == 0 {
                    continue;
                }
                for code in start..=end {
                    mapped.insert(code);
                }
            }
        }

        fn has(&self, glyph: char) -> bool {
            self.0.contains(&(glyph as u32))
        }
    }

    #[test]
    fn a_known_extension_gets_a_glyph_outside_ascii() {
        // The default entry is `*`; a real extension's glyph proves the table
        // was read.
        let icon = devicons::icon_for_file("src/main.rs", &None);
        assert!(!icon.icon.is_ascii(), "{:?} is the fallback", icon.icon);
    }

    #[test]
    fn the_folder_glyphs_are_not_the_file_fallback() {
        for path in ["x/", "x"] {
            let icon = devicons::icon_for_file(path, &None);
            assert!(!icon.icon.is_ascii(), "{path} fell back to {:?}", icon.icon);
        }
    }

    #[test]
    fn hex_colours_parse_and_fall_back() {
        assert_eq!(parse_hex("#000000"), rgb(0x000000).into());
        assert_eq!(parse_hex("#ffffff"), rgb(0xffffff).into());
        // A malformed colour is the table's problem, not the row's.
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
