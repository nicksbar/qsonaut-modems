use std::time::Duration;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AudioError {
    #[error("audio sample rate must be greater than zero")]
    InvalidSampleRate,
    #[error("audio buffer contains a non-finite sample")]
    NonFiniteSample,
}

/// A mono PCM block at the rate negotiated by the consumer's audio boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBlock {
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}

impl AudioBlock {
    pub fn new(sample_rate_hz: u32, samples: Vec<f32>) -> Result<Self, AudioError> {
        if sample_rate_hz == 0 {
            return Err(AudioError::InvalidSampleRate);
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioError::NonFiniteSample);
        }
        Ok(Self {
            sample_rate_hz,
            samples,
        })
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate_hz as f64)
    }
}

/// Copy an aligned decoder window from a rolling mono buffer.
///
/// `captured_samples` identifies the number of samples available at the
/// current slot boundary. A positive alignment shifts the requested window
/// forward; missing samples are zero-filled. This is deliberately a pure
/// buffer operation so consumers retain ownership of clocks and workers.
pub fn extract_aligned_window(
    rolling: &[f32],
    captured_samples: usize,
    window_samples: usize,
    alignment_seconds: f32,
    sample_rate_hz: u32,
) -> Vec<f32> {
    let mut window = vec![0.0; window_samples];
    let boundary = rolling.len() as isize - captured_samples.min(rolling.len()) as isize;
    let alignment = (alignment_seconds * sample_rate_hz as f32).round() as isize;
    let requested_start = boundary + alignment;
    let source_start = requested_start.max(0) as usize;
    let destination_start = requested_start.min(0).unsigned_abs().min(window_samples);
    let copy_len = rolling
        .len()
        .saturating_sub(source_start)
        .min(window_samples.saturating_sub(destination_start));
    if copy_len > 0 {
        window[destination_start..destination_start + copy_len]
            .copy_from_slice(&rolling[source_start..source_start + copy_len]);
    }
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(
            AudioBlock::new(0, vec![]),
            Err(AudioError::InvalidSampleRate)
        );
        assert_eq!(
            AudioBlock::new(12_000, vec![f32::NAN]),
            Err(AudioError::NonFiniteSample)
        );
    }

    #[test]
    fn reports_duration_from_samples_and_rate() {
        let audio = AudioBlock::new(12_000, vec![0.0; 24_000]).unwrap();
        assert_eq!(audio.duration(), Duration::from_secs(2));
    }

    #[test]
    fn extracts_an_aligned_window_with_zero_padding() {
        let rolling = (0..10).map(|sample| sample as f32).collect::<Vec<_>>();
        let window = extract_aligned_window(&rolling, 6, 6, 0.0, 1);
        assert_eq!(window, vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let shifted = extract_aligned_window(&rolling, 6, 6, -2.0, 1);
        assert_eq!(shifted, vec![2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
    }
}
