use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use super::{Store, StoreError, now_ms};

const SELECTED_SUMMARY_MODEL: &str = "summary_model_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelSource {
    Import,
    Download,
}

impl ModelSource {
    fn as_str(self) -> &'static str {
        match self {
            ModelSource::Import => "import",
            ModelSource::Download => "download",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    pub id: String,
    pub display_name: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub source: ModelSource,
    pub source_url: Option<String>,
    pub version: Option<String>,
    pub min_runtime: Option<String>,
    pub added_at: i64,
}

const COLUMNS: &str = "id, display_name, file_path, size_bytes, sha256, source, source_url, version, min_runtime, added_at";

fn from_row(row: &Row) -> rusqlite::Result<ModelRecord> {
    let source: String = row.get(5)?;
    Ok(ModelRecord {
        id: row.get(0)?,
        display_name: row.get(1)?,
        file_path: row.get(2)?,
        size_bytes: row.get(3)?,
        sha256: row.get(4)?,
        source: if source == "download" {
            ModelSource::Download
        } else {
            ModelSource::Import
        },
        source_url: row.get(6)?,
        version: row.get(7)?,
        min_runtime: row.get(8)?,
        added_at: row.get(9)?,
    })
}

impl Store {
    /// Records a model. `added_at` is set to now.
    pub fn add_model(&self, model: &ModelRecord) -> Result<ModelRecord, StoreError> {
        let added_at = now_ms();
        self.conn().execute(
            &format!(
                "INSERT INTO models ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                model.id,
                model.display_name,
                model.file_path,
                model.size_bytes,
                model.sha256,
                model.source.as_str(),
                model.source_url,
                model.version,
                model.min_runtime,
                added_at,
            ],
        )?;
        Ok(ModelRecord {
            added_at,
            ..model.clone()
        })
    }

    pub fn model_by_sha256(&self, sha256: &str) -> Result<Option<ModelRecord>, StoreError> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM models WHERE sha256 = ?1"),
                [sha256],
                from_row,
            )
            .optional()?)
    }

    pub fn model_by_id(&self, id: &str) -> Result<Option<ModelRecord>, StoreError> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM models WHERE id = ?1"),
                [id],
                from_row,
            )
            .optional()?)
    }

    /// Forgets a model, and the selection if it pointed at that model, so
    /// summaries fall back to the newest remaining one.
    pub fn delete_model(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if tx.execute("DELETE FROM models WHERE id = ?1", [id])? == 0 {
            return Err(StoreError::ModelNotFound(id.to_owned()));
        }
        tx.execute(
            "DELETE FROM settings WHERE key = ?1 AND value = ?2",
            params![SELECTED_SUMMARY_MODEL, id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Installed models, newest first.
    pub fn list_models(&self) -> Result<Vec<ModelRecord>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM models ORDER BY added_at DESC, id"
        ))?;
        let models = stmt.query_map([], from_row)?.collect::<Result<_, _>>()?;
        Ok(models)
    }

    pub fn select_summary_model(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn();
        let exists: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM models WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::ModelNotFound(id.to_owned()));
        }
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![SELECTED_SUMMARY_MODEL, id],
        )?;
        Ok(())
    }

    /// The selected model, or the newest one if none is selected or the
    /// selection no longer exists.
    pub fn summary_model(&self) -> Result<Option<ModelRecord>, StoreError> {
        Ok(self
            .conn()
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM models
                     ORDER BY id = (SELECT value FROM settings WHERE key = ?1) DESC, added_at DESC
                     LIMIT 1"
                ),
                [SELECTED_SUMMARY_MODEL],
                from_row,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, sha256: &str) -> ModelRecord {
        ModelRecord {
            id: id.into(),
            display_name: id.into(),
            file_path: format!("/models/{id}.gguf"),
            size_bytes: 100,
            sha256: sha256.into(),
            source: ModelSource::Import,
            source_url: None,
            version: Some("1".into()),
            min_runtime: Some("0.1.158".into()),
            added_at: 0,
        }
    }

    #[test]
    fn stores_and_finds_models_by_checksum() {
        let store = Store::open_in_memory().unwrap();
        let added = store.add_model(&record("a", "aaa")).unwrap();

        assert_eq!(store.model_by_sha256("aaa").unwrap(), Some(added));
        assert_eq!(store.model_by_sha256("zzz").unwrap(), None);
    }

    #[test]
    fn rejects_a_duplicate_checksum() {
        let store = Store::open_in_memory().unwrap();
        store.add_model(&record("a", "same")).unwrap();

        assert!(store.add_model(&record("b", "same")).is_err());
    }

    #[test]
    fn uses_the_newest_model_until_one_is_selected() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.summary_model().unwrap(), None);
        store.add_model(&record("first", "1")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.add_model(&record("second", "2")).unwrap();

        assert_eq!(store.summary_model().unwrap().unwrap().id, "second");

        store.select_summary_model("first").unwrap();
        assert_eq!(store.summary_model().unwrap().unwrap().id, "first");
    }

    #[test]
    fn deleting_the_selected_model_falls_back_to_the_newest_remaining() {
        let store = Store::open_in_memory().unwrap();
        store.add_model(&record("first", "1")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.add_model(&record("second", "2")).unwrap();
        store.select_summary_model("first").unwrap();

        store.delete_model("first").unwrap();

        assert_eq!(store.model_by_id("first").unwrap(), None);
        assert_eq!(store.summary_model().unwrap().unwrap().id, "second");
        assert_eq!(store.setting(SELECTED_SUMMARY_MODEL).unwrap(), None);
    }

    #[test]
    fn deleting_another_model_keeps_the_selection() {
        let store = Store::open_in_memory().unwrap();
        store.add_model(&record("first", "1")).unwrap();
        store.add_model(&record("second", "2")).unwrap();
        store.select_summary_model("first").unwrap();

        store.delete_model("second").unwrap();

        assert_eq!(store.summary_model().unwrap().unwrap().id, "first");
    }

    #[test]
    fn refuses_to_delete_an_unknown_model() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.delete_model("ghost"),
            Err(StoreError::ModelNotFound(_))
        ));
    }

    #[test]
    fn refuses_to_select_an_unknown_model() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.select_summary_model("ghost"),
            Err(StoreError::ModelNotFound(_))
        ));
    }
}
