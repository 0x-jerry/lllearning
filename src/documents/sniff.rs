//! The file-type table, and the binary/size checks that decide whether a file
//! can be opened at all.
//!
//! This is deliberately one small table rather than a trait per extension: the
//! providers' `score` functions read it, so the knowledge of "what is a
//! markdown file" lives in one place instead of being spread across four
//! modules that could disagree.

use super::Sniff;

/// Which provider an extension belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Video,
    Markdown,
    /// The fallback the text viewer claims. Nothing constructs it today; it
    /// exists so [`kind_for`] answers every extension rather than `None`.
    #[allow(dead_code)]
    Text,
}

/// Extensions that are markdown.
const MARKDOWN: [&str; 3] = ["md", "markdown", "mdx"];

/// Extensions that are images GPUI can decode.
///
/// The list matches the formats the image element accepts; anything else falls
/// through to the text viewer, which reports a binary file rather than drawing
/// a broken image.
const IMAGE: [&str; 12] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "avif", "svg", "qoi",
];

/// Extensions that are video.
const VIDEO: [&str; 5] = ["mp4", "webm", "mov", "mkv", "avi"];

/// The kind of file an extension names, if it names one.
pub fn kind_of(ext: Option<&str>) -> Option<FileKind> {
    let ext = ext?;
    if IMAGE.contains(&ext) {
        Some(FileKind::Image)
    } else if VIDEO.contains(&ext) {
        Some(FileKind::Video)
    } else if MARKDOWN.contains(&ext) {
        Some(FileKind::Markdown)
    } else {
        None
    }
}

/// The tree-sitter language name for an extension.
///
/// The editor accepts these as `language(..)`; a name with no grammar behind it
/// renders unhighlighted rather than failing, so this table only has to be
/// right about the common cases. Extensions not listed are plain text.
pub fn language_for(ext: Option<&str>) -> &'static str {
    match ext.unwrap_or("") {
        "rs" => "rust",
        "md" | "markdown" | "mdx" => "markdown",
        "json" | "jsonc" => "json",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sh" | "bash" | "zsh" => "bash",
        "py" | "pyi" => "python",
        "html" | "htm" => "html",
        "css" => "css",
        "go" => "go",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "rb" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "lua" => "lua",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "zig" => "zig",
        "diff" | "patch" => "diff",
        "xml" => "xml",
        "ini" | "cfg" | "conf" | "properties" => "ini",
        "dockerfile" | "docker" => "dockerfile",
        _ => "text",
    }
}

/// The display name of a language, for a viewer that reports one.
///
/// The tree-sitter names are lowercase identifiers; a reader writes the language
/// as a word.
pub fn language_label(language: &str) -> String {
    match language {
        "text" => "Plain Text".to_owned(),
        "rust" => "Rust".to_owned(),
        "json" => "JSON".to_owned(),
        "yaml" => "YAML".to_owned(),
        "toml" => "TOML".to_owned(),
        "html" => "HTML".to_owned(),
        "css" => "CSS".to_owned(),
        "sql" => "SQL".to_owned(),
        "csharp" => "C#".to_owned(),
        "cpp" => "C++".to_owned(),
        "javascript" => "JavaScript".to_owned(),
        "typescript" => "TypeScript".to_owned(),
        "tsx" => "TSX".to_owned(),
        "python" => "Python".to_owned(),
        "bash" => "Shell".to_owned(),
        "markdown" => "Markdown".to_owned(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Plain Text".to_owned(),
            }
        }
    }
}

/// Why a file cannot be opened as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The file is binary.
    Binary,
    /// The file is larger than [`crate::workspace::fs_util::MAX_TEXT_BYTES`].
    TooLarge { len: u64, limit: u64 },
}

impl Refusal {
    /// The sentence shown in the panel that replaces the buffer.
    pub fn message(&self) -> String {
        match self {
            Self::Binary => "This file is binary and cannot be shown as text.".to_owned(),
            Self::TooLarge { len, limit } => format!(
                "This file is {} and too large to open; the limit is {}.",
                human_size(*len),
                human_size(*limit)
            ),
        }
    }
}

/// Decide whether `sniff` allows the text viewer.
pub fn text_refusal(sniff: &Sniff) -> Option<Refusal> {
    if sniff.is_oversized() {
        return Some(Refusal::TooLarge {
            len: sniff.len,
            limit: crate::workspace::fs_util::MAX_TEXT_BYTES,
        });
    }
    if sniff.is_binary {
        return Some(Refusal::Binary);
    }
    None
}

/// A byte count in the largest unit that keeps it readable.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    match bytes {
        b if b >= GB => format!("{:.1} GB", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1} MB", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1} kB", b as f64 / KB as f64),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sniff(ext: &str, is_binary: bool) -> Sniff {
        Sniff {
            ext: Some(ext.to_owned()),
            len: 10,
            is_binary,
        }
    }

    #[test]
    fn the_extension_table_names_the_right_kind() {
        assert_eq!(kind_of(Some("png")), Some(FileKind::Image));
        assert_eq!(kind_of(Some("PNG")), None, "the table is lowercased first");
        assert_eq!(kind_of(Some("md")), Some(FileKind::Markdown));
        assert_eq!(kind_of(Some("mp4")), Some(FileKind::Video));
        assert_eq!(kind_of(Some("rs")), None, "code is the text viewer's job");
        assert_eq!(kind_of(None), None);
    }

    #[test]
    fn languages_are_mapped_by_extension() {
        assert_eq!(language_for(Some("rs")), "rust");
        assert_eq!(language_for(Some("md")), "markdown");
        assert_eq!(language_for(Some("py")), "python");
        assert_eq!(language_for(Some("toml")), "toml");
        assert_eq!(language_for(Some("txt")), "text");
        assert_eq!(language_for(None), "text");
    }

    #[test]
    fn a_binary_file_is_refused() {
        let refusal = text_refusal(&sniff("txt", true));
        assert_eq!(refusal, Some(Refusal::Binary));
    }

    #[test]
    fn an_oversized_file_is_refused_with_both_sizes() {
        let refusal = text_refusal(&Sniff {
            ext: Some("txt".to_owned()),
            len: crate::workspace::fs_util::MAX_TEXT_BYTES + 1,
            is_binary: false,
        });
        let Some(Refusal::TooLarge { len, limit }) = refusal else {
            panic!("expected a size refusal");
        };
        assert_eq!(len, crate::workspace::fs_util::MAX_TEXT_BYTES + 1);
        assert_eq!(limit, crate::workspace::fs_util::MAX_TEXT_BYTES);
    }

    #[test]
    fn a_plain_small_text_file_is_accepted() {
        assert_eq!(text_refusal(&sniff("txt", false)), None);
    }

    #[test]
    fn sizes_read_as_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 kB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn language_labels_are_readable() {
        assert_eq!(language_label("text"), "Plain Text");
        assert_eq!(language_label("csharp"), "C#");
        assert_eq!(language_label("python"), "Python");
        assert_eq!(language_label("unknown-thing"), "Unknown-thing");
    }
}
