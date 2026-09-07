use thiserror::Error;

/// Errors returned while packing a JS8 payload into a frame.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Js8EncodeError {
    #[error("JS8 payload must contain exactly 12 ASCII characters, got {actual} bytes")]
    InvalidLength { actual: usize },
    #[error("JS8 payload contains unsupported byte 0x{byte:02x} at index {index}")]
    InvalidCharacter { index: usize, byte: u8 },
}

/// Errors returned while synthesizing a JS8 waveform.
#[derive(Debug, Error, PartialEq)]
pub enum Js8SynthesisError {
    #[error("JS8 base frequency must be finite")]
    InvalidBaseFrequency,
    #[error("JS8 timing drift must be finite and preserve a positive sample clock")]
    InvalidTimingDrift,
    #[error("JS8 tone at index {index} is outside the 8-FSK range: {tone}")]
    InvalidTone { index: usize, tone: u8 },
}

/// Errors returned by the aligned JS8 tone correlator.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Js8DecodeError {
    #[error("JS8 base frequency must be finite")]
    InvalidBaseFrequency,
    #[error("JS8 waveform must contain {expected} samples, got {actual}")]
    InvalidSampleCount { expected: usize, actual: usize },
    #[error("JS8 waveform contains a non-finite sample")]
    NonFiniteSample,
    #[error("JS8 timing search requires at least {expected} samples, got {actual}")]
    InvalidTimingWindow { expected: usize, actual: usize },
    #[error("JS8 frequency search range is invalid")]
    InvalidFrequencySearchRange,
    #[error("JS8 frequency search step must be finite and greater than zero")]
    InvalidFrequencySearchStep,
    #[error("JS8 LDPC input contains a non-finite log-likelihood")]
    NonFiniteLogLikelihood,
    #[error("JS8 LDPC iteration count must be greater than zero")]
    InvalidFecIterations,
    #[error("JS8 LDPC decoder did not converge after {iterations} iterations")]
    FecDecodeFailed { iterations: usize },
    #[error("JS8 codeword bit at index {index} is not binary: {value}")]
    InvalidCodewordBit { index: usize, value: u8 },
    #[error("JS8 frame CRC mismatch: expected 0x{expected:03x}, got 0x{actual:03x}")]
    CrcMismatch { expected: u16, actual: u16 },
}
