use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError};

/// The microphone to record from; absent means the system default.
pub const INPUT_DEVICE: &str = "input_device_id";

impl Store {
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Stores a setting, or removes it when `value` is `None`.
    pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn();
        match value {
            Some(value) => conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?,
            None => conn.execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_replaces_and_clears_a_setting() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting(INPUT_DEVICE).unwrap(), None);

        store.set_setting(INPUT_DEVICE, Some("usb-mic")).unwrap();
        store.set_setting(INPUT_DEVICE, Some("headset")).unwrap();
        assert_eq!(
            store.setting(INPUT_DEVICE).unwrap().as_deref(),
            Some("headset")
        );

        store.set_setting(INPUT_DEVICE, None).unwrap();
        assert_eq!(store.setting(INPUT_DEVICE).unwrap(), None);
    }
}
