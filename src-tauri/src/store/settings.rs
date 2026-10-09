use rusqlite::{OptionalExtension, params};

use super::{Source, Store, StoreError};

/// The microphone to record from; absent means the system default.
pub const INPUT_DEVICE: &str = "input_device_id";
/// Whether recordings keep their audio; "on" keeps it, absent means off.
pub const KEEP_AUDIO: &str = "keep_audio_recordings";
/// The stored value of `KEEP_AUDIO` when it's on.
pub const ON: &str = "on";
/// How readily quiet microphone audio counts as speech; absent means normal.
pub const MIC_SENSITIVITY: &str = "mic_speech_sensitivity";
/// How readily quiet system audio counts as speech; absent means normal.
pub const SYSTEM_SENSITIVITY: &str = "system_speech_sensitivity";
/// The vocabulary's terms as JSON (`vocabulary::Term`).
pub const VOCABULARY: &str = "vocabulary";
/// The transcription model for new recordings, by id; absent means Whistle.
pub const TRANSCRIPTION_MODEL: &str = "transcription_model";
/// Whether the suggestion to download Whisper was dismissed; "dismissed" if so.
pub const WHISPER_SUGGESTION: &str = "whisper_suggestion";
/// The stored value of `WHISPER_SUGGESTION` once dismissed.
pub const DISMISSED: &str = "dismissed";

/// The setting that holds a source's speech sensitivity.
pub fn sensitivity_key(source: Source) -> &'static str {
    match source {
        Source::Me => MIC_SENSITIVITY,
        Source::Them => SYSTEM_SENSITIVITY,
    }
}

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

    #[test]
    fn keeps_each_source_s_sensitivity_under_its_own_key() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setting(sensitivity_key(Source::Me)).unwrap(), None);

        store
            .set_setting(sensitivity_key(Source::Me), Some("high"))
            .unwrap();
        store
            .set_setting(sensitivity_key(Source::Them), Some("low"))
            .unwrap();

        assert_eq!(
            store.setting(MIC_SENSITIVITY).unwrap().as_deref(),
            Some("high")
        );
        assert_eq!(
            store.setting(SYSTEM_SENSITIVITY).unwrap().as_deref(),
            Some("low")
        );
    }
}
