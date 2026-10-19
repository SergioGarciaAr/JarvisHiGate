//! Voice (phase 3A): speech synthesis with macOS `say` and a prepared, but
//! not yet implemented, speech recognition integration point. No microphone
//! access happens anywhere in this module.

pub mod neural;
pub mod prosody;
pub mod stt;
pub mod tts;
