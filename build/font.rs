//! Shared plumbing: finding a dependency's files, subsetting, and the small
//! filesystem helpers both font modules need.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use allsorts::{
    binary::read::ReadScope,
    font::read_cmap_subtable,
    font_data::FontData,
    subset::{CmapTarget, SubsetProfile, subset},
    tables::{FontTableProvider, cmap::Cmap},
};

/// Which font is being cut, for the quirks that are per-font.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    /// lucide's TTF carries a malformed `GSUB` table that has to be dropped.
    Lucide,
    NerdFont,
}

/// Whether `output` is missing or older than any of `inputs`.
pub fn stale(output: &Path, inputs: &[&Path]) -> bool {
    let Ok(modified) = fs::metadata(output).and_then(|meta| meta.modified()) else {
        return true;
    };
    inputs.iter().any(|input| {
        fs::metadata(input)
            .and_then(|meta| meta.modified())
            .map(|input| input > modified)
            .unwrap_or(true)
    })
}

/// The path of `file` inside the unpacked `crate_name` crate.
///
/// A build script cannot ask cargo where a dependency's source is, so this walks
/// the registry layout; the crate must already be fetched.
pub fn crate_file(crate_name: &str, file: &str) -> PathBuf {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .or_else(|| env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        })
        .expect("a cargo home to read the registry from");
    let registry = cargo_home.join("registry").join("src");
    let sources = fs::read_dir(&registry).unwrap_or_else(|error| {
        panic!(
            "cannot read {}: {error}; run `cargo fetch`",
            registry.display()
        )
    });

    for source in sources.flatten() {
        let crates = match fs::read_dir(source.path()) {
            Ok(crates) => crates,
            Err(_) => continue,
        };
        for entry in crates.flatten() {
            // Match `name-version`, not `name` alone.
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let matches = name.strip_prefix(crate_name).is_some_and(|rest| {
                rest.strip_prefix('-')
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(|first| first.is_ascii_digit())
            });
            if matches {
                let path = entry.path().join(file);
                if path.is_file() {
                    return path;
                }
            }
        }
    }
    panic!("cannot find {file} in the {crate_name} crate; run `cargo fetch`");
}

/// Subset `source` to `glyphs` and write it to `output`, unless neither the font
/// nor the glyph list changed since the last run.
///
/// The glyph list is an input like the font is: a new `Icon::Variant` changes it
/// without touching the font's timestamp, and a timestamp alone would draw tofu.
pub fn subset_if_changed(source: &Path, output: &Path, glyphs: &[u32], vendor: Vendor) {
    // The record beside the output makes the glyph list comparable across runs.
    let record = output.with_extension("glyphs");
    let wanted = glyphs
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let recorded = fs::read_to_string(&record).ok();
    if !stale(output, &[source]) && recorded.as_deref() == Some(wanted.as_str()) {
        return;
    }
    subset_font(source, output, glyphs, vendor);
    fs::write(&record, wanted)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", record.display()));
}

/// Subset `source` to `glyphs` and write it to `output`. Fails the build rather
/// than shipping a font with a hole in it, which would draw tofu at runtime.
pub fn subset_font(source: &Path, output: &Path, glyphs: &[u32], vendor: Vendor) {
    let what = match vendor {
        Vendor::Lucide => "lucide",
        Vendor::NerdFont => "the nerd font",
    };
    let data = fs::read(source)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));
    let mut data = data;

    // lucide's `GSUB` table has a ligature-set count that disagrees with its
    // glyph lists, so a shaper refuses the whole file. The app never asks for a
    // ligature, so the table is dead weight: renaming its tag leaves every
    // offset valid and makes the parser skip it.
    if vendor == Vendor::Lucide {
        drop_gsub(&mut data);
    }

    let font = ReadScope::new(&data)
        .read::<FontData<'_>>()
        .unwrap_or_else(|error| panic!("{what} is not a font: {error}"));
    let provider = font
        .table_provider(0)
        .unwrap_or_else(|error| panic!("{what} has no first face: {error}"));
    let cmap_data = provider
        .read_table_data(allsorts::tag::CMAP)
        .unwrap_or_else(|error| panic!("{what} has no cmap: {error}"));
    let cmap = ReadScope::new(&cmap_data)
        .read::<Cmap<'_>>()
        .unwrap_or_else(|error| panic!("cannot read {what}'s cmap: {error}"));
    let (_encoding, subtable) = read_cmap_subtable(&cmap)
        .unwrap_or_else(|error| panic!("cannot read {what}'s cmap: {error}"))
        .unwrap_or_else(|| panic!("{what} has no unicode cmap"));

    let mut glyph_ids = vec![0u16]; // `.notdef`, which a font must keep.
    let mut missing = Vec::new();
    for glyph in glyphs {
        match subtable.map_glyph(*glyph) {
            Ok(Some(id)) => glyph_ids.push(id),
            Ok(None) => missing.push(format!("U+{glyph:04X}")),
            Err(error) => panic!("cannot map U+{glyph:04X} in {what}: {error}"),
        }
    }
    assert!(
        missing.is_empty(),
        "{what} has no glyph for {}; the app would draw tofu",
        missing.join(", ")
    );
    glyph_ids.sort_unstable();
    glyph_ids.dedup();

    let subset = subset(
        &provider,
        &glyph_ids,
        &SubsetProfile::Minimal,
        CmapTarget::Unicode,
    )
    .unwrap_or_else(|error| panic!("cannot subset {what}: {error}"));
    fs::write(output, &subset)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", output.display()));
    println!(
        "cargo:warning={what} subset: {} glyphs, {} bytes (was {})",
        glyph_ids.len(),
        subset.len(),
        data.len()
    );
}

/// Rename a font's `GSUB` tag so parsers skip the table. See [`subset_font`].
fn drop_gsub(data: &mut [u8]) {
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    for index in 0..num_tables {
        let at = 12 + index * 16;
        if &data[at..at + 4] == b"GSUB" {
            data[at..at + 4].copy_from_slice(b"XSUB");
            return;
        }
    }
}

/// Every `.rs` file under `root`, recursively.
pub fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let entries = fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", root.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    found
}
