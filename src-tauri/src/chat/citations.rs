//! Finds the source labels an answer cites, such as `[S3]` or `[S3, N1]`.
//!
//! The prompt labels each transcript segment `S<n>` and each notes passage
//! `N<n>`. Answers cite them in square brackets. Only labels that were sent to
//! the model become citations; anything else is dropped.

use std::ops::Range;

/// Longest bracketed group considered a citation, in bytes.
const MAX_GROUP_BYTES: usize = 80;

/// One bracketed group of labels and where it is in the text.
#[derive(Debug, PartialEq)]
struct Group<'a> {
    range: Range<usize>,
    labels: Vec<&'a str>,
}

/// Whether `label` looks like `S12` or `N3`.
fn is_label(label: &str) -> bool {
    let mut chars = label.chars();
    matches!(chars.next(), Some('S' | 'N'))
        && !chars.as_str().is_empty()
        && chars.all(|c| c.is_ascii_digit())
}

fn groups(text: &str) -> Vec<Group<'_>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('[') {
        let start = from + offset;
        let rest = &text[start + 1..];
        let Some(len) = rest
            .find(']')
            .filter(|len| *len <= MAX_GROUP_BYTES && !rest[..*len].contains('['))
        else {
            from = start + 1;
            continue;
        };
        let labels: Vec<&str> = rest[..len].split([',', ';']).map(str::trim).collect();
        let end = start + 1 + len + 1;
        if labels.iter().all(|l| is_label(l)) {
            found.push(Group {
                range: start..end,
                labels,
            });
        }
        from = end;
    }
    found
}

/// Every label the text cites, in order of first appearance, without repeats.
pub fn cited_labels(text: &str) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for label in groups(text).into_iter().flat_map(|g| g.labels) {
        if !labels.iter().any(|l| l == label) {
            labels.push(label.to_owned());
        }
    }
    labels
}

/// Rewrites each citation group with `name`, which returns how to show a
/// label, such as `12:05`, or `None` to drop it. A group left with nothing is
/// removed with the space before it. Repeated names in a group are shown once.
pub fn rewrite(text: &str, name: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for group in groups(text) {
        let mut shown: Vec<String> = Vec::new();
        for label in group.labels {
            if let Some(n) = name(label).filter(|n| !shown.contains(n)) {
                shown.push(n);
            }
        }
        out.push_str(&text[copied..group.range.start]);
        if shown.is_empty() {
            let trimmed = out.trim_end_matches(' ').len();
            out.truncate(trimmed);
        } else {
            out.push('[');
            out.push_str(&shown.join(", "));
            out.push(']');
        }
        copied = group.range.end;
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_single_and_grouped_labels_once_each() {
        let text = "Launch moves to May 4 [S3]. Dana owns it [S3, N1][S7; S8].";

        assert_eq!(cited_labels(text), ["S3", "N1", "S7", "S8"]);
    }

    #[test]
    fn ignores_brackets_that_are_not_citations() {
        let text = "See [the doc] and [x] and [S] and [S1a] and [1] and [s2] and [[S4]] and [S5";

        assert_eq!(cited_labels(text), ["S4"]);
    }

    #[test]
    fn ignores_an_unclosed_bracket_far_from_its_end() {
        let text = format!("[{} S1]", "word ".repeat(30));

        assert!(cited_labels(&text).is_empty());
    }

    #[test]
    fn rewrites_labels_and_drops_unknown_ones_with_their_space() {
        let name = |label: &str| match label {
            "S1" => Some("0:05".to_owned()),
            "S2" => Some("0:05".to_owned()),
            "N1" => Some("My notes".to_owned()),
            _ => None,
        };

        assert_eq!(
            rewrite(
                "Ship Friday [S1, N1]. Maybe later [S9]. Agreed [S1, S2].",
                name
            ),
            "Ship Friday [0:05, My notes]. Maybe later. Agreed [0:05]."
        );
    }

    #[test]
    fn handles_text_without_citations_and_non_ascii_text() {
        let name = |_: &str| Some("x".to_owned());

        assert_eq!(rewrite("Café, no sources.", name), "Café, no sources.");
        assert_eq!(rewrite("Café [S1] ✓", name), "Café [x] ✓");
    }
}
