//! Splits **My notes** into passages: paragraphs, list items, and headings.
//! Each passage is a slice of the notes, so the UI can find it again.

/// The notes' passages, in order. A list item keeps its indented continuation
/// lines; a nested item is a passage of its own.
pub fn passages(notes: &str) -> Vec<&str> {
    let mut found = Vec::new();
    // Byte range of the passage being collected.
    let mut current: Option<(usize, usize)> = None;
    let mut offset = 0;
    for line in notes.split_inclusive('\n') {
        let (start, end) = (offset, offset + line.trim_end().len());
        offset += line.len();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            push(notes, current.take(), &mut found);
        } else if is_heading(trimmed) {
            push(notes, current.take(), &mut found);
            push(notes, Some((start, end)), &mut found);
        } else if is_list_item(trimmed) {
            push(notes, current.take(), &mut found);
            current = Some((start, end));
        } else {
            // A paragraph line, or a list item's continuation.
            current = Some(current.map_or((start, end), |(s, _)| (s, end)));
        }
    }
    push(notes, current, &mut found);
    found
}

fn push<'a>(notes: &'a str, range: Option<(usize, usize)>, found: &mut Vec<&'a str>) {
    if let Some((start, end)) = range {
        let text = notes[start..end].trim();
        if !text.is_empty() {
            found.push(text);
        }
    }
}

fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

fn is_list_item(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return rest.starts_with(' ');
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && line[digits..]
            .strip_prefix(['.', ')'])
            .is_some_and(|rest| rest.starts_with(' '))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paragraphs_list_items_and_headings() {
        let notes = "## Launch\nMoving to May 4\nbecause of QA.\n\n- Dana: release notes\n  by Friday\n  - check the date\n1. Draft\n2) Send\n\nClosing thought";

        assert_eq!(
            passages(notes),
            [
                "## Launch",
                "Moving to May 4\nbecause of QA.",
                "- Dana: release notes\n  by Friday",
                "- check the date",
                "1. Draft",
                "2) Send",
                "Closing thought",
            ]
        );
    }

    #[test]
    fn returns_slices_of_the_notes() {
        let notes = "  - first\r\n\n- second  \n";

        let found = passages(notes);

        assert_eq!(found, ["- first", "- second"]);
        for passage in found {
            assert!(notes.contains(passage));
        }
    }

    #[test]
    fn has_none_for_empty_notes() {
        assert!(passages("").is_empty());
        assert!(passages(" \n\n  ").is_empty());
    }

    #[test]
    fn treats_markers_without_a_space_as_text() {
        assert_eq!(passages("#hashtag\n-5 degrees"), ["#hashtag\n-5 degrees"]);
    }
}
