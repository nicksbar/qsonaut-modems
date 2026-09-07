use std::f32::consts::FRAC_PI_2;
use std::time::Instant;

use qsonaut_modems::{AudioBlock, DecodeBatch, DecodeEvent};

use crate::{
    decode_audio_block_detailed, encode_tones, synthesize_with_frequency_curve_phase,
    Js8AdapterError, Js8Mode, Js8RxConfig, Js8RxResult,
};

/// Consumer-controlled search policy for a complete recording or rolling
/// buffer. The scanner does not own slot clocks or worker scheduling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Js8ScanConfig {
    /// First candidate frame position in 12 kHz samples.
    pub start_sample: usize,
    /// Distance between candidate frame positions.
    pub step_samples: usize,
    /// Maximum number of candidate windows to attempt.
    pub max_candidates: usize,
    /// Suppress the same message when it reappears within this distance.
    pub dedup_samples: usize,
    /// Minimum normalized Costas quality before attempting frequency/FEC.
    pub minimum_sync_quality: f32,
    /// Coarse carrier search half-width. Zero reuses the RX search width.
    pub sync_frequency_half_width_hz: f32,
    /// Coarse carrier search spacing in hertz.
    pub sync_frequency_step_hz: f32,
    /// Optional absolute waterfall frequency range `(low, high)` in hertz.
    /// When absent, the RX center frequency and scan half-width are used.
    pub waterfall_frequency_range_hz: Option<(f32, f32)>,
    /// Maximum ranked carrier hypotheses retained per candidate window.
    pub max_frequency_hypotheses: usize,
    /// Maximum decoded signals extracted from one candidate window.
    pub max_signals_per_window: usize,
}

impl Default for Js8ScanConfig {
    fn default() -> Self {
        Self {
            start_sample: 0,
            step_samples: 12_000,
            max_candidates: 256,
            dedup_samples: 79 * 1_920,
            minimum_sync_quality: 0.01,
            sync_frequency_half_width_hz: 0.0,
            sync_frequency_step_hz: 5.0,
            waterfall_frequency_range_hz: None,
            max_frequency_hypotheses: 3,
            max_signals_per_window: 2,
        }
    }
}

impl Js8ScanConfig {
    /// Return the recommended profile for primary waterfall-wide operation.
    ///
    /// The band covers the normal JS8 audio passband and the limits remain
    /// bounded so callers can use this profile from a worker with explicit
    /// polling and cancellation opportunities.
    pub fn waterfall() -> Self {
        Self {
            sync_frequency_step_hz: 10.0,
            waterfall_frequency_range_hz: Some((200.0, 3_000.0)),
            max_frequency_hypotheses: 8,
            max_signals_per_window: 4,
            ..Self::default()
        }
    }
}

/// Protocol-specific recording results plus their generic event projection.
#[derive(Debug, Clone, PartialEq)]
pub struct Js8ScanResult {
    pub result: Js8RxResult,
    pub candidate_sample: usize,
    /// Coarse Costas quality measured before exact decode.
    pub sync_quality: f32,
}

