//! Writes "you" where the model calls the note-taker "Me" (#100).
//!
//! The transcript labels the note-taker's lines "Me", and models copy the
//! label as a name: "Me suggested a fix", "(Me)" as a task's owner. Asking the
//! model to write "you" instead made it credit participants' tasks to the
//! note-taker, so the word is replaced in its output. Works on streamed text:
//! only a word that could still become "Me" is held back.

/// Replaces the whole word "Me" with "you", or "You" where a sentence, line,
/// or list item starts, and the owner tag "(me)" with "(you)".
#[derive(Debug)]
pub struct NoteTaker {
    /// The start of a word that could still be "Me", held until it's complete.
    held: String,
    /// Inside a word that can no longer be "Me", already sent on.
    passing: bool,
    /// Whether the next word starts a sentence, line, or list item.
    at_start: bool,
    /// Whether the word in progress follows "(".
    after_paren: bool,
}

impl Default for NoteTaker {
    fn default() -> Self {
        Self {
            held: String::new(),
            passing: false,
            at_start: true,
            after_paren: false,
        }
    }
}

impl NoteTaker {
    /// Returns the text that can be shown so far.
    pub fn push(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            if c.is_alphanumeric() || c == '\'' {
                if self.passing {
                    out.push(c);
                } else {
                    self.held.push(c);
                    let held = self.held.as_str();
                    let could_be_me =
                        "Me".starts_with(held) || (self.after_paren && "me".starts_with(held));
                    if !could_be_me {
                        out.push_str(&self.held);
                        self.held.clear();
                        self.passing = true;
                    }
                }
                continue;
            }
            self.end_word(&mut out, Some(c));
            self.after_paren = c == '(';
            match c {
                '\n' | '.' | '!' | '?' | ':' => self.at_start = true,
                // Markdown markers and spaces leave a line's start in place.
                c if c.is_whitespace() || matches!(c, '*' | '-' | '#' | '>') => {}
                _ => self.at_start = false,
            }
            out.push(c);
        }
        out
    }

    /// Returns the rest once the text has ended.
    pub fn finish(mut self) -> String {
        let mut out = String::new();
        self.end_word(&mut out, None);
        out
    }

    /// Ends the word in progress at `next`, or at the end of the text.
    fn end_word(&mut self, out: &mut String, next: Option<char>) {
        if self.held.is_empty() && !self.passing {
            return;
        }
        match self.held.as_str() {
            "Me" if self.at_start => out.push_str("You"),
            "Me" => out.push_str("you"),
            "me" if self.after_paren && next == Some(')') => out.push_str("you"),
            held => out.push_str(held),
        }
        self.held.clear();
        self.passing = false;
        self.at_start = false;
    }
}

/// [`NoteTaker`] over a whole text.
pub fn you_for_me(text: &str) -> String {
    let mut note_taker = NoteTaker::default();
    let mut out = note_taker.push(text);
    out.push_str(&note_taker.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_you_for_me_as_a_name() {
        assert_eq!(
            you_for_me("Documentation shared by Me was missing a field."),
            "Documentation shared by you was missing a field."
        );
        assert_eq!(
            you_for_me("* Sign the lease by Friday (Me)."),
            "* Sign the lease by Friday (you)."
        );
    }

    #[test]
    fn capitalizes_at_the_start_of_a_sentence_line_or_list_item() {
        assert_eq!(
            you_for_me("## Next steps\n*   Me will write the alert. Me asked why."),
            "## Next steps\n*   You will write the alert. You asked why."
        );
        assert_eq!(you_for_me("1. Me to check"), "1. You to check");
    }

    #[test]
    fn writes_you_for_the_owner_tag_me() {
        assert_eq!(
            you_for_me("Me (me) will sign the lease by Friday."),
            "You (you) will sign the lease by Friday."
        );
    }

    #[test]
    fn leaves_other_words_alone() {
        let text = "Meeting Participants asked me about Mel's memo (meeting notes). ME ONLY (me";
        assert_eq!(you_for_me(text), text);
    }

    #[test]
    fn gives_the_same_text_however_it_streams() {
        let text = "## Billing\n* Me noted the 502 error.\n* Owner (Me)";
        let mut note_taker = NoteTaker::default();
        let mut streamed: String = text
            .chars()
            .map(|c| note_taker.push(&c.to_string()))
            .collect();
        streamed.push_str(&note_taker.finish());

        assert_eq!(streamed, you_for_me(text));
        assert_eq!(
            streamed,
            "## Billing\n* You noted the 502 error.\n* Owner (you)"
        );
    }

    #[test]
    fn holds_back_only_a_word_that_could_still_be_me() {
        let mut note_taker = NoteTaker::default();

        assert_eq!(note_taker.push("Shared by M"), "Shared by ");
        assert_eq!(note_taker.push("e."), "you.");
        assert_eq!(note_taker.push("## No"), "## No");
        assert_eq!(note_taker.push("tes by Me"), "tes by ");
        assert_eq!(note_taker.push("l"), "Mel");
        assert_eq!(note_taker.finish(), "");
    }
}
