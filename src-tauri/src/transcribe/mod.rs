//! Speech-to-text engines.

pub mod echo;
pub mod phantom;
pub mod whisper;
pub mod whistle;
pub mod worker;

/// Whether recordings may transcribe with Whisper (#25). Off until the model
/// download and its setting ship; while off, the worker runs every job on
/// Whistle.
pub const WHISPER_ENABLED: bool = false;
