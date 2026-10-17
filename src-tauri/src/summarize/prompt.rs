//! The instructions and inputs sent to the model.

use crate::llm::template::Message;
use crate::store::TranscriptSegment;

/// The built-in instructions for writing a summary. You can replace them in
/// **Settings** (see `summarize::instructions`); `READING_RULES` always follows.
pub const DEFAULT_INSTRUCTIONS: &str = "You write meeting notes after a meeting ends, from its transcript and the note-taker's rough notes. Read the whole transcript before you write anything: the notes should reflect the meeting as a whole, so that someone who missed it could act on them.

Structure:
- Group the notes by topic. Topics appear in the order each first came up. Give each topic a heading that names the specific thing discussed, such as a feature, system, customer, or ticket. Never use a generic heading like \"Discussion\" or \"Updates\".
- Each topic gets one heading, which holds everything said about it, including the details from each time the meeting came back to it.
- When the meeting refers to a ticket by its number, that ticket's heading gives the number and what the ticket is about, not the number alone. Only tickets get numbers in headings, and only the numbers said in the meeting. Tickets that get only a one-line status update go together under one heading that names the sprint or project.
- Under each heading, write bullet points. Use nested bullets for causes, details, and consequences.
- When the note-taker wrote notes, use them as the outline and priorities, and cover every point in them that the meeting discussed. For each point the meeting never discussed, write one bullet with the point exactly as the note-taker wrote it, followed by \"(not discussed)\", and add nothing else about it.
- When someone walks through a procedure, write it as numbered steps under its own topic.
- Combine points only when they say the same thing. Keep routine status updates brief.
- End with \"## Next steps\" as a bulleted list: each task, then the owner's name in parentheses, or (you) for the note-taker. Add context to a task only when the task needs it. Include plans that depend on an outcome, with their condition. Omit the section if there are no tasks.

Personal notes:
- When someone shares something about their life outside work, such as a trip to the Moon or a new pet dragon, add \"## Personal notes\" just before Next steps, with one brief bullet for each. It's the one heading that doesn't name a specific thing discussed.
- Only personal details go there, never work, tasks, or decisions. Leave out greetings and pleasantries. When no one shared anything personal, leave the section out: never write the heading alone or a section that says there's nothing.

Content:
- Keep specifics exactly: ticket and issue numbers, error codes and messages, amounts, dates, and the names of people, systems, products, customers, and settings.
- State what happened or was found, not that it was discussed. Write \"The lunar rover's battery drains overnight\", not \"Discussed an issue with the rover\".
- When something was suggested but not agreed, say so.
- Include reasons, results, rejected options, open questions, and risks when the meeting covered them.
- When a speaker corrects themselves, keep the corrected statement and drop the first one. A corrected number or code still belongs in the notes. Coming back to a topic later is not a correction: what was said each time stays, unless someone says the earlier point was wrong.";

/// How to read the meeting data, added after the instructions in use, custom
/// or default, so custom instructions can't change how the input is read.
/// They continue the default's last list without a heading of their own:
/// a separate "Reading the meeting" section made Qwen3.5 4B write only next
/// steps for long, noisy meetings (#161).
pub const READING_RULES: &str = "- The transcript comes from speech recognition and misspells names. When the note-taker's notes or the meeting title spell a name differently from the transcript, such as \"Lindgren\" in the notes and \"Lindgrin\" in the transcript, always use the notes' spelling.
- Only state what the transcript or notes support. Never invent names, numbers, dates, or decisions.
- \"Me\" is the note-taker. \"Meeting Participants\" is everyone else on the call; use their names when the transcript gives them.
- Attribute each statement and task to whoever said or took it on. A task is yours only when a \"Me\" line takes it on; when a participant says \"I'll do it\", it's theirs.
- Write plain Markdown without backslash escapes, such as $48,000 rather than \\$48,000.
- Reply with the notes only, no preamble.";

const PART_SYSTEM: &str =
    "You condense one part of a longer meeting transcript into factual bullet points that later become detailed meeting notes.

Rules:
- Specifics come first. Keep every specific exactly: ticket and issue numbers, error codes and messages, amounts, dates, and the names of people, systems, products, customers, and settings.
- Keep every decision with its reason, problem with its cause, change made with its result, rejected option, open question, risk, how-to explanation as steps, task with its owner and any condition, and anything someone said about their own life outside work.
- When the note-taker's notes are given, cover every point in them that this part of the transcript discusses.
- Compress routine status updates, such as \"no change\" or \"ready for review next week\", to at most one short bullet per project or person, and put them last. Never let routine updates crowd out specifics.
- State what happened or was found, not that it was discussed.
- When a speaker corrects themselves, keep only the corrected statement.
- Leave out greetings and pleasantries.
- \"Me\" is the note-taker. \"Meeting Participants\" is everyone else on the call.
- Only state what the transcript supports.
- Reply with bullet points only.";

const NEXT_STEPS_SYSTEM: &str = "You list the tasks in finished meeting notes, checking each owner against the meeting.

