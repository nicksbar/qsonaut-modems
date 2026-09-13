use std::collections::HashSet;
use std::time::Instant;

use qsonaut_modems::{AudioBlock, DecodeBatch, DecodeEvent};

use crate::{
    scan_audio_block_detailed, Js8AdapterError, Js8Mode, Js8RxConfig, Js8RxSession, Js8ScanConfig,
    Js8ScanResult,
};

/// Consumer-selected policy for decoding more than one JS8 speed from the
/// same captured audio window.
#[derive(Debug, Clone, PartialEq)]
pub struct Js8MultiRxConfig {
    pub modes: Vec<Js8Mode>,
    pub center_frequency_hz: f32,
    pub frequency_half_width_hz: f32,
    pub frequency_step_hz: f32,
    pub max_fec_iterations: usize,
}

impl Default for Js8MultiRxConfig {
    fn default() -> Self {
        Self {
            modes: Js8Mode::ALL.to_vec(),
            center_frequency_hz: 1_500.0,
            frequency_half_width_hz: 5.0,
            frequency_step_hz: 0.5,
            max_fec_iterations: 10,
        }
    }
}

impl Js8MultiRxConfig {
    fn for_mode(&self, mode: Js8Mode) -> Js8RxConfig {
        Js8RxConfig {
            mode,
            center_frequency_hz: self.center_frequency_hz,
            frequency_half_width_hz: self.frequency_half_width_hz,
            frequency_step_hz: self.frequency_step_hz,
            max_fec_iterations: self.max_fec_iterations,
        }
    }
}

/// One CRC-valid waterfall result annotated with the speed that decoded it.
#[derive(Debug, Clone, PartialEq)]
pub struct Js8MultiRxResult {
    pub mode: Js8Mode,
    pub scan: Js8ScanResult,
}

/// Chunk-fed JS8 receiver that keeps one bounded decoder session per selected
/// speed.
///
/// Separate sessions let short modes report as soon as their own frame window
/// is ready instead of waiting for the 30-second Slow window. The consumer
/// still owns capture, resampling, clocks, worker lifetime, and cancellation.
#[derive(Debug)]
pub struct Js8MultiRxSession {
    sessions: Vec<(Js8Mode, Js8RxSession)>,
}

impl Js8MultiRxSession {
    /// Create a multi-speed session with a caller-selected scan policy.
    pub fn new(
        config: Js8MultiRxConfig,
        scan_config: Js8ScanConfig,
    ) -> Result<Self, Js8AdapterError> {
        if config.modes.is_empty() {
            return Err(Js8AdapterError::EmptyModeSet);
        }

        let mut seen = HashSet::new();
        let sessions = config
            .modes
            .iter()
            .copied()
            .filter(|mode| seen.insert(*mode))
            .map(|mode| (mode, Js8RxSession::new(config.for_mode(mode), scan_config)))
            .collect();
        Ok(Self { sessions })
    }

    /// Create a multi-speed session using the primary waterfall profile.
    pub fn waterfall(config: Js8MultiRxConfig) -> Result<Self, Js8AdapterError> {
        Self::new(config, Js8ScanConfig::waterfall())
    }

    /// Append the same normalized 12 kHz mono samples to every selected speed.
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<(), Js8AdapterError> {
        for (_, session) in &mut self.sessions {
            session.push_samples(samples)?;
        }
        Ok(())
    }

    /// Poll each selected speed for bounded newly available candidate windows.
    pub fn poll(
        &mut self,
        max_candidates_per_mode: usize,
    ) -> Result<Vec<Js8MultiRxResult>, Js8AdapterError> {
        let mut results = Vec::new();
        for (mode, session) in &mut self.sessions {
            results.extend(
                session
                    .poll(max_candidates_per_mode)?
                    .into_iter()
                    .map(|scan| Js8MultiRxResult { mode: *mode, scan }),
            );
        }
        sort_results(&mut results);
        Ok(results)
    }

    /// Remove all buffered audio and duplicate history from every speed.
    pub fn reset(&mut self) {
        for (_, session) in &mut self.sessions {
            session.reset();
        }
    }

