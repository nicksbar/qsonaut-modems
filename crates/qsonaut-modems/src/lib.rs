//! First-party, UI-independent contracts shared by QSONaut modem consumers.
//!
//! This crate deliberately does not contain protocol algorithms, audio-device
//! ownership, GUI state, radio control, TX scheduling, or QSO automation.
//! Protocol implementations live in sibling first-party crates or external
//! adapter repositories, not in this generic contract crate.

mod audio;
mod events;
mod fixtures;
mod timing;
mod voice;

pub use audio::{
    extract_aligned_window, normalize_pcm16_interleaved, normalize_pcm16_mono, AudioBlock,
    AudioError, AudioNormalizer, AudioRingBuffer, Decimator48To12,
};
pub use events::{DecodeBatch, DecodeEvent, DecodeTelemetry, ModemId};
pub use fixtures::{FixtureDecode, FixtureError, FixtureProvenance, ModemFixture};
pub use timing::{SlotGate, SlotSpec};
pub use voice::{
    VoiceDirection, VoiceModemCapabilities, VoiceRxBlock, VoiceRxStatus, VoiceSyncState,
    VoiceTxBlock,
};