Rules:
- List every task someone took on in the notes, as one bullet each: the task, starting with its verb, then the owner's name in parentheses, or (you) for the note-taker.
- Take each owner from the transcript: the person who said they'd do it. A task is the note-taker's only when a \"Me\" line takes it on; when a participant says \"I'll do it\", it's theirs. Use the name the transcript gives them.
- \"Me\" and \"you\" are the note-taker. \"Meeting Participants\" is everyone else on the call.
- Include plans that depend on an outcome, with their condition.
- A decision, or a status update on work already under way, is not a task.
- Only list tasks the notes state. Never invent tasks, names, dates, or numbers.
- Reply with the bullets only, no heading. When the notes have no tasks, reply with nothing.";

const TITLE_SYSTEM: &str = "You name meetings from their notes.

Rules:
- If the notes show what the meeting was for, such as a goal stated at the start, name that.
- Otherwise, name the topic the notes cover most: the heading with the most bullets under it, wherever it comes in the notes.
- Reply with that one name only, as a short phrase of at most 15 words.
- Use only what the notes say.
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

/// Asks the model to spell the vocabulary's terms exactly. Empty without terms.
fn terms_section(terms: &[String]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    format!(
        "Spell these names and terms exactly as written here, including when the transcript spells them differently: {}.\n\n",
        terms.join(", ")
    )
}

fn notes_section(notes_markdown: &str) -> String {
    let notes = notes_markdown.trim();
    if notes.is_empty() {
        "The note-taker didn't write any notes. Choose the outline from the transcript.".into()
    } else {
        format!("The note-taker's notes:\n<notes>\n{notes}\n</notes>")
    }
}

/// The system message that writes the notes: the instructions in use, then
/// the reading rules.
fn notes_system(instructions: &str) -> Message {
    Message::system(format!("{}\n{READING_RULES}", instructions.trim()))
}

/// One pass over the whole transcript.
pub fn summary_messages(
    instructions: &str,
    title: &str,
    notes_markdown: &str,
    terms: &[String],
    transcript: &str,
) -> Vec<Message> {
    vec![
        notes_system(instructions),
        Message::user(format!(
            "Meeting: {title}\n\n{}{}\n\nTranscript:\n<transcript>\n{transcript}\n</transcript>",
            terms_section(terms),
            notes_section(notes_markdown)
        )),
    ]
}

/// Condenses one window of a transcript too long for a single pass.
pub fn part_messages(
    part: usize,
    parts: usize,
    notes_markdown: &str,
    terms: &[String],
    transcript: &str,
) -> Vec<Message> {
    let notes = notes_markdown.trim();
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!("The note-taker's notes, which mark what matters:\n<notes>\n{notes}\n</notes>\n\n")
    };
    vec![
        Message::system(PART_SYSTEM),
        Message::user(format!(
            "{}{notes}Part {part} of {parts} of the transcript:\n<transcript>\n{transcript}\n</transcript>",
            terms_section(terms)
        )),
    ]
}

/// The final pass when the transcript was condensed in parts.
pub fn combine_messages(
    instructions: &str,
    title: &str,
    notes_markdown: &str,
    terms: &[String],
    parts: &[String],
) -> Vec<Message> {
    let condensed = condensed(parts);
    vec![
        notes_system(instructions),
        Message::user(format!(
            "Meeting: {title}\n\n{}{}\n\nThe transcript was long, so here it is condensed in order:\n<transcript>\n{condensed}\n</transcript>",
            terms_section(terms),
            notes_section(notes_markdown)
        )),
    ]
}

