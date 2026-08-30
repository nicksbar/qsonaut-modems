use std::time::Duration;

const TARGET_MODEM_RATE_HZ: u32 = 12_000;
const DECIMATION_FACTOR: usize = 4;
const FIR_TAPS: usize = 48;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AudioError {
    #[error("audio sample rate must be greater than zero")]
    InvalidSampleRate,
    #[error("audio buffer contains a non-finite sample")]
    NonFiniteSample,
    #[error("audio channel count must be greater than zero")]
    InvalidChannelCount,
    #[error("audio sample rate must be exactly 48 kHz for the shared modem normalizer")]
    UnsupportedInputRate,
}

/// Convert signed PCM16 frames into normalized mono samples.
pub fn normalize_pcm16_mono(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|sample| f32::from(*sample) / 32_768.0)
        .collect()
}

/// Convert interleaved signed PCM16 frames to normalized mono samples.
pub fn normalize_pcm16_interleaved(
    samples: &[i16],
    channels: usize,
) -> Result<Vec<f32>, AudioError> {
    if channels == 0 {
        return Err(AudioError::InvalidChannelCount);
    }
    Ok(samples
        .chunks(channels)
        .map(|frame| {
            frame.iter().map(|sample| f32::from(*sample)).sum::<f32>()
                / (32_768.0 * frame.len() as f32)
        })
        .collect())
}

#[cfg(test)]
mod normalization_tests {
    use super::*;

    #[test]
    fn normalizes_pcm16_and_downmixes_interleaved_frames() {
        assert_eq!(
            normalize_pcm16_mono(&[-32_768, 0, 32_767]),
            vec![-1.0, 0.0, 0.9999695]
        );
        let mono = normalize_pcm16_interleaved(&[-32_768, 32_767, 16_384, -16_384], 2).unwrap();
        assert_eq!(mono, vec![-0.000015258789, 0.0]);
    }

    #[test]
    fn rejects_zero_channels() {
        assert_eq!(
            normalize_pcm16_interleaved(&[1], 0),
            Err(AudioError::InvalidChannelCount)
        );
    }

    #[test]
    fn normalizer_is_stable_across_capture_chunks() {
        let input = vec![10_000_i16; 48_000];
        let mut one_shot = AudioNormalizer::new(48_000).unwrap();
        let expected = one_shot.process_mono(&input);
        let mut chunked = AudioNormalizer::new(48_000).unwrap();
        let actual = input
            .chunks(1_379)
            .flat_map(|chunk| chunked.process_mono(chunk))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 12_000);
        assert_eq!(AudioNormalizer::output_rate_hz(), 12_000);
    }

    #[test]
    fn ring_buffer_keeps_recent_samples_and_supports_aligned_windows() {
        let mut ring = AudioRingBuffer::new(6);
        ring.push(&[0.0, 1.0, 2.0, 3.0]);
        ring.push(&[4.0, 5.0, 6.0, 7.0]);
        assert_eq!(ring.len(), 6);
        assert_eq!(ring.as_slice(), vec![2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(
            ring.extract_aligned_window(4, 4, 0.0, 1),
            vec![4.0, 5.0, 6.0, 7.0]
        );
    }
}

/// Stateful 48 kHz PCM normalizer for modem consumers.
///
/// Capture and monitor devices remain owned by each application. This type
/// only owns the deterministic PCM conversion and the 48 kHz -> 12 kHz
/// anti-aliased decimation needed by the shared modem adapters.
#[derive(Debug)]
pub struct AudioNormalizer {
    decimator: Decimator48To12,
}

impl Default for AudioNormalizer {
    fn default() -> Self {
        Self::new(48_000).expect("48 kHz is a supported modem input rate")
    }
}

impl AudioNormalizer {
    pub fn new(input_rate_hz: u32) -> Result<Self, AudioError> {
        if input_rate_hz != 48_000 {
            return Err(AudioError::UnsupportedInputRate);
        }
        Ok(Self {
            decimator: Decimator48To12::new(),
        })
    }

    pub fn process_mono(&mut self, samples: &[i16]) -> Vec<f32> {
        self.decimator.process(&normalize_pcm16_mono(samples))
    }

    pub fn process_f32_mono(&mut self, samples: &[f32]) -> Result<Vec<f32>, AudioError> {
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioError::NonFiniteSample);
        }
        Ok(self.decimator.process(samples))
    }

    pub fn process_interleaved(
        &mut self,
        samples: &[i16],
        channels: usize,
    ) -> Result<Vec<f32>, AudioError> {
        Ok(self
            .decimator
            .process(&normalize_pcm16_interleaved(samples, channels)?))
    }

    pub const fn output_rate_hz() -> u32 {
        TARGET_MODEM_RATE_HZ
    }
}