/// Scan candidate windows and retain successful JS8 decodes.
pub fn scan_audio_block_detailed(
    audio: &AudioBlock,
    rx_config: Js8RxConfig,
    scan_config: Js8ScanConfig,
) -> Result<Vec<Js8ScanResult>, Js8AdapterError> {
    validate_scan_config(scan_config)?;
    let frame_samples = 79 * rx_config.mode.samples_per_symbol();
    let search_window = frame_samples + rx_config.mode.samples_per_symbol() - 1;
    if audio.sample_rate_hz != crate::SAMPLE_RATE_HZ {
        return Err(Js8AdapterError::UnsupportedSampleRate {
            actual: audio.sample_rate_hz,
        });
    }
    if audio.samples.len() < search_window || scan_config.start_sample >= audio.samples.len() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut working_samples = audio.samples.to_vec();
    let mut candidate = scan_config.start_sample;
    for _ in 0..scan_config.max_candidates {
        let Some(end) = candidate.checked_add(search_window) else {
            break;
        };
        if end > audio.samples.len() {
            break;
        }
        let mut residual = working_samples[candidate..end].to_vec();
        let mut decoded_frequencies: Vec<f32> =
            Vec::with_capacity(scan_config.max_signals_per_window);
        for _ in 0..scan_config.max_signals_per_window {
            let (lower_frequency, upper_frequency) =
                waterfall_frequency_bounds(rx_config, scan_config);
            let frequency_hypotheses = crate::sync::find_waterfall_frequency_hypotheses(
                &residual,
                rx_config.mode,
                lower_frequency,
                upper_frequency,
                scan_config.sync_frequency_step_hz,
                scan_config.max_frequency_hypotheses,
            )?;
            let frequency_hypotheses: Vec<_> = frequency_hypotheses
                .into_iter()
                .filter(|(_, frequency)| {
                    decoded_frequencies
                        .iter()
                        .all(|previous| (frequency - previous).abs() > 30.0)
                })
                .collect();
            let Some(&(best_quality, _)) = frequency_hypotheses.first() else {
                break;
            };
            if best_quality < scan_config.minimum_sync_quality {
                break;
            }

            let window = AudioBlock::new(crate::SAMPLE_RATE_HZ, residual.clone())?;
            let mut decoded = None;
            for (quality, frequency) in frequency_hypotheses {
                let decode_config = Js8RxConfig {
                    center_frequency_hz: frequency,
                    frequency_half_width_hz: rx_config
                        .frequency_step_hz
                        .max(scan_config.sync_frequency_step_hz)
                        * 2.0,
                    ..rx_config
                };
                if let Ok(result) = decode_audio_block_detailed(&window, decode_config) {
                    let rank = (quality, result.event.snr_db.unwrap_or(f32::NEG_INFINITY));
                    if decoded
                        .as_ref()
                        .is_none_or(|(_, _, previous_rank)| rank > *previous_rank)
                    {
                        decoded = Some((result, quality, rank));
                    }
                }
            }
            let Some((mut result, quality, _)) = decoded else {
                break;
            };
            let local_offset =
                result.event.delta_time_seconds.unwrap_or_default() * crate::SAMPLE_RATE_HZ as f32;
            result.event.delta_time_seconds =
                Some((candidate as f32 + local_offset) / crate::SAMPLE_RATE_HZ as f32);
            let cancel_offset = local_offset.round() as isize;
            if let Some(audio_frequency_hz) = result.event.audio_frequency_hz {
                decoded_frequencies.push(audio_frequency_hz);
            }
            if !subtract_decoded_signal(&mut residual, &result, cancel_offset, rx_config.mode) {
                break;
            }
            working_samples[candidate..end].copy_from_slice(&residual);
            if let Some(previous) = results.iter_mut().find(|previous: &&mut Js8ScanResult| {
                previous.result.frame.message == result.frame.message
                    && candidate.abs_diff(previous.candidate_sample) <= scan_config.dedup_samples
            }) {
                let previous_rank = (
                    previous.sync_quality,
                    previous.result.event.snr_db.unwrap_or(f32::NEG_INFINITY),
                );
                let candidate_rank = (quality, result.event.snr_db.unwrap_or(f32::NEG_INFINITY));
                if candidate_rank > previous_rank {
                    previous.result = result;
                    previous.candidate_sample = candidate;
                    previous.sync_quality = quality;
                }
            } else {
                results.push(Js8ScanResult {
                    result,
                    candidate_sample: candidate,
                    sync_quality: quality,
                });
            }
        }
        let Some(next) = candidate.checked_add(scan_config.step_samples) else {
            break;
        };
        candidate = next;
    }
    Ok(results)
}

