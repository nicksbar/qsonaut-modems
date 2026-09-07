use std::time::{Duration, Instant};

use qsonaut_modems::{AudioBlock, AudioError, DecodeBatch, DecodeEvent, ModemId, SlotSpec};
use thiserror::Error;

use crate::{
    encode_tones, errors::Js8DecodeError, Js8DecodedFrame, Js8EncodeError, Js8Message,
    Js8MessageError, Js8Mode, Js8SynthesisError, SAMPLE_RATE_HZ,
};

/// Stable modem identifier used in generic `DecodeEvent` values.
pub const MODEM_ID: ModemId = ModemId("js8");

/// TX parameters owned by the consumer's send workflow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Js8TxConfig {
    pub mode: Js8Mode,
    pub base_frequency_hz: f32,
    pub frame_type: u8,
}

impl Default for Js8TxConfig {
    fn default() -> Self {
        Self {
            mode: Js8Mode::Normal,
            base_frequency_hz: 1500.0,
            frame_type: 0,
        }
    }
}

/// RX search and FEC parameters for one complete JS8 frame window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Js8RxConfig {
    pub mode: Js8Mode,
    pub center_frequency_hz: f32,
    pub frequency_half_width_hz: f32,
    pub frequency_step_hz: f32,
    pub max_fec_iterations: usize,
}

impl Default for Js8RxConfig {
    fn default() -> Self {
        Self {
            mode: Js8Mode::Normal,
            center_frequency_hz: 1500.0,
            frequency_half_width_hz: 5.0,
            frequency_step_hz: 0.5,
            max_fec_iterations: 10,
        }
    }
}

/// Protocol-specific RX details plus the normalized event projection.
#[derive(Debug, Clone, PartialEq)]
pub struct Js8RxResult {
    pub frame: Js8DecodedFrame,
    /// JS8Call message-layer interpretation of the decoded frame.
    pub message: Js8Message,
    pub event: DecodeEvent,
    /// Fitted carrier slope in hertz per second across the frame.
    pub frequency_drift_hz_per_second: f32,
    /// Fitted carrier curvature in hertz per second squared.
    pub frequency_curvature_hz_per_second2: f32,
    /// Fitted cumulative sample-clock drift in samples per second.
    pub timing_drift_samples_per_second: f32,
    /// Fitted quadratic sample-clock drift in samples per second squared.
    pub timing_curvature_samples_per_second2: f32,
}

