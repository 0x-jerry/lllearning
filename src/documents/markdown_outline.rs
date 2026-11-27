//! Markdown's outline: its ATX headings, with the line each sits on. It lives
//! with the viewer because the outline is a fact about this format.

use super::OutlineEntry;

/// Every ATX heading in `text`, in document order. A heading inside a fenced code
/// block is a comment, not a heading.
pub fn headings(text: &str) -> Vec<OutlineEntry> {
    let mut entries = Vec::new();
    let mut fence: Option<&str> = None;

    for (ix, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = fence_marker(trimmed) {
            fence = Some(open);
            continue;
        }
        if let Some((depth, label)) = heading(trimmed) {
            entries.push(OutlineEntry {
                label,
                depth,
                line: ix as u32 + 1,
            });
        }
    }
    entries
}

/// The fence marker a line opens, if it opens one.
fn fence_marker(line: &str) -> Option<&'static str> {
    ["```", "~~~"]
        .into_iter()
        .find(|marker| line.starts_with(marker))
}

/// A heading's depth and label, without the hashes.
fn heading(line: &str) -> Option<(usize, String)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    let label = rest.trim_end_matches('#').trim();
    if label.is_empty() {
        return None;
    }
    Some((hashes - 1, label.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(text: &str) -> Vec<(usize, String, u32)> {
        headings(text)
            .into_iter()
            .map(|entry| (entry.depth, entry.label, entry.line))
            .collect()
    }

    #[test]
    fn headings_become_nested_entries_with_their_lines() {
        let text = "# Title\n\nSome prose.\n\n## Section\n\n### Deeper\n";
        assert_eq!(
            labels(text),
            vec![
                (0, "Title".to_owned(), 1),
                (1, "Section".to_owned(), 5),
                (2, "Deeper".to_owned(), 7),
            ]
        );
    }

    #[test]
    fn a_heading_inside_a_fence_is_not_a_heading() {
        let text = "# Real\n\n```bash\n# a shell comment\n```\n\n## Also real\n";
        assert_eq!(
            labels(text),
            vec![(0, "Real".to_owned(), 1), (1, "Also real".to_owned(), 7)],
            "a `#` in a code block is a comment, not a heading"
        );
    }

    #[test]
    fn an_unclosed_fence_swallows_the_rest() {
        let text = "# Real\n```\n# swallowed\n";
        assert_eq!(labels(text), vec![(0, "Real".to_owned(), 1)]);
    }

    #[test]
    fn a_heading_without_text_is_ignored() {
        assert!(headings("#\n##   \n").is_empty());
    }

    #[test]
    fn more_than_six_hashes_is_not_a_heading() {
        assert!(headings("####### Too deep\n").is_empty());
    }

    #[test]
    fn prose_has_no_outline() {
        assert!(headings("just some words\nand more words\n").is_empty());
    }
}
