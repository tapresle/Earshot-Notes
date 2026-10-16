use rusqlite::{OptionalExtension, params};

use super::{Source, Store, StoreError};

/// The microphone to record from; absent means the system default.
pub const INPUT_DEVICE: &str = "input_device_id";
/// Whether recordings keep their audio; "on" keeps it, absent means off.
pub const KEEP_AUDIO: &str = "keep_audio_recordings";
/// The stored value of an on/off setting, such as `KEEP_AUDIO`, when it's on.
pub const ON: &str = "on";
/// Whether **New meeting** in the window also starts recording; "on" starts
/// it, absent means off.
pub const RECORD_NEW_MEETINGS: &str = "record_new_meetings";
/// Whether stopping a recording summarizes the meeting; "on" summarizes it,
/// absent means off.
pub const SUMMARIZE_ON_STOP: &str = "summarize_on_stop";
/// How readily quiet microphone audio counts as speech; absent means normal.
pub const MIC_SENSITIVITY: &str = "mic_speech_sensitivity";
/// How readily quiet system audio counts as speech; absent means normal.
pub const SYSTEM_SENSITIVITY: &str = "system_speech_sensitivity";
/// The vocabulary's terms as JSON (`vocabulary::Term`).
pub const VOCABULARY: &str = "vocabulary";
/// The transcription model for new recordings, by id; absent means Whistle.
pub const TRANSCRIPTION_MODEL: &str = "transcription_model";
/// How long the summary model stays loaded after a request
/// (`llm::resident::KeepLoaded`); absent means five minutes.
pub const KEEP_MODEL_LOADED: &str = "keep_model_loaded";

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

    /// Whether an on/off setting is on; absent means off.
    pub fn switch(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self.setting(key)?.as_deref() == Some(ON))
    }

    /// Turns an on/off setting on, or removes it to turn it off.
    pub fn set_switch(&self, key: &str, on: bool) -> Result<(), StoreError> {
        self.set_setting(key, on.then_some(ON))
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
    fn switches_stay_off_until_turned_on() {
        let store = Store::open_in_memory().unwrap();
        for key in [RECORD_NEW_MEETINGS, SUMMARIZE_ON_STOP] {
            assert!(!store.switch(key).unwrap());

            store.set_switch(key, true).unwrap();
            assert!(store.switch(key).unwrap());

            store.set_switch(key, false).unwrap();
            assert!(!store.switch(key).unwrap());
            assert_eq!(store.setting(key).unwrap(), None);
        }
    }

    #[test]
    fn switches_are_independent() {
        let store = Store::open_in_memory().unwrap();
        store.set_switch(SUMMARIZE_ON_STOP, true).unwrap();

        assert!(store.switch(SUMMARIZE_ON_STOP).unwrap());
        assert!(!store.switch(RECORD_NEW_MEETINGS).unwrap());
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
