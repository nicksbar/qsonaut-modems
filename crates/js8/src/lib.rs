//! First-party JS8 protocol and modem primitives.
//!
//! JS8-specific implementation is kept in focused modules. This crate is
//! separate from the generic `qsonaut-modems` contracts crate and remains
//! UI-, device-, and consumer-neutral.

mod adapter;
mod alphabet;
mod costas;
mod crc;
mod decode;
mod errors;
mod fec;
mod frame;
mod messages;
mod metrics;
mod mode;
mod scan;
mod sync;
mod synth;

pub use adapter::{
    decode_audio_block, decode_audio_block_detailed, encode_audio_block,
    encode_message_audio_block, slot_spec, Js8AdapterError, Js8RxConfig, Js8RxResult, Js8TxConfig,
};
pub use decode::{decode_audio, decode_frame, decode_llrs, Js8DecodedFrame};
pub use errors::Js8DecodeError;
pub use errors::Js8EncodeError;
pub use errors::Js8SynthesisError;
pub use fec::decode_codeword;
pub use frame::encode_tones;
pub use messages::{
    decode_message, encode_message, Js8Command, Js8FrameType, Js8Message, Js8MessageError,
};
pub use metrics::{
    demodulate_bit_llrs, demodulate_soft, estimate_snr_db, tone_metrics_to_bit_llrs, Js8BitLlrs,
    Js8ToneMetrics,
};
pub use mode::Js8Mode;
pub use qsonaut_modems::AudioBlock;
pub use scan::{scan_audio_block, scan_audio_block_detailed, Js8ScanConfig, Js8ScanResult};
pub use sync::{demodulate_aligned, estimate_base_frequency, find_symbol_boundary};
pub use synth::{
    synthesize, synthesize_with_frequency_curve, synthesize_with_frequency_drift,
    synthesize_with_timing_drift, SAMPLE_RATE_HZ,
};
