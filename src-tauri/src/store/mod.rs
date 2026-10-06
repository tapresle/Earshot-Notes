//! SQLite persistence for meetings and notes, and search over them.

mod export;
mod meetings;
mod migrations;
mod models;
mod search;
pub mod settings;
mod summaries;
mod transcript;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use export::ExportMeeting;
pub use meetings::{
    DEFAULT_TITLE, MAX_PAGE_SIZE, Meeting, MeetingCursor, MeetingDetail, MeetingPage,
};
pub use models::{ModelRecord, ModelSource};
pub use summaries::{NewSummary, Summary};
pub use transcript::{NewSegment, Source, TranscriptSegment};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("meeting {0} doesn't exist")]
    MeetingNotFound(i64),
    #[error("model {0} doesn't exist")]
    ModelNotFound(String),
    #[error(
        "the database was created by a newer version of the app (schema {found}, this build supports {supported})"
    )]
    SchemaTooNew { found: i64, supported: usize },
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens or creates the database file and applies pending migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrations::apply(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock can't leave SQLite in a torn state, so recover.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
