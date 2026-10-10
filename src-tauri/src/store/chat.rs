use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use super::{Store, StoreError, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatRole {
    /// Your question.
    User,
    /// The model's answer.
    Assistant,
}

impl ChatRole {
    fn as_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }
}

/// A source an answer cites, under the label the answer uses, such as `S3`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Citation {
    #[serde(rename_all = "camelCase")]
    Transcript {
        label: String,
        segment_id: i64,
        start_ms: i64,
    },
    /// A passage of **My notes**, kept as text so it can be found again after
    /// the notes change.
    Notes { label: String, text: String },
}

impl Citation {
    pub fn label(&self) -> &str {
        match self {
            Citation::Transcript { label, .. } | Citation::Notes { label, .. } => label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: i64,
    pub meeting_id: i64,
    pub role: ChatRole,
    /// The answer keeps its citation labels, such as `[S3]`; `sources` says what each is.
    pub body_markdown: String,
    pub sources: Vec<Citation>,
    pub model_id: Option<String>,
    pub created_at: i64,
}

/// A question and its finished answer, saved together.
#[derive(Debug, Clone, PartialEq)]
pub struct NewExchange {
    pub meeting_id: i64,
    pub question: String,
    pub answer_markdown: String,
    pub sources: Vec<Citation>,
    pub model_id: String,
}

pub(super) const CHAT_COLUMNS: &str =
    "id, meeting_id, role, body_markdown, sources_json, model_id, created_at";

pub(super) fn chat_from_row(row: &Row) -> rusqlite::Result<ChatMessage> {
    let role: String = row.get(2)?;
    let sources: String = row.get(4)?;
    Ok(ChatMessage {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        role: if role == "assistant" {
            ChatRole::Assistant
        } else {
            ChatRole::User
        },
        body_markdown: row.get(3)?,
        sources: serde_json::from_str(&sources).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        model_id: row.get(5)?,
        created_at: row.get(6)?,
    })
}

impl Store {
    /// The meeting's conversation, oldest first.
    pub fn chat_messages(&self, meeting_id: i64) -> Result<Vec<ChatMessage>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHAT_COLUMNS} FROM chat_messages WHERE meeting_id = ?1 ORDER BY id"
        ))?;
        let messages = stmt
            .query_map([meeting_id], chat_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(messages)
    }

    /// Saves a question and its answer in one transaction, and returns both.
    pub fn insert_chat_exchange(
        &self,
        exchange: NewExchange,
    ) -> Result<(ChatMessage, ChatMessage), StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let created_at = now_ms();
        let sources = serde_json::to_string(&exchange.sources).expect("citations serialize");
        let question = insert(
            &tx,
            exchange.meeting_id,
            ChatRole::User,
            &exchange.question,
            "[]",
            None,
            created_at,
        )?;
        let answer = insert(
            &tx,
            exchange.meeting_id,
            ChatRole::Assistant,
            &exchange.answer_markdown,
            &sources,
            Some(&exchange.model_id),
            created_at,
        )?;
        tx.commit()?;
        Ok((
            ChatMessage {
                id: question,
                meeting_id: exchange.meeting_id,
                role: ChatRole::User,
                body_markdown: exchange.question,
                sources: Vec::new(),
                model_id: None,
                created_at,
            },
            ChatMessage {
                id: answer,
                meeting_id: exchange.meeting_id,
                role: ChatRole::Assistant,
                body_markdown: exchange.answer_markdown,
                sources: exchange.sources,
                model_id: Some(exchange.model_id),
                created_at,
            },
        ))
    }
}

fn insert(
    conn: &Connection,
    meeting_id: i64,
    role: ChatRole,
    body: &str,
    sources_json: &str,
    model_id: Option<&str>,
    created_at: i64,
) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO chat_messages (meeting_id, role, body_markdown, sources_json, model_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![meeting_id, role.as_str(), body, sources_json, model_id, created_at],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StoreError::MeetingNotFound(meeting_id)
        }
        other => other.into(),
    })?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(meeting_id: i64, question: &str) -> NewExchange {
        NewExchange {
            meeting_id,
            question: question.into(),
            answer_markdown: "It moved to May 4 [S1].".into(),
            sources: vec![
                Citation::Transcript {
                    label: "S1".into(),
                    segment_id: 7,
                    start_ms: 65_000,
                },
                Citation::Notes {
                    label: "N1".into(),
                    text: "- launch: May 4".into(),
                },
            ],
            model_id: "test-model".into(),
        }
    }

    #[test]
    fn saves_a_question_and_answer_and_reads_them_back_in_order() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_meeting("Sync").unwrap().id;

        let (question, answer) = store
            .insert_chat_exchange(exchange(id, "When is the launch?"))
            .unwrap();
        store
            .insert_chat_exchange(exchange(id, "Who owns it?"))
            .unwrap();

        assert_eq!(question.role, ChatRole::User);
        assert_eq!(question.model_id, None);
        assert_eq!(answer.role, ChatRole::Assistant);
        assert_eq!(answer.sources.len(), 2);
        let saved = store.chat_messages(id).unwrap();
        assert_eq!(saved.len(), 4);
        assert_eq!(saved[0], question);
        assert_eq!(saved[1], answer);
        assert_eq!(saved[2].body_markdown, "Who owns it?");
    }

    #[test]
    fn keeps_each_meeting_s_chat_separate() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_meeting("A").unwrap().id;
        let b = store.create_meeting("B").unwrap().id;

        store.insert_chat_exchange(exchange(a, "Question")).unwrap();

        assert_eq!(store.chat_messages(a).unwrap().len(), 2);
        assert!(store.chat_messages(b).unwrap().is_empty());
    }

    #[test]
    fn deletes_the_chat_with_its_meeting() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_meeting("Sync").unwrap().id;
        store
            .insert_chat_exchange(exchange(id, "Question"))
            .unwrap();

        store.delete_meeting(id).unwrap();

        assert!(store.chat_messages(id).unwrap().is_empty());
        let rows: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn refuses_an_exchange_for_a_deleted_meeting() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.insert_chat_exchange(exchange(42, "Question")),
            Err(StoreError::MeetingNotFound(42))
        ));
        assert!(store.chat_messages(42).unwrap().is_empty());
    }

    #[test]
    fn serializes_citations_for_the_ui() {
        let transcript = Citation::Transcript {
            label: "S1".into(),
            segment_id: 7,
            start_ms: 65_000,
        };

        assert_eq!(
            serde_json::to_value(&transcript).unwrap(),
            serde_json::json!({ "kind": "transcript", "label": "S1", "segmentId": 7, "startMs": 65000 })
        );
        assert_eq!(transcript.label(), "S1");
    }
}