/// Condensed parts in order, as the final pass reads them.
pub fn condensed(parts: &[String]) -> String {
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| format!("Part {}:\n{}", i + 1, part.trim()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Asks for the tasks in finished notes that left out their Next steps,
/// with the transcript the notes came from, or its condensed parts, so the
/// owners can be checked.
pub fn next_steps_messages(
    summary_markdown: &str,
    transcript: &str,
    was_condensed: bool,
) -> Vec<Message> {
    let label = if was_condensed {
        "The transcript was long, so here it is condensed in order:"
    } else {
        "Transcript:"
    };
    vec![
        Message::system(NEXT_STEPS_SYSTEM),
        Message::user(format!(
            "Meeting notes:\n<notes>\n{}\n</notes>\n\n{label}\n<transcript>\n{}\n</transcript>",
            summary_markdown.trim(),
            transcript.trim()
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
        let messages = summary_messages(
            DEFAULT_INSTRUCTIONS,
            "Design sync",
            "- dark mode first",
            &[],
            "[0:01] Me: Hi",
        );

        assert_eq!(messages[0].role, "system");
        let user = &messages[1].content;
        assert!(user.contains("Meeting: Design sync"));
        assert!(user.contains("<notes>\n- dark mode first\n</notes>"));
        assert!(user.contains("<transcript>\n[0:01] Me: Hi\n</transcript>"));
    }

    #[test]
    fn asks_for_the_vocabulary_spellings_only_when_there_are_terms() {
        // Invented names only.
        let terms = ["Bluefin Ledger".to_owned(), "Corvina".to_owned()];

        let with_terms =
            &summary_messages(DEFAULT_INSTRUCTIONS, "Sync", "", &terms, "[0:01] Me: Hi")[1].content;
        let part = &part_messages(1, 2, "", &terms, "[0:01] Me: Hi")[1].content;
        let combined =
            &combine_messages(DEFAULT_INSTRUCTIONS, "Sync", "", &terms, &["- a".into()])[1].content;
        let without =
            &summary_messages(DEFAULT_INSTRUCTIONS, "Sync", "", &[], "[0:01] Me: Hi")[1].content;

        for content in [with_terms, part, combined] {
            assert!(content.contains("exactly as written here"), "{content}");
            assert!(content.contains("Bluefin Ledger, Corvina."), "{content}");
        }
        assert!(!without.contains("exactly as written"));
    }

    #[test]
    fn keeps_small_talk_at_every_stage() {
        for prompt in [DEFAULT_INSTRUCTIONS, PART_SYSTEM] {
            assert!(prompt.contains("outside work"), "{prompt}");
            assert!(!prompt.to_lowercase().contains("small talk"), "{prompt}");
        }
    }

    #[test]
    fn adds_the_reading_rules_after_custom_or_default_instructions() {
        let custom = "  List decisions only.\n";
        let messages = [
            summary_messages(custom, "Sync", "", &[], "[0:01] Me: Hi"),
            combine_messages(custom, "Sync", "", &[], &["- a".into()]),
        ];
        for messages in &messages {
            assert_eq!(
                messages[0].content,
                format!("List decisions only.\n{READING_RULES}")
            );
        }

        let default = &summary_messages(DEFAULT_INSTRUCTIONS, "Sync", "", &[], "")[0].content;
        assert_eq!(*default, format!("{DEFAULT_INSTRUCTIONS}\n{READING_RULES}"));
    }

    #[test]
    fn continues_the_default_list_with_the_rules_instead_of_a_new_section() {
        let system = &summary_messages(DEFAULT_INSTRUCTIONS, "Sync", "", &[], "")[0].content;

        assert!(
            system.contains(
                "unless someone says the earlier point was wrong.\n- The transcript comes from speech recognition"
            ),
            "{system}"
        );
        assert!(DEFAULT_INSTRUCTIONS.ends_with('.'));
        assert!(READING_RULES.starts_with("- "));
    }

    #[test]
    fn keeps_the_input_rules_out_of_the_replaceable_instructions() {
        for rule in [
            "\"Me\" is the note-taker",
            "Meeting Participants",
            "always use the notes' spelling",
            "Never invent",
            "A task is yours only when",
            "plain Markdown",
            "no preamble",
        ] {
            assert!(READING_RULES.contains(rule), "{rule}");
            assert!(!DEFAULT_INSTRUCTIONS.contains(rule), "{rule}");
        }
    }

    #[test]
    fn says_when_there_are_no_notes() {
        let user = &summary_messages(DEFAULT_INSTRUCTIONS, "Sync", "  \n", &[], "[0:01] Me: Hi")[1]
            .content;

        assert!(user.contains("didn't write any notes"));
        assert!(!user.contains("<notes>"));
    }

    #[test]
    fn combines_condensed_parts_in_order() {
        let user = &combine_messages(
            DEFAULT_INSTRUCTIONS,
            "Sync",
            "",
            &[],
            &["- a".into(), "- b".into()],
        )[1]
        .content;

        assert!(user.contains("Part 1:\n- a\n\nPart 2:\n- b"));
    }

    #[test]
    fn asks_for_next_steps_with_the_notes_and_what_they_came_from() {
        let single = next_steps_messages("  ## Notes\n- a\n", "[0:01] Me: Hi", false);
        let condensed = next_steps_messages("## Notes\n- a", &condensed(&["- b".into()]), true);

        assert!(single[0].content.starts_with("You list the tasks"));
        assert_eq!(
            single[1].content,
            "Meeting notes:\n<notes>\n## Notes\n- a\n</notes>\n\nTranscript:\n<transcript>\n[0:01] Me: Hi\n</transcript>"
        );
        assert!(
            condensed[1]
                .content
                .ends_with("condensed in order:\n<transcript>\nPart 1:\n- b\n</transcript>"),
            "{}",
            condensed[1].content
        );
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

    #[test]
    fn gives_each_part_the_notes_when_there_are_any() {
        let with_notes =
            &part_messages(1, 3, "- budget\n- move date", &[], "[0:01] Me: Hi")[1].content;
        let without = &part_messages(1, 3, "  \n", &[], "[0:01] Me: Hi")[1].content;

        assert!(
            with_notes.contains("<notes>\n- budget\n- move date\n</notes>\n\nPart 1 of 3"),
            "{with_notes}"
        );
        assert!(!without.contains("<notes>"), "{without}");
    }
}
