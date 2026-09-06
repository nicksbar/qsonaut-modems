use std::time::Instant;

use qsonaut_modems::{AudioBlock, DecodeBatch, DecodeEvent};

use crate::{decode_audio_block_detailed, Js8AdapterError, Js8RxConfig, Js8RxResult};

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
    let mut candidate = scan_config.start_sample;
    for _ in 0..scan_config.max_candidates {
        let Some(end) = candidate.checked_add(search_window) else {
            break;
        };
        if end > audio.samples.len() {
            break;
        }
        let window = AudioBlock::new(
            crate::SAMPLE_RATE_HZ,
            audio.samples[candidate..end].to_vec(),
        )?;
        let sync_half_width = if scan_config.sync_frequency_half_width_hz == 0.0 {
            rx_config.frequency_half_width_hz
        } else {
            scan_config.sync_frequency_half_width_hz
        };
        let candidate_count =
            (2.0 * sync_half_width / scan_config.sync_frequency_step_hz).floor() as usize;
        let lower_frequency = rx_config.center_frequency_hz - sync_half_width;
        let mut best_frequency = rx_config.center_frequency_hz;
        let mut best_quality = 0.0_f32;
        for index in 0..=candidate_count {
            let frequency = lower_frequency + index as f32 * scan_config.sync_frequency_step_hz;
            let (_, quality) = crate::sync::find_coarse_boundary_with_quality(
                &window.samples,
                rx_config.mode,
                frequency,
            )?;
            if quality > best_quality {
                best_quality = quality;
                best_frequency = frequency;
            }
        }
        let quality = best_quality;
        if quality < scan_config.minimum_sync_quality {
            let Some(next) = candidate.checked_add(scan_config.step_samples) else {
                break;
            };
            candidate = next;
            continue;
        }
        let decode_config = Js8RxConfig {
            center_frequency_hz: best_frequency,
            frequency_half_width_hz: rx_config
                .frequency_step_hz
                .max(scan_config.sync_frequency_step_hz)
                * 2.0,
            ..rx_config
        };
        if let Ok(mut result) = decode_audio_block_detailed(&window, decode_config) {
            let local_offset =
                result.event.delta_time_seconds.unwrap_or_default() * crate::SAMPLE_RATE_HZ as f32;
            result.event.delta_time_seconds =
                Some((candidate as f32 + local_offset) / crate::SAMPLE_RATE_HZ as f32);
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
}
