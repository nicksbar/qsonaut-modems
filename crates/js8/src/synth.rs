use std::f32::consts::TAU;

use crate::{errors::Js8SynthesisError, frame::SYMBOL_COUNT, Js8Mode};

/// The audio sample rate used by the JS8Call modem waveform.
pub const SAMPLE_RATE_HZ: u32 = 12_000;

/// Synthesize one JS8 tone sequence as real-valued mono audio.
///
/// Phase starts at zero and is continuous across symbol boundaries, matching
/// JS8Call's reference-signal generator. `base_frequency_hz` is an audio
/// frequency offset, not an RF frequency.
pub fn synthesize(
    tones: &[u8; SYMBOL_COUNT],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<Vec<f32>, Js8SynthesisError> {
    synthesize_with_frequency_curve(tones, mode, base_frequency_hz, 0.0, 0.0)
}

/// Synthesize a JS8 waveform with a linear carrier drift in hertz per second.
pub fn synthesize_with_frequency_drift(
    tones: &[u8; SYMBOL_COUNT],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
) -> Result<Vec<f32>, Js8SynthesisError> {
    synthesize_with_frequency_curve(tones, mode, base_frequency_hz, drift_hz_per_second, 0.0)
}

/// Synthesize a frame whose sample clock drifts linearly in samples per second.
/// This is primarily useful for deterministic receiver and interoperability
/// tests; consumers should normally handle resampling before calling the RX
/// adapter.
pub fn synthesize_with_timing_drift(
    tones: &[u8; SYMBOL_COUNT],
    mode: Js8Mode,
    base_frequency_hz: f32,
    timing_drift_samples_per_second: f32,
) -> Result<Vec<f32>, Js8SynthesisError> {
    if !timing_drift_samples_per_second.is_finite() {
        return Err(Js8SynthesisError::InvalidTimingDrift);
    }
    let clock_scale = 1.0 + timing_drift_samples_per_second / SAMPLE_RATE_HZ as f32;
    if clock_scale <= 0.0 {
        return Err(Js8SynthesisError::InvalidTimingDrift);
    }
    let source = synthesize(tones, mode, base_frequency_hz)?;
    let output_len = (source.len() as f32 * clock_scale).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        let source_position = output_index as f32 / clock_scale;
        let lower = source_position.floor() as usize;
        let upper = (lower + 1).min(source.len() - 1);
        let fraction = source_position - lower as f32;
        output.push(source[lower].mul_add(1.0 - fraction, source[upper] * fraction));
    }
    Ok(output)
}

/// Synthesize a JS8 waveform with linear and quadratic carrier drift terms.
pub fn synthesize_with_frequency_curve(
    tones: &[u8; SYMBOL_COUNT],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
) -> Result<Vec<f32>, Js8SynthesisError> {
    if !base_frequency_hz.is_finite() {
        return Err(Js8SynthesisError::InvalidBaseFrequency);
    }
    if !drift_hz_per_second.is_finite() || !curvature_hz_per_second2.is_finite() {
        return Err(Js8SynthesisError::InvalidBaseFrequency);
    }

    for (index, &tone) in tones.iter().enumerate() {
        if tone > 7 {
            return Err(Js8SynthesisError::InvalidTone { index, tone });
        }
    }

    let samples_per_symbol = mode.samples_per_symbol();
    let phase_step_base = TAU * base_frequency_hz / SAMPLE_RATE_HZ as f32;
    let mut phase = 0.0_f32;
    let mut samples = Vec::with_capacity(SYMBOL_COUNT * samples_per_symbol);

    for (symbol_index, &tone) in tones.iter().enumerate() {
        for sample_index in 0..samples_per_symbol {
            samples.push(phase.cos());
            let absolute_sample = symbol_index * samples_per_symbol + sample_index;
            let time = absolute_sample as f32 / SAMPLE_RATE_HZ as f32;
            let phase_step = phase_step_base
                + TAU * drift_hz_per_second * time / SAMPLE_RATE_HZ as f32
                + TAU * curvature_hz_per_second2 * time * time / SAMPLE_RATE_HZ as f32
                + TAU * f32::from(tone) / samples_per_symbol as f32;
            phase = (phase + phase_step).rem_euclid(TAU);
        }
    }

    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::{
        synthesize, synthesize_with_frequency_curve, synthesize_with_frequency_drift,
        synthesize_with_timing_drift, SAMPLE_RATE_HZ,
    };
    use crate::{Js8Mode, Js8SynthesisError};

    #[test]
    fn produces_continuous_mode_length_audio() {
        let tones = [0_u8; 79];
        let audio = synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        assert_eq!(audio.len(), 79 * 1920);
        assert_eq!(audio[0], 1.0);
        assert_eq!(SAMPLE_RATE_HZ, 12_000);
    }

    #[test]
    fn matches_oracle_phase_recurrence_checkpoints() {
        let mut tones = [0_u8; 79];
        tones[1] = 7;
        tones[2] = 1;
        let audio = synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        let checkpoints = [
            (0, 1.0_f32),
            (1, 0.707_106_77),
            (2, -4.371_139e-8),
            (1919, 0.707_026_06),
            (1920, 1.0),
            (1921, 0.690_807_34),
            (3839, 0.690_706_8),
            (3840, 1.0),
            (3841, 0.704_806_7),
            (5759, 0.704_665_36),
        ];

        for (index, expected) in checkpoints {
            assert!((audio[index] - expected).abs() < 1e-6, "sample {index}");
        }
    }

    #[test]
    fn rejects_invalid_inputs() {
        let mut tones = [0_u8; 79];
        tones[12] = 8;
        assert_eq!(
            synthesize(&tones, Js8Mode::Fast, 1500.0),
            Err(Js8SynthesisError::InvalidTone { index: 12, tone: 8 })
        );
        assert_eq!(
            synthesize(&[0_u8; 79], Js8Mode::Fast, f32::NAN),
            Err(Js8SynthesisError::InvalidBaseFrequency)
        );
        assert_eq!(
            synthesize_with_frequency_drift(&[0_u8; 79], Js8Mode::Fast, 1500.0, f32::NAN),
            Err(Js8SynthesisError::InvalidBaseFrequency)
        );
        assert_eq!(
            synthesize_with_frequency_curve(&[0_u8; 79], Js8Mode::Fast, 1500.0, 0.0, f32::NAN),
            Err(Js8SynthesisError::InvalidBaseFrequency)
        );
        assert_eq!(
            synthesize_with_timing_drift(&[0_u8; 79], Js8Mode::Fast, 1500.0, f32::NAN),
            Err(Js8SynthesisError::InvalidTimingDrift)
        );
    }
}
