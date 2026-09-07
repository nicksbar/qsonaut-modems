use std::collections::VecDeque;

use qsonaut_modems::AudioBlock;

use crate::{
    scan::{scan_audio_block_detailed, subtract_decoded_signal},
    Js8AdapterError, Js8RxConfig, Js8ScanConfig, Js8ScanResult, SAMPLE_RATE_HZ,
};

/// Chunk-fed, bounded JS8 receiver state for a consumer-owned modem worker.
///
/// This type owns only JS8 sample buffering and candidate progression. The
/// consumer remains responsible for capture, resampling, clocks, slot policy,
/// worker lifetime, cancellation, and what to do with returned results.
#[derive(Debug)]
pub struct Js8RxSession {
    rx_config: Js8RxConfig,
    scan_config: Js8ScanConfig,
    samples: VecDeque<f32>,
    buffer_start_sample: usize,
    total_samples_received: usize,
    next_candidate_sample: usize,
    capacity_samples: usize,
    frame_samples: usize,
    search_window_samples: usize,
    reported_results: Vec<(String, usize)>,
}

impl Js8RxSession {
    /// Create a streaming receiver using the bounded waterfall-wide profile.
    pub fn waterfall(rx_config: Js8RxConfig) -> Self {
        Self::new(rx_config, Js8ScanConfig::waterfall())
    }

    /// Create a session with enough capacity for one nominal JS8 slot.
    pub fn new(rx_config: Js8RxConfig, scan_config: Js8ScanConfig) -> Self {
        let frame_samples = 79 * rx_config.mode.samples_per_symbol();
        let search_window_samples = frame_samples + rx_config.mode.samples_per_symbol() - 1;
        let capacity_samples = (rx_config.mode.tx_seconds().saturating_mul(SAMPLE_RATE_HZ)
            as usize)
            .max(search_window_samples);
        Self::with_capacity_samples(rx_config, scan_config, capacity_samples)
    }

    /// Create a session with an explicit bounded sample capacity.
    ///
    /// A capacity smaller than one candidate window is increased to one
    /// candidate window. If old samples are evicted before polling, candidate
    /// positions that can no longer be represented are skipped safely.
    pub fn with_capacity_samples(
        rx_config: Js8RxConfig,
        scan_config: Js8ScanConfig,
        capacity_samples: usize,
    ) -> Self {
        let frame_samples = 79 * rx_config.mode.samples_per_symbol();
        let search_window_samples = frame_samples + rx_config.mode.samples_per_symbol() - 1;
        let capacity_samples = capacity_samples.max(search_window_samples);
        Self {
            rx_config,
            next_candidate_sample: scan_config.start_sample,
            scan_config,
            samples: VecDeque::with_capacity(capacity_samples),
            buffer_start_sample: 0,
            total_samples_received: 0,
            capacity_samples,
            frame_samples,
            search_window_samples,
            reported_results: Vec::new(),
        }
    }

