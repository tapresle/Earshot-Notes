//! The instructions and inputs sent to the model.

use super::template::Message;
use crate::store::TranscriptSegment;

const SYSTEM: &str = "You write meeting notes from a transcript and the note-taker's own rough notes.

Rules:
- Use the note-taker's notes as the outline and priorities. Expand them with details from the transcript.
- Write Markdown: short section headings and concise bullet points.
- End with \"## Action items\" listing each task with its owner when the transcript names one. Omit the section if there are none.
- Only state what the transcript or notes support. Never invent names, numbers, dates, or decisions.
- \"Me\" is the note-taker. \"Meeting Participants\" is everyone else on the call.
- Reply with the notes only, no preamble.";

const PART_SYSTEM: &str =
    "You condense one part of a longer meeting transcript into factual bullet points.

Rules:
- Keep every decision, number, date, name, open question, and task with its owner.
- \"Me\" is the note-taker. \"Meeting Participants\" is everyone else on the call.
- Only state what the transcript supports.
- Reply with bullet points only.";

const TITLE_SYSTEM: &str = "You name meetings from their notes.

Rules:
- Reply with a title of at most 6 words, and nothing else.
- Name the main topic, using only what the notes say.
- No quotes, no Markdown, and no punctuation at the end.";

/// Formats one transcript line, such as `[12:05] Meeting Participants: Let's ship it.`
pub fn transcript_line(segment: &TranscriptSegment) -> String {
    let speaker = segment.source.label();
    let seconds = segment.start_ms.max(0) / 1000;
    let stamp = if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    };
    format!("[{stamp}] {speaker}: {}", segment.text)
}

fn notes_section(notes_markdown: &str) -> String {
    let notes = notes_markdown.trim();
    if notes.is_empty() {
        "The note-taker didn't write any notes. Choose the outline from the transcript.".into()
    } else {
        format!("The note-taker's notes:\n<notes>\n{notes}\n</notes>")
    }
}

/// One pass over the whole transcript.
pub fn summary_messages(title: &str, notes_markdown: &str, transcript: &str) -> Vec<Message> {
    vec![
        Message::system(SYSTEM),
        Message::user(format!(
            "Meeting: {title}\n\n{}\n\nTranscript:\n<transcript>\n{transcript}\n</transcript>",
            notes_section(notes_markdown)
        )),
    ]
}

/// Condenses one window of a transcript too long for a single pass.
pub fn part_messages(part: usize, parts: usize, transcript: &str) -> Vec<Message> {
    vec![
        Message::system(PART_SYSTEM),
        Message::user(format!(
            "Part {part} of {parts} of the transcript:\n<transcript>\n{transcript}\n</transcript>"
        )),
    ]
}

/// The final pass when the transcript was condensed in parts.
pub fn combine_messages(title: &str, notes_markdown: &str, parts: &[String]) -> Vec<Message> {
    let condensed = parts
        .iter()
        .enumerate()
        .map(|(i, part)| format!("Part {}:\n{}", i + 1, part.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    vec![
        Message::system(SYSTEM),
        Message::user(format!(
            "Meeting: {title}\n\n{}\n\nThe transcript was long, so here it is condensed in order:\n<transcript>\n{condensed}\n</transcript>",
            notes_section(notes_markdown)
        )),
    ]
}

/// Asks for a short title for a finished summary.
pub fn title_messages(summary_markdown: &str) -> Vec<Message> {
    vec![
        Message::system(TITLE_SYSTEM),
        Message::user(format!(
            "Meeting notes:\n<notes>\n{}\n</notes>",
            summary_markdown.trim()
        )),
    ]
}

/// Groups lines into windows whose token count stays within `budget`, given each
/// line's token count. A single line over budget gets a window of its own.
pub fn windows(lines: &[String], costs: &[usize], budget: usize) -> Vec<String> {
    let mut windows = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for (line, cost) in lines.iter().zip(costs) {
        let cost = cost + 1;
        if used + cost > budget && !current.is_empty() {
            windows.push(std::mem::take(&mut current));
            used = 0;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
        used += cost;
    }
    if !current.is_empty() {
        windows.push(current);
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;

    fn segment(source: Source, start_ms: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: 1,
            meeting_id: 1,
            source,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.into(),
            language: None,
        }
    }

    #[test]
    fn formats_transcript_lines_with_speaker_and_time() {
        assert_eq!(
            transcript_line(&segment(Source::Them, 725_000, "Ship it.")),
            "[12:05] Meeting Participants: Ship it."
        );
        assert_eq!(
            transcript_line(&segment(Source::Me, 3_725_000, "Agreed.")),
            "[1:02:05] Me: Agreed."
        );
    }

    #[test]
    fn includes_notes_title_and_transcript() {
        let messages = summary_messages("Design sync", "- dark mode first", "[0:01] Me: Hi");

        assert_eq!(messages[0].role, "system");
        let user = &messages[1].content;
        assert!(user.contains("Meeting: Design sync"));
        assert!(user.contains("<notes>\n- dark mode first\n</notes>"));
        assert!(user.contains("<transcript>\n[0:01] Me: Hi\n</transcript>"));
    }

    #[test]
    fn says_when_there_are_no_notes() {
        let user = &summary_messages("Sync", "  \n", "[0:01] Me: Hi")[1].content;

        assert!(user.contains("didn't write any notes"));
        assert!(!user.contains("<notes>"));
    }

    #[test]
    fn combines_condensed_parts_in_order() {
        let user = &combine_messages("Sync", "", &["- a".into(), "- b".into()])[1].content;

        assert!(user.contains("Part 1:\n- a\n\nPart 2:\n- b"));
    }

    #[test]
    fn splits_lines_into_windows_within_budget() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();

        // Each line costs 2 + 1 for the newline, so 3 lines fit in 9.
        let windows = windows(&lines, &[2; 10], 9);

        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0], "line 0\nline 1\nline 2");
        assert_eq!(windows[3], "line 9");
    }

    #[test]
    fn gives_an_oversized_line_its_own_window() {
        let lines = vec!["short".to_string(), "huge".to_string(), "short".to_string()];

        let windows = windows(&lines, &[1, 50, 1], 5);

        assert_eq!(windows, vec!["short", "huge", "short"]);
    }
}