    /// Whether any selected speed has a complete candidate window ready.
    pub fn is_ready(&self) -> bool {
        self.sessions.iter().any(|(_, session)| session.is_ready())
    }
}

/// Scan one captured window at every selected JS8 speed.
///
/// The caller still owns buffering, clock alignment, scheduling, cancellation,
/// and worker parallelism. Duplicate mode entries are ignored.
pub fn scan_audio_block_multi_detailed(
    audio: &AudioBlock,
    config: &Js8MultiRxConfig,
    scan_config: Js8ScanConfig,
) -> Result<Vec<Js8MultiRxResult>, Js8AdapterError> {
    if config.modes.is_empty() {
        return Err(Js8AdapterError::EmptyModeSet);
    }

    let mut seen = HashSet::new();
    let mut results = Vec::new();
    for &mode in &config.modes {
        if !seen.insert(mode) {
            continue;
        }
        results.extend(
            scan_audio_block_detailed(audio, config.for_mode(mode), scan_config)?
                .into_iter()
                .map(|scan| Js8MultiRxResult { mode, scan }),
        );
    }
    sort_results(&mut results);
    Ok(results)
}

fn sort_results(results: &mut [Js8MultiRxResult]) {
    results.sort_by(|left, right| {
        left.scan
            .candidate_sample
            .cmp(&right.scan.candidate_sample)
            .then_with(|| {
                left.scan
                    .result
                    .event
                    .audio_frequency_hz
                    .unwrap_or_default()
                    .total_cmp(
                        &right
                            .scan
                            .result
                            .event
                            .audio_frequency_hz
                            .unwrap_or_default(),
                    )
            })
    });
}

/// Scan multiple JS8 speeds and project the results into generic events.
pub fn scan_audio_block_multi(
    audio: &AudioBlock,
    config: &Js8MultiRxConfig,
    scan_config: Js8ScanConfig,
) -> Result<DecodeBatch, Js8AdapterError> {
    let started = Instant::now();
    let events: Vec<DecodeEvent> = scan_audio_block_multi_detailed(audio, config, scan_config)?
        .into_iter()
        .map(|result| result.scan.result.event)
        .collect();
    Ok(DecodeBatch::finish(audio.samples.len(), started, events))
}

#[cfg(test)]
mod tests {
    use super::{scan_audio_block_multi_detailed, Js8MultiRxConfig, Js8MultiRxSession};
    use crate::{encode_audio_block, Js8AdapterError, Js8Mode, Js8ScanConfig, Js8TxConfig};
    use qsonaut_modems::AudioBlock;

    #[test]
    fn decodes_selected_speeds_and_reports_the_matching_mode() {
        let channels = [
            (Js8Mode::Fast, "MULTIFAST123", 1_300.0),
            (Js8Mode::Ultra, "MULTI60ABCDE", 1_900.0),
        ];
        let frames: Vec<_> = channels
            .iter()
            .map(|(mode, message, frequency)| {
                encode_audio_block(
                    message,
                    Js8TxConfig {
                        mode: *mode,
                        base_frequency_hz: *frequency,
                        frame_type: 4,
                    },
                )
                .unwrap()
            })
            .collect();
        let sample_count = frames
            .iter()
            .zip(channels)
            .map(|(frame, (mode, _, _))| frame.samples.len() + mode.samples_per_symbol() - 1)
            .max()
            .unwrap();
        let mut samples = vec![0.0; sample_count];
        for frame in frames {
            for (mixed, sample) in samples.iter_mut().zip(frame.samples) {
                *mixed += sample * 0.45;
            }
        }
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let config = Js8MultiRxConfig {
            modes: vec![Js8Mode::Fast, Js8Mode::Ultra, Js8Mode::Fast],
            center_frequency_hz: 1_500.0,
            frequency_half_width_hz: 5.0,
            frequency_step_hz: 0.5,
            max_fec_iterations: 10,
        };
        let results = scan_audio_block_multi_detailed(
            &audio,
            &config,
            Js8ScanConfig {
                max_candidates: 1,
                step_samples: 1,
                dedup_samples: 0,
                minimum_sync_quality: 0.0,
                waterfall_frequency_range_hz: Some((1_100.0, 2_100.0)),
                sync_frequency_step_hz: 10.0,
                max_frequency_hypotheses: 4,
                max_signals_per_window: 1,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|result| {
            result.mode == Js8Mode::Fast && result.scan.result.frame.message == "MULTIFAST123"
        }));
        assert!(results.iter().any(|result| {
            result.mode == Js8Mode::Ultra && result.scan.result.frame.message == "MULTI60ABCDE"
        }));
    }