/// Bounded rolling mono buffer for slot-based modem consumers.
#[derive(Debug, Clone)]
pub struct AudioRingBuffer {
    samples: std::collections::VecDeque<f32>,
    capacity: usize,
}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, samples: &[f32]) {
        if self.capacity == 0 {
            return;
        }
        self.samples.extend(samples.iter().copied());
        while self.samples.len() > self.capacity {
            let excess = self.samples.len() - self.capacity;
            self.samples.drain(..excess);
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn as_slice(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    pub fn extract_aligned_window(
        &self,
        captured_samples: usize,
        window_samples: usize,
        alignment_seconds: f32,
        sample_rate_hz: u32,
    ) -> Vec<f32> {
        extract_aligned_window(
            &self.as_slice(),
            captured_samples,
            window_samples,
            alignment_seconds,
            sample_rate_hz,
        )
    }
}

/// Chunk-boundary-stable anti-aliased 48 kHz -> 12 kHz converter.
#[derive(Debug)]
pub struct Decimator48To12 {
    buffer: [f32; FIR_TAPS],
    position: usize,
    phase: usize,
}

impl Decimator48To12 {
    pub fn new() -> Self {
        Self {
            buffer: [0.0; FIR_TAPS],
            position: 0,
            phase: 0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(input.len() / DECIMATION_FACTOR + 1);
        for &sample in input {
            self.buffer[self.position] = sample;
            self.position = (self.position + 1) % FIR_TAPS;
            if self.phase == DECIMATION_FACTOR - 1 {
                let value = (0..FIR_TAPS)
                    .map(|index| {
                        let buffer_index = (self.position + index) % FIR_TAPS;
                        fir_coefficients()[index] * self.buffer[buffer_index]
                    })
                    .sum();
                output.push(value);
            }
            self.phase = (self.phase + 1) % DECIMATION_FACTOR;
        }
        output
    }
}

impl Default for Decimator48To12 {
    fn default() -> Self {
        Self::new()
    }
}

fn fir_coefficients() -> &'static [f32; FIR_TAPS] {
    use std::sync::OnceLock;
    static COEFFICIENTS: OnceLock<[f32; FIR_TAPS]> = OnceLock::new();
    COEFFICIENTS.get_or_init(|| {
        use std::f64::consts::PI;
        const FC: f64 = 0.25;
        const ALPHA: f64 = 5.0;
        let m = (FIR_TAPS - 1) as f64;
        let i0a = bessel_i0(ALPHA);
        let mut coefficients = [0.0; FIR_TAPS];
        for (index, coefficient) in coefficients.iter_mut().enumerate() {
            let n = index as f64 - m / 2.0;
            let sinc = if n == 0.0 {
                2.0 * FC
            } else {
                (2.0 * FC * PI * n).sin() / (PI * n)
            };
            let t = 2.0 * index as f64 / m - 1.0;
            let window = bessel_i0(ALPHA * (1.0 - t * t).max(0.0).sqrt()) / i0a;
            *coefficient = (sinc * window) as f32;
        }
        coefficients
    })
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x / 2.0;
    for k in 1..=25 {
        term *= half / k as f64;
        term *= half / k as f64;
        sum += term;
        if term < 1e-15 {
            break;
        }
    }
    sum
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
