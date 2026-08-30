//! First-party, UI-independent contracts shared by QSONaut modem consumers.
//!
//! This crate deliberately does not contain protocol algorithms, audio-device
//! ownership, GUI state, radio control, TX scheduling, or QSO automation.
//! Those belong in an adapter or consumer application.

mod audio;
mod events;
mod timing;

pub use audio::{extract_aligned_window, AudioBlock, AudioError};
pub use events::{DecodeBatch, DecodeEvent, DecodeTelemetry, ModemId};
pub use timing::{SlotGate, SlotSpec};