pub(crate) fn subtract_decoded_signal(
    residual: &mut [f32],
    result: &Js8RxResult,
    offset: isize,
    mode: Js8Mode,
) -> bool {
    let Ok(tones) = encode_tones(&result.frame.message, result.frame.frame_type, mode) else {
        return false;
    };
    let Some(base_frequency_hz) = result.event.audio_frequency_hz else {
        return false;
    };
    let Ok(reference) = synthesize_with_frequency_curve_phase(
        &tones,
        mode,
        base_frequency_hz,
        result.frequency_drift_hz_per_second,
        result.frequency_curvature_hz_per_second2,
        0.0,
    ) else {
        return false;
    };
    let Ok(quadrature) = synthesize_with_frequency_curve_phase(
        &tones,
        mode,
        base_frequency_hz,
        result.frequency_drift_hz_per_second,
        result.frequency_curvature_hz_per_second2,
        FRAC_PI_2,
    ) else {
        return false;
    };
    let mut sum_cc = 0.0_f32;
    let mut sum_cq = 0.0_f32;
    let mut sum_qq = 0.0_f32;
    let mut sum_rc = 0.0_f32;
    let mut sum_rq = 0.0_f32;
    let mut count = 0;
    for (index, &sample) in reference.iter().enumerate() {
        let target = index as isize + offset;
        if let Ok(target) = usize::try_from(target) {
            if let Some(&value) = residual.get(target) {
                let quadrature = quadrature[index];
                sum_cc += sample * sample;
                sum_cq += sample * quadrature;
                sum_qq += quadrature * quadrature;
                sum_rc += value * sample;
                sum_rq += value * quadrature;
                count += 1;
            }
        }
    }
    let determinant = sum_cc * sum_qq - sum_cq * sum_cq;
    if count < reference.len() / 2 || determinant.abs() < 1e-6 {
        return false;
    }
    let coefficient_c = (sum_rc * sum_qq - sum_rq * sum_cq) / determinant;
    let coefficient_q = (sum_rq * sum_cc - sum_rc * sum_cq) / determinant;
    if !coefficient_c.is_finite() || !coefficient_q.is_finite() {
        return false;
    }
    for (index, &sample) in reference.iter().enumerate() {
        let target = index as isize + offset;
        if let Ok(target) = usize::try_from(target) {
            if let Some(value) = residual.get_mut(target) {
                *value -= coefficient_c * sample + coefficient_q * quadrature[index];
            }
        }
    }
    true
}

#[cfg(test)]
fn retain_frequency_hypothesis(
    hypotheses: &mut Vec<(f32, f32)>,
    quality: f32,
    frequency: f32,
    max_hypotheses: usize,
) {
    if let Some(existing) = hypotheses
        .iter_mut()
        .find(|(_, candidate)| *candidate == frequency)
    {
        existing.0 = existing.0.max(quality);
    } else {
        hypotheses.push((quality, frequency));
    }
    hypotheses.sort_unstable_by(|left, right| right.0.total_cmp(&left.0));
    hypotheses.truncate(max_hypotheses);
}

fn waterfall_frequency_bounds(rx_config: Js8RxConfig, scan_config: Js8ScanConfig) -> (f32, f32) {
    scan_config.waterfall_frequency_range_hz.unwrap_or_else(|| {
        let half_width = if scan_config.sync_frequency_half_width_hz == 0.0 {
            rx_config.frequency_half_width_hz
        } else {
            scan_config.sync_frequency_half_width_hz
        };
        (
            rx_config.center_frequency_hz - half_width,
            rx_config.center_frequency_hz + half_width,
        )
    })
}

/// Scan a recording and return the generic decode batch used by consumers.
pub fn scan_audio_block(
    audio: &AudioBlock,
    rx_config: Js8RxConfig,
    scan_config: Js8ScanConfig,
) -> Result<DecodeBatch, Js8AdapterError> {
    let started = Instant::now();
    let results = scan_audio_block_detailed(audio, rx_config, scan_config)?;
    let events: Vec<DecodeEvent> = results
        .into_iter()
        .map(|result| result.result.event)
        .collect();
    Ok(DecodeBatch::finish(audio.samples.len(), started, events))
}

