//! Full-text search over meeting titles, notes, and transcripts, through the
//! FTS5 indexes from migration 0005.

use rusqlite::types::Value;

use super::{MeetingCursor, MeetingPage, Store, StoreError};

/// The most words a search uses. Later words are ignored.
const MAX_TERMS: usize = 8;

/// Turns what the user typed into one FTS5 prefix query per word. Each word is
/// quoted, so FTS5 operators and punctuation in it are taken literally.
/// Words with no letters or digits are dropped, since they can't match.
fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(MAX_TERMS)
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect()
}

/// A condition that holds for meetings whose title, notes, or transcript
/// matches the FTS5 query in parameter `?{param}`.
fn matches(param: usize) -> String {
    format!(
        "AND m.id IN (
             SELECT rowid FROM meetings_fts WHERE meetings_fts MATCH ?{param}
             UNION
             SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?{param}
             UNION
             SELECT s.meeting_id FROM transcript_fts
             JOIN transcript_segments s ON s.id = transcript_fts.rowid
             WHERE transcript_fts MATCH ?{param}
         )"
    )
}

impl Store {
    /// One page of the meetings that match `query`, newest first, paged like
    /// [`Store::list_meetings`]. Every word in the query must start a word
    /// somewhere in the meeting's title, notes, or transcript; case and
    /// accents are ignored. A query with no words lists every meeting.
    pub fn search_meetings(
        &self,
        query: &str,
        after: Option<MeetingCursor>,
        limit: u32,
    ) -> Result<MeetingPage, StoreError> {
        let terms = terms(query);
        // Parameters ?1 to ?3 are the cursor and limit.
        let filter: String = (4..4 + terms.len()).map(matches).collect();
        let params = terms.into_iter().map(Value::Text).collect();
        self.page_of_meetings(&filter, params, after, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Meeting, NewSegment, Source};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn found(store: &Store, query: &str) -> Vec<String> {
        let mut titles = Vec::new();
        let mut after = None;
        loop {
            let page = store.search_meetings(query, after, 2).unwrap();
            titles.extend(page.meetings.into_iter().map(|m: Meeting| m.title));
            match page.next {
                Some(next) => after = Some(next),
                None => return titles,
            }
        }
    }

    fn say(store: &Store, meeting_id: i64, text: &str) {
        store
            .insert_segment(NewSegment {
                meeting_id,
                source: Source::Them,
                start_ms: 0,
                end_ms: 1000,
                text: text.into(),
                language: None,
            })
            .unwrap();
    }

    #[test]
    fn quotes_each_word_as_a_prefix() {
        assert_eq!(
            terms("  budget  review "),
            vec!["\"budget\"*", "\"review\"*"]
        );
        assert_eq!(terms("say \"hi\""), vec!["\"say\"*", "\"\"\"hi\"\"\"*"]);
        assert!(terms(" -- * \" ").is_empty());
        assert_eq!(terms(&"word ".repeat(20)).len(), MAX_TERMS);
    }

    #[test]
    fn finds_meetings_by_title_notes_or_transcript() {
        let store = store();
        store.create_meeting("Budget review").unwrap();
        let notes = store.create_meeting("Planning").unwrap();
        store.update_notes(notes.id, "- move the budget").unwrap();
        let heard = store.create_meeting("Standup").unwrap();
        say(&store, heard.id, "The budget is final.");
        store.create_meeting("Retro").unwrap();

        assert_eq!(
            found(&store, "budget"),
            vec!["Standup", "Planning", "Budget review"]
        );
    }

    #[test]
    fn matches_word_prefixes_ignoring_case_and_accents() {
        let store = store();
        store.create_meeting("Café roadmap").unwrap();

        assert_eq!(found(&store, "CAFE"), vec!["Café roadmap"]);
        assert_eq!(found(&store, "road"), vec!["Café roadmap"]);
        assert!(found(&store, "map").is_empty());
    }

    #[test]
    fn needs_every_word_but_not_in_the_same_place() {
        let store = store();
        let both = store.create_meeting("Hiring sync").unwrap();
        store.update_notes(both.id, "Talked to Megan").unwrap();
        store.create_meeting("Hiring plan").unwrap();

        assert_eq!(found(&store, "hiring megan"), vec!["Hiring sync"]);
    }

    #[test]
    fn takes_search_syntax_literally() {
        let store = store();
        store.create_meeting("Q3 OR Q4 planning").unwrap();
        store.create_meeting("Q4 only").unwrap();

        assert_eq!(found(&store, "q3 OR"), vec!["Q3 OR Q4 planning"]);
        assert_eq!(found(&store, "NOT q4"), Vec::<String>::new());
        assert_eq!(found(&store, "\"planning"), vec!["Q3 OR Q4 planning"]);
        assert_eq!(found(&store, "q3*"), vec!["Q3 OR Q4 planning"]);
    }

    #[test]
    fn lists_everything_for_a_query_without_words() {
        let store = store();
        store.create_meeting("One").unwrap();
        store.create_meeting("Two").unwrap();

        assert_eq!(found(&store, "  "), vec!["Two", "One"]);
        assert_eq!(found(&store, "-"), vec!["Two", "One"]);
    }

    #[test]
    fn follows_renames_note_edits_and_deletes() {
        let store = store();
        let meeting = store.create_meeting("Old name").unwrap();
        store.update_notes(meeting.id, "first draft").unwrap();
        say(&store, meeting.id, "spoken words");

        store.rename_meeting(meeting.id, "New name").unwrap();
        store.update_notes(meeting.id, "second draft").unwrap();

        assert!(found(&store, "old").is_empty());
        assert!(found(&store, "first").is_empty());
        assert_eq!(found(&store, "new second spoken"), vec!["New name"]);

        store.delete_meeting(meeting.id).unwrap();

        assert!(found(&store, "new").is_empty());
        assert!(found(&store, "spoken").is_empty());
        let rows: i64 = store
            .conn()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM meetings_fts)
                      + (SELECT COUNT(*) FROM notes_fts)
                      + (SELECT COUNT(*) FROM transcript_fts)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn pages_through_matches_newest_first() {
        let store = store();
        for title in ["Sync 1", "Other", "Sync 2", "Sync 3", "Sync 4", "Sync 5"] {
            store.create_meeting(title).unwrap();
        }

        assert_eq!(
            found(&store, "sync"),
            vec!["Sync 5", "Sync 4", "Sync 3", "Sync 2", "Sync 1"]
        );
    }

    #[test]
    fn keeps_the_indexes_consistent() {
        let store = store();
        let meeting = store.create_meeting("Check").unwrap();
        store.update_notes(meeting.id, "notes").unwrap();
        say(&store, meeting.id, "words");
        store.rename_meeting(meeting.id, "Checked").unwrap();

        let conn = store.conn();
        for table in ["meetings_fts", "notes_fts", "transcript_fts"] {
            conn.execute(
                &format!("INSERT INTO {table} ({table}, rank) VALUES ('integrity-check', 1)"),
                [],
            )
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        }
    }
}
