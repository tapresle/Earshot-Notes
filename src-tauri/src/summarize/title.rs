//! Names an untitled meeting from its finished summary.

use std::sync::atomic::AtomicBool;

use super::prompt::title_messages;
use crate::llm::generator::{GenerateError, Generator};

/// Room for the reply. A title of 15 words needs about half.
const MAX_TITLE_TOKENS: usize = 48;
/// Longest generated title kept, in characters.
const MAX_TITLE_CHARS: usize = 120;
/// Quotes and Markdown emphasis that models wrap titles in.
const WRAPPERS: &[char] = &[
    '"', '\'', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '*', '_', '`', '#',
];
const TRAILING_PUNCTUATION: &[char] = &['.', ',', ';', ':', '!'];

/// Asks the model for a short title for the summary. Returns `None` when the
/// reply has nothing usable in it.
pub fn suggest_title(
    generator: &mut dyn Generator,
    summary_markdown: &str,
    cancel: &AtomicBool,
) -> Result<Option<String>, GenerateError> {
    let reply = generator.generate(
        &title_messages(summary_markdown),
        MAX_TITLE_TOKENS,
        cancel,
        &mut |_| {},
    )?;
    Ok(clean_title(&reply))
}

/// Turns a model reply into a plain title: its first line, without Markdown
/// markers, a "Title:" label, quotes, or trailing punctuation, and cut at a word
/// boundary when it's too long.
pub fn clean_title(reply: &str) -> Option<String> {
    let line = reply.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(|c: char| c.is_whitespace() || "#->*".contains(c));
    let title = truncate(trim_edges(strip_label(line)));
    (!title.is_empty()).then(|| title.to_owned())
}

fn strip_label(line: &str) -> &str {
    match line.get(..6) {
        Some(label) if label.eq_ignore_ascii_case("title:") => &line[6..],
        _ => line,
    }
}

/// Removes wrapping quotes, emphasis, whitespace, and trailing punctuation,
/// repeating until none are left, since models nest them (`"**Title**."`).
fn trim_edges(mut text: &str) -> &str {
    loop {
        let next = text
            .trim_matches(|c: char| c.is_whitespace() || WRAPPERS.contains(&c))
            .trim_end_matches(TRAILING_PUNCTUATION);
        if next == text {
            return text;
        }
        text = next;
    }
}

fn truncate(title: &str) -> &str {
    let Some((cut, _)) = title.char_indices().nth(MAX_TITLE_CHARS) else {
        return title;
    };
    let head = &title[..cut];
    let head = match head.rfind(char::is_whitespace) {
        Some(space) if space > 0 => &head[..space],
        _ => head,
    };
    trim_edges(head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::template::Message;

    #[test]
    fn keeps_a_plain_title() {
        assert_eq!(
            clean_title("Budget review").as_deref(),
            Some("Budget review")
        );
    }

    #[test]
    fn strips_quotes() {
        assert_eq!(
            clean_title("\"Budget review\"").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("\u{201C}Budget review\u{201D}").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("'Budget review'").as_deref(),
            Some("Budget review")
        );
    }

    #[test]
    fn strips_markdown_markers() {
        assert_eq!(
            clean_title("# Budget review").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("**Budget review**").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("- Budget review").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("**Title:** Budget review").as_deref(),
            Some("Budget review")
        );
    }

    #[test]
    fn strips_trailing_punctuation() {
        assert_eq!(
            clean_title("Budget review.").as_deref(),
            Some("Budget review")
        );
        assert_eq!(
            clean_title("\"Budget review\".").as_deref(),
            Some("Budget review")
        );
        assert_eq!(clean_title("Ship it!").as_deref(), Some("Ship it"));
        assert_eq!(
            clean_title("Should we ship Friday?").as_deref(),
            Some("Should we ship Friday?")
        );
    }

    #[test]
    fn keeps_only_the_first_line() {
        assert_eq!(
            clean_title("\n\nBudget review\nThis meeting covered the budget.").as_deref(),
            Some("Budget review")
        );
    }

    #[test]
    fn keeps_a_fifteen_word_title() {
        let reply = "Quarterly planning for the partner portal launch, the pricing page refresh, and Q1 support staffing";

        assert_eq!(clean_title(reply).as_deref(), Some(reply));
    }

    #[test]
    fn cuts_an_overlong_reply_at_a_word() {
        let reply = "word ".repeat(40);

        let title = clean_title(&reply).unwrap();

        assert!(title.chars().count() <= MAX_TITLE_CHARS, "{title}");
        assert!(title.ends_with("word"), "{title}");
    }

    #[test]
    fn has_nothing_for_an_empty_reply() {
        assert_eq!(clean_title(""), None);
        assert_eq!(clean_title("  \n \n"), None);
        assert_eq!(clean_title("\"\"."), None);
        assert_eq!(clean_title("Title:"), None);
    }

    struct Replies {
        reply: &'static str,
        prompts: Vec<String>,
    }

    impl Generator for Replies {
        fn context_size(&self) -> usize {
            8192
        }
        fn prompt_tokens(&mut self, _: &[Message]) -> Result<usize, GenerateError> {
            Ok(0)
        }
        fn count_tokens(&mut self, _: &str) -> Result<usize, GenerateError> {
            Ok(0)
        }
        fn generate(
            &mut self,
            messages: &[Message],
            _: usize,
            _: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<String, GenerateError> {
            self.prompts.push(messages[1].content.clone());
            Ok(self.reply.into())
        }
    }

    #[test]
    fn names_the_meeting_from_the_summary() {
        let mut model = Replies {
            reply: "\"Q3 budget review.\"",
            prompts: Vec::new(),
        };

        let title = suggest_title(
            &mut model,
            "## Budget\n- Cut travel",
            &AtomicBool::new(false),
        );

        assert_eq!(title.unwrap().as_deref(), Some("Q3 budget review"));
        assert!(model.prompts[0].contains("<notes>\n## Budget\n- Cut travel\n</notes>"));
    }
}