fn validate_scan_config(config: Js8ScanConfig) -> Result<(), Js8AdapterError> {
    if config.step_samples == 0 {
        return Err(Js8AdapterError::InvalidScanStep);
    }
    if config.max_candidates == 0 {
        return Err(Js8AdapterError::InvalidScanLimit);
    }
    if !config.minimum_sync_quality.is_finite() || config.minimum_sync_quality < 0.0 {
        return Err(Js8AdapterError::InvalidSyncQuality);
    }
    if !config.sync_frequency_half_width_hz.is_finite() || config.sync_frequency_half_width_hz < 0.0
    {
        return Err(Js8AdapterError::InvalidSyncFrequencyRange);
    }
    if !config.sync_frequency_step_hz.is_finite() || config.sync_frequency_step_hz <= 0.0 {
        return Err(Js8AdapterError::InvalidSyncFrequencyStep);
    }
    if config.max_frequency_hypotheses == 0 || config.max_signals_per_window == 0 {
        return Err(Js8AdapterError::InvalidScanLimit);
    }
    if let Some((lower, upper)) = config.waterfall_frequency_range_hz {
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return Err(Js8AdapterError::InvalidSyncFrequencyRange);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{scan_audio_block, scan_audio_block_detailed, Js8ScanConfig};
    use crate::{encode_audio_block, Js8AdapterError, Js8Mode, Js8RxConfig, Js8TxConfig};
    use qsonaut_modems::AudioBlock;

    #[test]
    fn scans_multiple_candidate_windows_and_deduplicates_hits() {
        let tx = Js8TxConfig {
            mode: Js8Mode::Ultra,
            base_frequency_hz: 1501.0,
            frame_type: 4,
        };
        let frame = encode_audio_block("SCANFIRST123", tx).unwrap();
        let mut samples = frame.samples.clone();
        samples.resize(36_000, 0.0);
        samples.extend_from_slice(&frame.samples);
        samples.resize(samples.len() + 2_000, 0.0);
        let audio = AudioBlock::new(12_000, samples).unwrap();
        let scan = Js8ScanConfig {
            step_samples: 12_000,
            max_candidates: 8,
            dedup_samples: 1_000,
            ..Js8ScanConfig::default()
        };
        let results = scan_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode: Js8Mode::Ultra,
                center_frequency_hz: 1500.0,
                ..Js8RxConfig::default()
            },
            scan,
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].result.frame.message, "SCANFIRST123");
        assert!(results[0].sync_quality >= scan.minimum_sync_quality);
        assert_eq!(results[1].candidate_sample, 36_000);
        assert_eq!(
            scan_audio_block(
                &audio,
                Js8RxConfig {
                    mode: Js8Mode::Ultra,
                    center_frequency_hz: 1500.0,
                    ..Js8RxConfig::default()
                },
                scan,
            )
            .unwrap()
            .events
            .len(),
            2
        );
    }

    #[test]
    fn silence_is_a_clean_no_decode_result() {
        let audio = AudioBlock::new(12_000, vec![0.0; 40_000]).unwrap();
        let results = scan_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode: Js8Mode::Ultra,
                ..Js8RxConfig::default()
            },
            Js8ScanConfig {
                step_samples: 12_000,
                max_candidates: 4,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn rejects_unbounded_scan_policies() {
        let audio = AudioBlock::new(12_000, vec![0.0; 40_000]).unwrap();
        assert!(matches!(
            scan_audio_block_detailed(
                &audio,
                Js8RxConfig::default(),
                Js8ScanConfig {
                    step_samples: 0,
                    ..Js8ScanConfig::default()
                }
            ),
            Err(Js8AdapterError::InvalidScanStep)
        ));
        assert!(matches!(
            scan_audio_block_detailed(
                &audio,
                Js8RxConfig::default(),
                Js8ScanConfig {
                    max_candidates: 0,
                    ..Js8ScanConfig::default()
                }
            ),
            Err(Js8AdapterError::InvalidScanLimit)
        ));
        assert!(matches!(
            scan_audio_block_detailed(
                &audio,
                Js8RxConfig::default(),
                Js8ScanConfig {
                    minimum_sync_quality: f32::NAN,
                    ..Js8ScanConfig::default()
                }
            ),
            Err(Js8AdapterError::InvalidSyncQuality)
        ));
        assert!(matches!(
            scan_audio_block_detailed(
                &audio,
                Js8RxConfig::default(),
                Js8ScanConfig {
                    sync_frequency_step_hz: 0.0,
                    ..Js8ScanConfig::default()
                }
            ),
            Err(Js8AdapterError::InvalidSyncFrequencyStep)
        ));
    }

    #[test]
    fn retains_the_strongest_distinct_frequency_hypotheses() {
        let mut hypotheses = Vec::new();
        super::retain_frequency_hypothesis(&mut hypotheses, 0.2, 1500.0, 3);
        super::retain_frequency_hypothesis(&mut hypotheses, 0.8, 1510.0, 3);
        super::retain_frequency_hypothesis(&mut hypotheses, 0.5, 1490.0, 3);
        super::retain_frequency_hypothesis(&mut hypotheses, 0.9, 1520.0, 3);
        super::retain_frequency_hypothesis(&mut hypotheses, 0.95, 1510.0, 3);
        assert_eq!(
            hypotheses,
            vec![(0.95, 1510.0), (0.9, 1520.0), (0.5, 1490.0)]
        );
    }

    #[test]
    fn waterfall_profile_is_broad_but_bounded() {
        let config = Js8ScanConfig::waterfall();
        assert_eq!(config.waterfall_frequency_range_hz, Some((200.0, 3_000.0)));
        assert_eq!(config.max_frequency_hypotheses, 8);
        assert_eq!(config.max_signals_per_window, 4);
        assert!(config.sync_frequency_step_hz > 0.0);
    }

    #[test]
    fn subtracts_a_decoded_signal_to_recover_an_overlapping_frame() {
        let first = encode_audio_block(
            "OVERLAPONE12",
            Js8TxConfig {
                mode: Js8Mode::Ultra,
                base_frequency_hz: 1500.0,
                frame_type: 3,
            },
        )
        .unwrap();
        let second = encode_audio_block(
            "OVERLAPTWO12",
            Js8TxConfig {
                mode: Js8Mode::Ultra,
                base_frequency_hz: 1540.0,
                frame_type: 4,
            },
        )
        .unwrap();
        let mut samples: Vec<f32> = first
            .samples
            .iter()
            .zip(&second.samples)
            .map(|(&left, &right)| left + 0.25 * right)
            .collect();
        samples.resize(samples.len() + Js8Mode::Ultra.samples_per_symbol() - 1, 0.0);
        let audio = AudioBlock::new(12_000, samples).unwrap();
        let results = scan_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode: Js8Mode::Ultra,
                center_frequency_hz: 1520.0,
                frequency_half_width_hz: 50.0,
                frequency_step_hz: 0.5,
                ..Js8RxConfig::default()
            },
            Js8ScanConfig {
                max_candidates: 1,
                step_samples: 1,
                dedup_samples: 0,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();
        let mut messages: Vec<_> = results
            .iter()
            .map(|result| result.result.frame.message.as_str())
            .collect();
        messages.sort_unstable();
        assert_eq!(messages, vec!["OVERLAPONE12", "OVERLAPTWO12"]);
    }

    #[test]
    fn decodes_three_channels_across_a_waterfall_band() {
        let channels = [
            ("WATERFALLA12", 1200.0, 1.0),
            ("WATERFALLB12", 1500.0, 0.45),
            ("WATERFALLC12", 1800.0, 0.25),
        ];
        let frames: Vec<_> = channels
            .iter()
            .map(|(message, frequency, _)| {
                encode_audio_block(
                    message,
                    Js8TxConfig {
                        mode: Js8Mode::Ultra,
                        base_frequency_hz: *frequency,
                        frame_type: 4,
                    },
                )
                .unwrap()
            })
            .collect();
        let samples: Vec<f32> = (0..frames[0].samples.len())
            .map(|index| {
                frames
                    .iter()
                    .zip(channels)
                    .map(|(frame, (_, _, amplitude))| frame.samples[index] * amplitude)
                    .sum()
            })
            .collect();
        let mut samples = samples;
        samples.resize(samples.len() + Js8Mode::Ultra.samples_per_symbol() - 1, 0.0);
        let audio = AudioBlock::new(12_000, samples).unwrap();
        let results = scan_audio_block_detailed(
            &audio,
            Js8RxConfig {
                mode: Js8Mode::Ultra,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 5.0,
                frequency_step_hz: 0.5,
                ..Js8RxConfig::default()
            },
            Js8ScanConfig {
                max_candidates: 1,
                step_samples: 1,
                dedup_samples: 0,
                minimum_sync_quality: 0.0,
                waterfall_frequency_range_hz: Some((200.0, 3_000.0)),
                sync_frequency_step_hz: 10.0,
                max_frequency_hypotheses: 8,
                max_signals_per_window: 3,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();
        let mut messages: Vec<_> = results
            .iter()
            .map(|result| result.result.frame.message.as_str())
            .collect();
        messages.sort_unstable();
        assert_eq!(
            messages,
            vec!["WATERFALLA12", "WATERFALLB12", "WATERFALLC12"]
        );
    }
}