#[derive(Debug, Error)]
pub enum Js8AdapterError {
    #[error("JS8 adapter requires 12 kHz mono audio, got {actual} Hz")]
    UnsupportedSampleRate { actual: u32 },
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Encode(#[from] Js8EncodeError),
    #[error(transparent)]
    Message(#[from] Js8MessageError),
    #[error(transparent)]
    Synthesis(#[from] Js8SynthesisError),
    #[error(transparent)]
    Decode(#[from] Js8DecodeError),
    #[error("JS8 scan step must be greater than zero")]
    InvalidScanStep,
    #[error("JS8 scan candidate limit must be greater than zero")]
    InvalidScanLimit,
    #[error("JS8 minimum sync quality must be finite and non-negative")]
    InvalidSyncQuality,
    #[error("JS8 sync frequency range must be finite and non-negative")]
    InvalidSyncFrequencyRange,
    #[error("JS8 sync frequency step must be finite and greater than zero")]
    InvalidSyncFrequencyStep,
}

/// Encode one JS8 message into a validated generic audio block.
pub fn encode_audio_block(
    message: &str,
    config: Js8TxConfig,
) -> Result<AudioBlock, Js8AdapterError> {
    let tones = encode_tones(message, config.frame_type, config.mode)?;
    let samples = crate::synthesize(&tones, config.mode, config.base_frequency_hz)?;
    Ok(AudioBlock::new(SAMPLE_RATE_HZ, samples)?)
}

/// Encode a typed JS8 message into a validated 12 kHz mono audio block.
pub fn encode_message_audio_block(
    message: &Js8Message,
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<AudioBlock, Js8AdapterError> {
    let (payload, frame_type) = crate::encode_message(message)?;
    let tones = encode_tones(&payload, frame_type, mode)?;
    let samples = crate::synthesize(&tones, mode, base_frequency_hz)?;
    Ok(AudioBlock::new(SAMPLE_RATE_HZ, samples)?)
}

/// Decode one aligned or one-symbol-leading JS8 frame window into normalized
/// generic events. The consumer still owns slot gating, buffering, retries,
/// and no-decode policy.
pub fn decode_audio_block(
    audio: &AudioBlock,
    config: Js8RxConfig,
) -> Result<DecodeBatch, Js8AdapterError> {
    let started = Instant::now();
    let result = decode_audio_block_detailed(audio, config)?;
    Ok(DecodeBatch::finish(
        audio.samples.len(),
        started,
        vec![result.event],
    ))
}

/// Decode a block while preserving protocol-specific frame metadata such as
/// the frame type for the consumer's message-semantics layer.
pub fn decode_audio_block_detailed(
    audio: &AudioBlock,
    config: Js8RxConfig,
) -> Result<Js8RxResult, Js8AdapterError> {
    if audio.sample_rate_hz != SAMPLE_RATE_HZ {
        return Err(Js8AdapterError::UnsupportedSampleRate {
            actual: audio.sample_rate_hz,
        });
    }

    let frame_samples = 79 * config.mode.samples_per_symbol();
    let mut offset = if audio.samples.len() == frame_samples {
        0
    } else {
        crate::find_symbol_boundary(&audio.samples, config.mode, config.center_frequency_hz)?
    };
    let mut frame = audio.samples.get(offset..offset + frame_samples).ok_or(
        Js8DecodeError::InvalidSampleCount {
            expected: frame_samples,
            actual: audio.samples.len().saturating_sub(offset),
        },
    )?;
    let frequency = crate::estimate_base_frequency(
        frame,
        config.mode,
        config.center_frequency_hz,
        config.frequency_half_width_hz,
        config.frequency_step_hz,
    )?;
    // A large frequency offset can rotate the long-symbol Costas correlation
    // enough to move the initial timing peak. Re-run timing once with the
    // acquired frequency before committing the decode window.
    if audio.samples.len() != frame_samples {
        let refined_offset = crate::find_symbol_boundary(&audio.samples, config.mode, frequency)?;
        if refined_offset != offset {
            offset = refined_offset;
            frame = audio.samples.get(offset..offset + frame_samples).ok_or(
                Js8DecodeError::InvalidSampleCount {
                    expected: frame_samples,
                    actual: audio.samples.len().saturating_sub(offset),
                },
            )?;
        }
    }
    let (frequency, drift_hz_per_second, curvature_hz_per_second2) =
        crate::sync::estimate_frequency_track(
            frame,
            config.mode,
            config.center_frequency_hz,
            config.frequency_half_width_hz,
            config.frequency_step_hz,
        )?;
    let timing_window = audio
        .samples
        .get(offset..)
        .ok_or(Js8DecodeError::InvalidSampleCount {
            expected: frame_samples,
            actual: audio.samples.len().saturating_sub(offset),
        })?;
    let (timing_drift_samples_per_second, timing_curvature_samples_per_second2) =
        crate::sync::estimate_timing_drift_curve(
            timing_window,
            config.mode,
            frequency,
            drift_hz_per_second,
            curvature_hz_per_second2,
        )?;
    let decoded = crate::decode::decode_audio_with_frequency_curve_and_timing_curve(
        timing_window,
        config.mode,
        frequency,
        drift_hz_per_second,
        curvature_hz_per_second2,
        timing_drift_samples_per_second,
        timing_curvature_samples_per_second2,
        config.max_fec_iterations,
    )?;
    let snr_db = crate::metrics::estimate_snr_db_with_frequency_curve(
        frame,
        config.mode,
        frequency,
        drift_hz_per_second,
        curvature_hz_per_second2,
    )?;
    let event = DecodeEvent {
        modem: MODEM_ID,
        message: decoded.message.clone(),
        snr_db,
        delta_time_seconds: Some(offset as f32 / SAMPLE_RATE_HZ as f32),
        audio_frequency_hz: Some(frequency),
    };
    let message = crate::decode_message(&decoded);
    Ok(Js8RxResult {
        frame: decoded,
        message,
        event,
        frequency_drift_hz_per_second: drift_hz_per_second,
        frequency_curvature_hz_per_second2: curvature_hz_per_second2,
        timing_drift_samples_per_second,
        timing_curvature_samples_per_second2,
    })
}

/// Return the generic slot description for a JS8 mode.
pub fn slot_spec(mode: Js8Mode) -> SlotSpec {
    let duration = Duration::from_secs(mode.tx_seconds() as u64);
    SlotSpec {
        modem: MODEM_ID,
        slot: duration,
        decode_after: duration,
        sample_rate_hz: SAMPLE_RATE_HZ,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_audio_block, decode_audio_block_detailed, encode_audio_block, slot_spec,
        Js8AdapterError, Js8RxConfig, Js8TxConfig,
    };
    use crate::Js8DecodeError;
    use qsonaut_modems::AudioBlock;

    #[test]
    fn tx_and_rx_cross_the_generic_audio_boundary() {
        let tx = Js8TxConfig {
            mode: crate::Js8Mode::Fast,
            base_frequency_hz: 1502.0,
            frame_type: 3,
        };
        let audio = encode_audio_block("0123456789AB", tx).unwrap();
        let rx = Js8RxConfig {
            mode: tx.mode,
            center_frequency_hz: 1500.0,
            ..Js8RxConfig::default()
        };
        let result = decode_audio_block_detailed(&audio, rx).unwrap();
        assert_eq!(result.frame.message, "0123456789AB");
        assert_eq!(result.frame.frame_type, 3);
        assert_eq!(result.event.audio_frequency_hz, Some(1502.0));
        assert!(result.event.snr_db.is_some());
        assert_eq!(decode_audio_block(&audio, rx).unwrap().events.len(), 1);
        let audio2 = encode_audio_block("CQTEST1234AB", tx).unwrap();
        let result2 = decode_audio_block_detailed(&audio2, rx).unwrap();
        assert_eq!(result2.frame.message, "CQTEST1234AB");
    }

    #[test]
    fn rx_tracks_a_linear_carrier_drift() {
        let mode = crate::Js8Mode::Normal;
        let tones = crate::encode_tones("DRIFT12ABCDE", 2, mode).unwrap();
        let samples = crate::synthesize_with_frequency_drift(&tones, mode, 1500.0, 0.5).unwrap();
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let result = decode_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 15.0,
                frequency_step_hz: 0.5,
                max_fec_iterations: 30,
            },
        )
        .unwrap();
        assert_eq!(result.frame.message, "DRIFT12ABCDE");
        assert_eq!(result.frame.frame_type, 2);
        assert!(
            (result.frequency_drift_hz_per_second - 0.5).abs() < 0.1,
            "estimated drift: {} curvature: {}",
            result.frequency_drift_hz_per_second,
            result.frequency_curvature_hz_per_second2
        );
        assert!(result.event.snr_db.is_some());
    }

    #[test]
    fn rx_tracks_a_curved_carrier() {
        let mode = crate::Js8Mode::Normal;
        let tones = crate::encode_tones("CURVE12ABCDE", 6, mode).unwrap();
        let samples =
            crate::synthesize_with_frequency_curve(&tones, mode, 1500.0, 0.2, 0.05).unwrap();
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let result = decode_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 15.0,
                frequency_step_hz: 0.5,
                max_fec_iterations: 30,
            },
        )
        .unwrap();
        assert_eq!(result.frame.message, "CURVE12ABCDE");
        assert_eq!(result.frame.frame_type, 6);
        assert!(result.frequency_curvature_hz_per_second2.abs() > 0.01);
    }

    #[test]
    fn rx_tracks_a_linear_sample_clock_drift() {
        let mode = crate::Js8Mode::Normal;
        let tones = crate::encode_tones("TIMING12ABCD", 4, mode).unwrap();
        let mut samples = crate::synthesize_with_timing_drift(&tones, mode, 1500.0, 24.0).unwrap();
        samples.resize(samples.len() + mode.samples_per_symbol() - 1, 0.0);
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let result = decode_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 15.0,
                frequency_step_hz: 0.5,
                max_fec_iterations: 30,
            },
        )
        .unwrap();
        assert_eq!(result.frame.message, "TIMING12ABCD");
        assert_eq!(result.frame.frame_type, 4);
        assert!(
            result.timing_drift_samples_per_second.is_finite()
                && result.timing_drift_samples_per_second > 0.0
                && result.timing_drift_samples_per_second < 64.0,
            "estimated timing drift: {}",
            result.timing_drift_samples_per_second
        );
    }

    #[test]
    fn rx_tracks_a_quadratic_sample_clock_drift() {
        let mode = crate::Js8Mode::Normal;
        let tones = crate::encode_tones("TIMECURV12AB", 4, mode).unwrap();
        let mut samples =
            crate::synthesize_with_timing_curve(&tones, mode, 1500.0, 0.0, 4.0).unwrap();
        samples.resize(samples.len() + mode.samples_per_symbol() - 1, 0.0);
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let result = decode_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 15.0,
                frequency_step_hz: 0.5,
                max_fec_iterations: 30,
            },
        )
        .unwrap();
        assert_eq!(result.frame.message, "TIMECURV12AB");
        assert!(result.timing_curvature_samples_per_second2.is_finite());
    }

    #[test]
    fn rx_accepts_one_symbol_of_leading_audio() {
        let audio = encode_audio_block("0123456789AB", Js8TxConfig::default()).unwrap();
        let mut samples = vec![0.0; 11];
        samples.extend_from_slice(&audio.samples);
        samples.resize(samples.len() + 1_919, 0.0);
        let window = AudioBlock::new(12_000, samples).unwrap();
        let batch = decode_audio_block(&window, Js8RxConfig::default()).unwrap();
        assert_eq!(batch.events[0].message, "0123456789AB");
        assert_eq!(batch.events[0].delta_time_seconds, Some(11.0 / 12_000.0));
    }

    #[test]
    fn slot_metadata_matches_mode_duration() {
        let spec = slot_spec(crate::Js8Mode::Turbo);
        assert_eq!(spec.slot.as_secs(), 6);
        assert_eq!(spec.samples_for(spec.slot), 72_000);
    }

    #[test]
    fn round_trips_every_supported_mode_and_frame_type() {
        for (mode, frequency) in [
            (crate::Js8Mode::Normal, 1498.0),
            (crate::Js8Mode::Fast, 1501.5),
            (crate::Js8Mode::Turbo, 1503.0),
            (crate::Js8Mode::Slow, 1497.5),
            (crate::Js8Mode::Ultra, 1504.0),
        ] {
            let tx = Js8TxConfig {
                mode,
                base_frequency_hz: frequency,
                frame_type: 7,
            };
            let audio = encode_audio_block("CQTEST1234AB", tx).unwrap();
            let rx = Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                ..Js8RxConfig::default()
            };
            let result = decode_audio_block_detailed(&audio, rx).unwrap();
            assert_eq!(result.frame.message, "CQTEST1234AB");
            assert_eq!(result.frame.frame_type, 7);
            assert!((result.event.audio_frequency_hz.unwrap() - frequency).abs() < 0.26);
        }
    }

    #[test]
    fn rejects_bad_adapter_inputs_before_or_during_decode() {
        let audio = encode_audio_block("0123456789AB", Js8TxConfig::default()).unwrap();
        let wrong_rate = AudioBlock::new(48_000, audio.samples.clone()).unwrap();
        assert!(matches!(
            decode_audio_block_detailed(&wrong_rate, Js8RxConfig::default()),
            Err(Js8AdapterError::UnsupportedSampleRate { actual: 48_000 })
        ));

        let invalid = Js8RxConfig {
            frequency_half_width_hz: -0.1,
            ..Js8RxConfig::default()
        };
        assert!(matches!(
            decode_audio_block_detailed(&audio, invalid),
            Err(Js8AdapterError::Decode(
                Js8DecodeError::InvalidFrequencySearchRange
            ))
        ));

        let invalid = Js8RxConfig {
            frequency_step_hz: 0.0,
            ..Js8RxConfig::default()
        };
        assert!(matches!(
            decode_audio_block_detailed(&audio, invalid),
            Err(Js8AdapterError::Decode(
                Js8DecodeError::InvalidFrequencySearchStep
            ))
        ));

        let invalid = Js8RxConfig {
            max_fec_iterations: 0,
            ..Js8RxConfig::default()
        };
        assert!(matches!(
            decode_audio_block_detailed(&audio, invalid),
            Err(Js8AdapterError::Decode(
                Js8DecodeError::InvalidFecIterations
            ))
        ));

        let short = AudioBlock::new(12_000, vec![0.0; 79 * 1_920 - 1]).unwrap();
        assert!(matches!(
            decode_audio_block_detailed(&short, Js8RxConfig::default()),
            Err(Js8AdapterError::Decode(
                Js8DecodeError::InvalidTimingWindow { .. }
            ))
        ));
    }

    #[test]
    fn recovers_several_timing_offsets_in_each_mode() {
        for mode in [
            crate::Js8Mode::Normal,
            crate::Js8Mode::Fast,
            crate::Js8Mode::Turbo,
            crate::Js8Mode::Slow,
            crate::Js8Mode::Ultra,
        ] {
            let tx = Js8TxConfig {
                mode,
                base_frequency_hz: 1502.0,
                frame_type: 2,
            };
            let audio = encode_audio_block("OFFSETTEST12", tx).unwrap();
            for leading in [
                11,
                137,
                mode.samples_per_symbol() / 2,
                mode.samples_per_symbol() - 1,
            ] {
                let mut samples = vec![0.0; leading];
                samples.extend_from_slice(&audio.samples);
                samples.resize(audio.samples.len() + mode.samples_per_symbol() - 1, 0.0);
                let window = AudioBlock::new(12_000, samples).unwrap();
                let result = decode_audio_block_detailed(
                    &window,
                    Js8RxConfig {
                        mode,
                        center_frequency_hz: 1500.0,
                        ..Js8RxConfig::default()
                    },
                )
                .unwrap();
                assert_eq!(result.frame.message, "OFFSETTEST12");
                let recovered = result.event.delta_time_seconds.unwrap() * 12_000.0;
                assert!((recovered - leading as f32).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn decodes_deterministic_low_level_noise_without_changing_the_message() {
        let tx = Js8TxConfig {
            mode: crate::Js8Mode::Fast,
            base_frequency_hz: 1502.0,
            frame_type: 6,
        };
        let clean = encode_audio_block("NOISECHECK12", tx).unwrap();
        for amplitude in [0.0_f32, 0.0005, 0.001] {
            let mut state = 0x1234_5678_u32;
            let samples = clean
                .samples
                .iter()
                .map(|&sample| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    sample + amplitude * noise
                })
                .collect();
            let audio = AudioBlock::new(12_000, samples).unwrap();
            let result = decode_audio_block_detailed(
                &audio,
                Js8RxConfig {
                    mode: tx.mode,
                    center_frequency_hz: 1500.0,
                    ..Js8RxConfig::default()
                },
            )
            .unwrap();
            assert_eq!(result.frame.message, "NOISECHECK12");
            assert_eq!(result.frame.frame_type, 6);
        }
    }
}