    /// Append normalized mono samples at exactly 12 kHz.
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<(), Js8AdapterError> {
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(Js8AdapterError::Audio(
                qsonaut_modems::AudioError::NonFiniteSample,
            ));
        }
        self.samples.extend(samples.iter().copied());
        self.total_samples_received = self.total_samples_received.saturating_add(samples.len());
        while self.samples.len() > self.capacity_samples {
            self.samples.pop_front();
            self.buffer_start_sample = self.buffer_start_sample.saturating_add(1);
        }
        self.advance_past_evicted_candidates();
        Ok(())
    }

    /// Decode at most `max_candidates` newly available candidate windows.
    ///
    /// This method performs bounded synchronous work. A consumer should call it
    /// from its own modem worker and can use a small candidate limit to provide
    /// cancellation and UI progress opportunities between calls.
    pub fn poll(&mut self, max_candidates: usize) -> Result<Vec<Js8ScanResult>, Js8AdapterError> {
        if max_candidates == 0 {
            return Err(Js8AdapterError::InvalidScanLimit);
        }
        if self.scan_config.step_samples == 0 {
            return Err(Js8AdapterError::InvalidScanStep);
        }
        let mut results = Vec::new();
        let mut attempted = 0;
        while attempted < max_candidates && self.window_is_available() {
            let local_start = self.next_candidate_sample - self.buffer_start_sample;
            let mut window = self
                .samples
                .iter()
                .skip(local_start)
                .take(
                    self.search_window_samples.min(
                        self.total_samples_received
                            .saturating_sub(self.next_candidate_sample),
                    ),
                )
                .copied()
                .collect::<Vec<_>>();
            window.resize(self.search_window_samples, 0.0);
            let audio = AudioBlock::new(SAMPLE_RATE_HZ, window.clone())?;
            let mut scan_config = self.scan_config;
            scan_config.start_sample = 0;
            scan_config.max_candidates = 1;
            scan_config.step_samples = 1;
            scan_config.dedup_samples = 0;
            let mut candidate_results =
                scan_audio_block_detailed(&audio, self.rx_config, scan_config)?;
            let available_samples = self.search_window_samples.min(
                self.total_samples_received
                    .saturating_sub(self.next_candidate_sample),
            );
            for result in &candidate_results {
                let offset = (result.result.event.delta_time_seconds.unwrap_or_default()
                    * SAMPLE_RATE_HZ as f32)
                    .round() as isize;
                subtract_decoded_signal(&mut window, &result.result, offset, self.rx_config.mode);
            }
            for (sample, value) in self
                .samples
                .iter_mut()
                .skip(local_start)
                .take(available_samples)
                .zip(window.iter())
            {
                *sample = *value;
            }
            for result in &mut candidate_results {
                result.candidate_sample = self.next_candidate_sample;
                result.result.event.delta_time_seconds = Some(
                    self.next_candidate_sample as f32 / SAMPLE_RATE_HZ as f32
                        + result.result.event.delta_time_seconds.unwrap_or_default(),
                );
            }
            for result in candidate_results {
                let duplicate = self.scan_config.dedup_samples > 0
                    && self.reported_results.iter().any(|(message, sample)| {
                        message == &result.result.frame.message
                            && result.candidate_sample.abs_diff(*sample)
                                <= self.scan_config.dedup_samples
                    });
                if !duplicate {
                    self.reported_results
                        .push((result.result.frame.message.clone(), result.candidate_sample));
                    results.push(result);
                }
            }
            self.next_candidate_sample = self
                .next_candidate_sample
                .saturating_add(self.scan_config.step_samples);
            attempted += 1;
        }
        self.prune_reported_results();
        Ok(results)
    }

    /// Remove all buffered audio and restart candidate progression.
    pub fn reset(&mut self) {
        self.samples.clear();
        self.buffer_start_sample = 0;
        self.total_samples_received = 0;
        self.next_candidate_sample = self.scan_config.start_sample;
        self.reported_results.clear();
    }

    /// Number of samples currently retained by the bounded session.
    pub fn buffered_samples(&self) -> usize {
        self.samples.len()
    }

    /// Total number of samples accepted since construction or the last reset.
    pub fn total_samples_received(&self) -> usize {
        self.total_samples_received
    }

    /// Absolute sample position of the next candidate window.
    pub fn next_candidate_sample(&self) -> usize {
        self.next_candidate_sample
    }

    /// Whether at least one complete candidate window is ready to poll.
    pub fn is_ready(&self) -> bool {
        self.window_is_available()
    }

    fn window_is_available(&self) -> bool {
        self.next_candidate_sample >= self.buffer_start_sample
            && self
                .next_candidate_sample
                .saturating_add(self.frame_samples)
                <= self.total_samples_received
    }

    fn advance_past_evicted_candidates(&mut self) {
        while self.next_candidate_sample < self.buffer_start_sample {
            self.next_candidate_sample = self
                .next_candidate_sample
                .saturating_add(self.scan_config.step_samples);
        }
    }

    fn prune_reported_results(&mut self) {
        if self.scan_config.dedup_samples == 0 {
            self.reported_results.clear();
            return;
        }
        let oldest_relevant_sample = self
            .next_candidate_sample
            .saturating_sub(self.scan_config.dedup_samples);
        self.reported_results
            .retain(|(_, sample)| *sample >= oldest_relevant_sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_audio_block, Js8Mode, Js8TxConfig};

    fn configs() -> (Js8RxConfig, Js8ScanConfig) {
        (
            Js8RxConfig {
                mode: Js8Mode::Normal,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 5.0,
                frequency_step_hz: 0.5,
                max_fec_iterations: 10,
            },
            Js8ScanConfig {
                step_samples: 12_000,
                max_candidates: 1,
                ..Js8ScanConfig::default()
            },
        )
    }

    #[test]
    fn accepts_chunks_and_decodes_after_a_complete_window_arrives() {
        let (rx_config, scan_config) = configs();
        let tx = encode_audio_block(
            "0123456789AB",
            Js8TxConfig {
                mode: Js8Mode::Normal,
                base_frequency_hz: 1500.0,
                frame_type: 0,
            },
        )
        .unwrap();
        let mut session = Js8RxSession::new(rx_config, scan_config);
        assert!(!session.is_ready());
        for chunk in tx.samples.chunks(1_337) {
            session.push_samples(chunk).unwrap();
        }
        assert!(session.is_ready());
        let results = session.poll(1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].candidate_sample, 0);
        assert_eq!(results[0].result.frame.message, "0123456789AB");
        assert!(!session.is_ready());
    }

    #[test]
    fn persists_signal_cancellation_across_polls() {
        let (rx_config, scan_config) = configs();
        let tx = encode_audio_block(
            "0123456789AB",
            Js8TxConfig {
                mode: Js8Mode::Normal,
                base_frequency_hz: 1500.0,
                frame_type: 0,
            },
        )
        .unwrap();
        let mut session = Js8RxSession::new(rx_config, scan_config);
        session.push_samples(&tx.samples).unwrap();
        let before: f32 = session.samples.iter().map(|sample| sample.abs()).sum();
        assert_eq!(session.poll(1).unwrap().len(), 1);
        let after: f32 = session.samples.iter().map(|sample| sample.abs()).sum();
        assert!(after < before * 0.5, "residual energy: {after} / {before}");
    }

    #[test]
    fn bounds_memory_and_skips_evicted_candidates() {
        let (rx_config, scan_config) = configs();
        let mut session = Js8RxSession::with_capacity_samples(rx_config, scan_config, 10);
        session.push_samples(&vec![0.0; 200_000]).unwrap();
        assert_eq!(
            session.buffered_samples(),
            80 * Js8Mode::Normal.samples_per_symbol() - 1
        );
        assert!(session.next_candidate_sample() > 0);
    }

    #[test]
    fn rejects_non_finite_input() {
        let (rx_config, scan_config) = configs();
        let mut session = Js8RxSession::new(rx_config, scan_config);
        assert!(matches!(
            session.push_samples(&[0.0, f32::NAN]),
            Err(Js8AdapterError::Audio(
                qsonaut_modems::AudioError::NonFiniteSample
            ))
        ));
    }

    #[test]
    fn suppresses_same_message_across_polls_within_dedup_distance() {
        let (rx_config, mut scan_config) = configs();
        scan_config.step_samples = 1;
        scan_config.dedup_samples = 12_000;
        let tx = encode_audio_block(
            "0123456789AB",
            Js8TxConfig {
                mode: Js8Mode::Normal,
                base_frequency_hz: 1500.0,
                frame_type: 0,
            },
        )
        .unwrap();
        let mut session = Js8RxSession::with_capacity_samples(
            rx_config,
            scan_config,
            tx.samples.len() + Js8Mode::Normal.samples_per_symbol(),
        );
        session.push_samples(&tx.samples).unwrap();
        assert_eq!(session.poll(1).unwrap().len(), 1);
        session.push_samples(&[0.0; 1]).unwrap();
        assert!(session.poll(1).unwrap().is_empty());
    }

    #[test]
    fn reset_allows_the_same_message_to_be_reported_again() {
        let (rx_config, mut scan_config) = configs();
        scan_config.dedup_samples = 12_000;
        let tx = encode_audio_block(
            "0123456789AB",
            Js8TxConfig {
                mode: Js8Mode::Normal,
                base_frequency_hz: 1500.0,
                frame_type: 0,
            },
        )
        .unwrap();
        let mut session = Js8RxSession::new(rx_config, scan_config);
        session.push_samples(&tx.samples).unwrap();
        assert_eq!(session.poll(1).unwrap().len(), 1);
        session.reset();
        session.push_samples(&tx.samples).unwrap();
        assert_eq!(session.poll(1).unwrap().len(), 1);
    }
}
