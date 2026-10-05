//! Renders meetings as Markdown files: summary first, then notes, then transcript.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::store::ExportMeeting;

const MAX_SLUG_CHARS: usize = 60;

/// Renders one meeting. Times use `tz`, normally the system time zone.
pub fn render_meeting(meeting: &ExportMeeting, tz: &TimeZone) -> String {
    let m = &meeting.meeting;
    let mut out = String::new();

    out.push_str("---\n");
    out.push_str(&format!("title: \"{}\"\n", yaml_escape(&m.title)));
    out.push_str(&format!(
        "date: {}\n",
        local(m.started_at.unwrap_or(m.created_at), tz).strftime("%Y-%m-%dT%H:%M:%S%:z")
    ));
    if let (Some(start), Some(end)) = (m.started_at, m.ended_at) {
        out.push_str(&format!("duration: {}\n", duration(end - start)));
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", m.title));

    out.push_str("## Summary\n\n");
    match &meeting.summary {
        Some(summary) => {
            if !summary.is_final {
                out.push_str(&format!(
                    "_Partial summary, up to {} into the meeting._\n\n",
                    clock(summary.upto_ms)
                ));
            }
            out.push_str(&demote_headings(summary.body_markdown.trim()));
            out.push_str("\n\n");
        }
        None => out.push_str("_No summary yet._\n\n"),
    }

    out.push_str("## My notes\n\n");
    match meeting.notes_markdown.trim() {
        "" => out.push_str("_No notes._\n\n"),
        notes => {
            out.push_str(&demote_headings(notes));
            out.push_str("\n\n");
        }
    }

    out.push_str("## Transcript\n\n");
    if meeting.segments.is_empty() {
        out.push_str("_No transcript._\n");
    }
    for segment in &meeting.segments {
        out.push_str(&format!(
            "**{}** [{}] {}\n",
            segment.source.label(),
            clock(segment.start_ms),
            segment.text.trim()
        ));
    }
    out
}

/// `YYYY-MM-DD-title-slug.md`, dated in local time.
pub fn file_name(meeting: &ExportMeeting, tz: &TimeZone) -> String {
    let m = &meeting.meeting;
    format!(
        "{}-{}.md",
        local(m.started_at.unwrap_or(m.created_at), tz).strftime("%Y-%m-%d"),
        slug(&m.title)
    )
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFailure {
    pub title: String,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub written: Vec<String>,
    pub failed: Vec<ExportFailure>,
}

/// Writes one file per meeting into `dir`. Existing files are never
/// overwritten: a name that's taken gets a numeric suffix. A failure on one
/// meeting is reported and the rest are still written.
pub fn write_meetings(meetings: &[ExportMeeting], dir: &Path, tz: &TimeZone) -> ExportReport {
    let mut report = ExportReport::default();
    for meeting in meetings {
        let result = write_new(
            &dir.join(file_name(meeting, tz)),
            &render_meeting(meeting, tz),
        );
        match result {
            Ok(path) => report.written.push(path.to_string_lossy().into_owned()),
            Err(error) => report.failed.push(ExportFailure {
                title: meeting.meeting.title.clone(),
                message: error.to_string(),
            }),
        }
    }
    report
}

/// Writes one meeting to the exact path the user chose, replacing any file there.
pub fn write_meeting(meeting: &ExportMeeting, path: &Path, tz: &TimeZone) -> io::Result<()> {
    std::fs::write(path, render_meeting(meeting, tz))
}

/// Creates `path`, or the first free `name-2.md`, `name-3.md`, and so on.
fn write_new(path: &Path, contents: &str) -> io::Result<PathBuf> {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let candidates = std::iter::once(path.to_path_buf())
        .chain((2..).map(|n| path.with_file_name(format!("{stem}-{n}.md"))));
    for candidate in candidates.take(1000) {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(contents.as_bytes())?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("too many files with the same name"))
}

fn local(ms: i64, tz: &TimeZone) -> jiff::Zoned {
    Timestamp::from_millisecond(ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(tz.clone())
}

/// `hh:mm:ss` from a meeting offset.
fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// `42m` or `1h 05m`, rounded to the minute.
fn duration(ms: i64) -> String {
    let minutes = (ms.max(0) + 30_000) / 60_000;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h {:02}m", minutes / 60, minutes % 60)
    }
}

fn yaml_escape(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Lowercase ASCII words joined by hyphens, at most 60 characters.
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug: String = slug.chars().take(MAX_SLUG_CHARS).collect();
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "meeting".into()
    } else {
        slug
    }
}

/// Moves headings down one level so they nest under the file's `##` sections.
/// Lines inside fenced code blocks are left alone.
fn demote_headings(markdown: &str) -> String {
    let mut in_fence = false;
    markdown
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
            }
            let hashes = line.chars().take_while(|&c| c == '#').count();
            let is_heading =
                !in_fence && (1..=6).contains(&hashes) && line[hashes..].starts_with(' ');
            if is_heading && hashes < 6 {
                format!("#{line}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Meeting, Source, Summary, TranscriptSegment};

    // 2026-10-03 14:00:00 in New York (UTC-4 in October).
    const START: i64 = 1_791_050_400_000;

    fn new_york() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    fn meeting() -> ExportMeeting {
        ExportMeeting {
            meeting: Meeting {
                id: 1,
                title: "Design sync".into(),
                created_at: START - 60_000,
                started_at: Some(START),
                ended_at: Some(START + 42 * 60_000),
            },
            notes_markdown: "- dark mode first\n".into(),
            segments: vec![
                segment(
                    Source::Me,
                    1_378_000,
                    "Let's lock the onboarding copy today.",
                ),
                segment(
                    Source::Them,
                    1_382_000,
                    "Agreed, I'll send the final strings tonight.",
                ),
            ],
            summary: Some(summary("## Decisions\n- Ship dark mode first", true)),
        }
    }

    fn segment(source: Source, start_ms: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: start_ms,
            meeting_id: 1,
            source,
            start_ms,
            end_ms: start_ms + 3000,
            text: text.into(),
            language: Some("en".into()),
        }
    }

    fn summary(body: &str, is_final: bool) -> Summary {
        Summary {
            id: 1,
            meeting_id: 1,
            upto_ms: 754_000,
            is_final,
            body_markdown: body.into(),
            model_id: "gemma".into(),
            created_at: START,
        }
    }

    #[test]
    fn renders_summary_then_notes_then_transcript() {
        assert_eq!(
            render_meeting(&meeting(), &new_york()),
            "---
title: \"Design sync\"
date: 2026-10-03T14:00:00-04:00
duration: 42m
---

# Design sync

## Summary

### Decisions
- Ship dark mode first

## My notes

- dark mode first

## Transcript

**Me** [00:22:58] Let's lock the onboarding copy today.
**Meeting Participants** [00:23:02] Agreed, I'll send the final strings tonight.
"
        );
    }

    #[test]
    fn marks_a_partial_summary() {
        let mut m = meeting();
        m.summary = Some(summary("- so far", false));

        let out = render_meeting(&m, &new_york());

        assert!(out.contains(
            "## Summary\n\n_Partial summary, up to 00:12:34 into the meeting._\n\n- so far\n"
        ));
    }

    #[test]
    fn keeps_the_same_shape_when_sections_are_empty() {
        let mut m = meeting();
        m.summary = None;
        m.notes_markdown = "  \n".into();
        m.segments.clear();
        m.meeting.started_at = None;
        m.meeting.ended_at = None;

        let out = render_meeting(&m, &new_york());

        assert!(out.contains("## Summary\n\n_No summary yet._\n\n## My notes\n\n_No notes._\n\n## Transcript\n\n_No transcript._\n"));
        assert!(!out.contains("duration:"));
        // Unrecorded meetings are dated by when they were created.
        assert!(out.contains("date: 2026-10-03T13:59:00-04:00"));
    }

    #[test]
    fn uses_the_offset_in_effect_on_the_meeting_date() {
        let mut m = meeting();
        m.meeting.started_at = Some(START - 120 * 24 * 3_600_000); // early June, still daylight time
        let january = START + 100 * 24 * 3_600_000; // mid January, standard time

        assert!(render_meeting(&m, &new_york()).contains("-04:00"));
        m.meeting.started_at = Some(january);
        assert!(render_meeting(&m, &new_york()).contains("-05:00"));
    }

    #[test]
    fn escapes_the_title_in_front_matter() {
        let mut m = meeting();
        m.meeting.title = "Q4: \"big\" plan\\next".into();

        assert!(
            render_meeting(&m, &new_york()).contains("title: \"Q4: \\\"big\\\" plan\\\\next\"\n")
        );
    }

    #[test]
    fn formats_long_durations_with_hours() {
        assert_eq!(duration(65 * 60_000), "1h 05m");
        assert_eq!(duration(29_000), "0m");
        assert_eq!(duration(90_000), "2m");
    }

    #[test]
    fn demotes_headings_but_not_code_or_tags() {
        let input = "# Top\n#hashtag\n```\n# comment\n```\n###### Deepest";

        assert_eq!(
            demote_headings(input),
            "## Top\n#hashtag\n```\n# comment\n```\n###### Deepest"
        );
    }

    #[test]
    fn builds_file_names_from_the_date_and_title() {
        assert_eq!(
            file_name(&meeting(), &new_york()),
            "2026-10-03-design-sync.md"
        );
        assert_eq!(
            slug("  Q4 Planning: Budget & Hiring!! "),
            "q4-planning-budget-hiring"
        );
        assert_eq!(slug("会議"), "meeting");
        assert_eq!(slug(&"word ".repeat(30)).len(), 59);
    }

    #[test]
    fn writes_one_file_per_meeting_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2026-10-03-design-sync.md"), "keep me").unwrap();
        let meetings = vec![meeting(), meeting()];

        let report = write_meetings(&meetings, dir.path(), &new_york());

        assert!(report.failed.is_empty(), "{report:?}");
        let names: Vec<String> = report
            .written
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            vec!["2026-10-03-design-sync-2.md", "2026-10-03-design-sync-3.md"]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("2026-10-03-design-sync.md")).unwrap(),
            "keep me"
        );
        assert!(
            std::fs::read_to_string(&report.written[0])
                .unwrap()
                .starts_with("---\ntitle:")
        );
    }

    #[test]
    fn reports_each_failure_and_keeps_going() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let mut second = meeting();
        second.meeting.title = "Retro".into();

        let report = write_meetings(&[meeting(), second], &missing, &new_york());

        assert!(report.written.is_empty());
        let titles: Vec<&str> = report.failed.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, vec!["Design sync", "Retro"]);
    }

    #[test]
    fn writes_a_single_meeting_to_the_chosen_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chosen.md");
        std::fs::write(&path, "old").unwrap();

        write_meeting(&meeting(), &path, &new_york()).unwrap();

        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# Design sync")
        );
    }
}