    #[test]
    fn rejects_an_empty_mode_set() {
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, vec![0.0; 40_000]).unwrap();
        let config = Js8MultiRxConfig {
            modes: Vec::new(),
            ..Js8MultiRxConfig::default()
        };
        assert!(matches!(
            scan_audio_block_multi_detailed(&audio, &config, Js8ScanConfig::default()),
            Err(Js8AdapterError::EmptyModeSet)
        ));
        assert!(matches!(
            Js8MultiRxSession::waterfall(config),
            Err(Js8AdapterError::EmptyModeSet)
        ));
    }

    #[test]
    fn streaming_session_reports_short_modes_without_waiting_for_slow() {
        let tx = encode_audio_block(
            "STREAMFAST12",
            Js8TxConfig {
                mode: Js8Mode::Fast,
                base_frequency_hz: 1_500.0,
                frame_type: 4,
            },
        )
        .unwrap();
        let mut session = Js8MultiRxSession::new(
            Js8MultiRxConfig {
                modes: vec![Js8Mode::Slow, Js8Mode::Fast],
                ..Js8MultiRxConfig::default()
            },
            Js8ScanConfig {
                step_samples: 12_000,
                max_candidates: 1,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();

        session.push_samples(&tx.samples).unwrap();
        assert!(session.is_ready());
        let results = session.poll(1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].mode, Js8Mode::Fast);
        assert_eq!(results[0].scan.result.frame.message, "STREAMFAST12");
    }

    #[test]
    fn decodes_unequal_power_speeds_with_overlapping_passbands() {
        let channels = [
            (Js8Mode::Fast, "WEAKFAST2026", 1_450.0, 0.22),
            (Js8Mode::Ultra, "STRONG602026", 1_500.0, 0.75),
        ];
        let frames: Vec<_> = channels
            .iter()
            .map(|(mode, message, frequency, _)| {
                encode_audio_block(
                    message,
                    Js8TxConfig {
                        mode: *mode,
                        base_frequency_hz: *frequency,
                        frame_type: 4,
                    },
                )
                .unwrap()
            })
            .collect();
        let sample_count = frames
            .iter()
            .zip(channels)
            .map(|(frame, (mode, _, _, _))| frame.samples.len() + mode.samples_per_symbol() - 1)
            .max()
            .unwrap();
        let mut samples = vec![0.0; sample_count];
        for (frame, (_, _, _, gain)) in frames.into_iter().zip(channels) {
            for (mixed, sample) in samples.iter_mut().zip(frame.samples) {
                *mixed += sample * gain;
            }
        }
        let audio = AudioBlock::new(crate::SAMPLE_RATE_HZ, samples).unwrap();
        let results = scan_audio_block_multi_detailed(
            &audio,
            &Js8MultiRxConfig {
                modes: vec![Js8Mode::Fast, Js8Mode::Ultra],
                ..Js8MultiRxConfig::default()
            },
            Js8ScanConfig {
                max_candidates: 1,
                step_samples: 1,
                dedup_samples: 0,
                minimum_sync_quality: 0.0,
                waterfall_frequency_range_hz: Some((1_200.0, 1_900.0)),
                sync_frequency_step_hz: 10.0,
                max_frequency_hypotheses: 8,
                max_signals_per_window: 1,
                ..Js8ScanConfig::default()
            },
        )
        .unwrap();

        assert!(results.iter().any(|result| {
            result.mode == Js8Mode::Fast && result.scan.result.frame.message == "WEAKFAST2026"
        }));
        assert!(results.iter().any(|result| {
            result.mode == Js8Mode::Ultra && result.scan.result.frame.message == "STRONG602026"
        }));
    }
}
