use std::f32::consts::TAU;

use rustfft::{num_complex::Complex32, FftPlanner};

use crate::{costas::COSTAS_SYMBOLS, errors::Js8DecodeError, frame::SYMBOL_COUNT, Js8Mode};

use super::synth::SAMPLE_RATE_HZ;

/// Recover JS8 tones from an exactly aligned, complete waveform.
///
/// This is the first RX primitive, not the complete synchronizer: the caller
/// must provide one complete 79-symbol window and the audio-frequency offset.
/// Timing search, fine frequency tracking, soft metrics, LDPC decoding, and
/// CRC validation are separate subsequent layers.
pub fn demodulate_aligned(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<[u8; SYMBOL_COUNT], Js8DecodeError> {
    if !base_frequency_hz.is_finite() {
        return Err(Js8DecodeError::InvalidBaseFrequency);
    }

    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    if samples.len() != expected_samples {
        return Err(Js8DecodeError::InvalidSampleCount {
            expected: expected_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    let mut tones = [0_u8; SYMBOL_COUNT];
    for (symbol_index, symbol_samples) in samples.chunks_exact(samples_per_symbol).enumerate() {
        let mut best_tone = 0_u8;
        let mut best_score = f32::NEG_INFINITY;
        for tone in 0_u8..8 {
            let phase_step = TAU
                * (base_frequency_hz / SAMPLE_RATE_HZ as f32
                    + f32::from(tone) / samples_per_symbol as f32);
            let (mut in_phase, mut quadrature) = (0.0_f32, 0.0_f32);
            for (sample_index, &sample) in symbol_samples.iter().enumerate() {
                let phase = phase_step * sample_index as f32;
                in_phase += sample * phase.cos();
                quadrature += sample * phase.sin();
            }
            let score = in_phase.mul_add(in_phase, quadrature * quadrature);
            if score > best_score {
                best_score = score;
                best_tone = tone;
            }
        }
        tones[symbol_index] = best_tone;
    }

    Ok(tones)
}

/// Find the sample offset of a JS8 frame using its three Costas sequences.
///
/// The input may contain up to one symbol of leading uncertainty. The return
/// value is the frame start relative to `samples`; frequency search and drift
/// correction are intentionally separate concerns.
pub fn find_symbol_boundary(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<usize, Js8DecodeError> {
    Ok(find_symbol_boundary_with_quality(samples, mode, base_frequency_hz)?.0)
}

/// Find the strongest Costas boundary and return its normalized sync quality.
/// The quality is approximately the fraction of symbol energy concentrated in
/// the known Costas tones, making it useful as a cheap recording scan gate.
pub(crate) fn find_symbol_boundary_with_quality(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<(usize, f32), Js8DecodeError> {
    if !base_frequency_hz.is_finite() {
        return Err(Js8DecodeError::InvalidBaseFrequency);
    }

    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    let minimum_samples = expected_samples + samples_per_symbol - 1;
    if samples.len() < minimum_samples {
        return Err(Js8DecodeError::InvalidTimingWindow {
            expected: minimum_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    let costas = mode.costas();
    let mut best_offset = 0;
    let mut best_score = f32::NEG_INFINITY;
    for offset in 0..samples_per_symbol {
        let mut score = 0.0_f32;
        for (block, sequence) in costas.iter().enumerate() {
            let symbol_start = offset + block * (COSTAS_SYMBOLS + 29) * samples_per_symbol;
            for (symbol, &tone) in sequence.iter().enumerate() {
                let start = symbol_start + symbol * samples_per_symbol;
                score += correlate_tone(
                    &samples[start..start + samples_per_symbol],
                    tone,
                    samples_per_symbol,
                    base_frequency_hz,
                );
            }
        }
        if score > best_score {
            best_score = score;
            best_offset = offset;
        }
    }

    let frame = &samples[best_offset..best_offset + expected_samples];
    let energy = frame.iter().map(|sample| sample * sample).sum::<f32>();
    let quality = if energy > 0.0 {
        best_score / (energy * samples_per_symbol as f32)
    } else {
        0.0
    };
    Ok((best_offset, quality))
}

/// Rank carrier bases across a wide band using one FFT per known Costas
/// symbol. This is the waterfall acquisition path: it avoids recomputing a
/// time-domain correlation for every frequency candidate and leaves exact
/// timing/frequency refinement to the normal decoder.
pub(crate) fn find_waterfall_frequency_hypotheses(
    samples: &[f32],
    mode: Js8Mode,
    lower_frequency_hz: f32,
    upper_frequency_hz: f32,
    step_hz: f32,
    max_hypotheses: usize,
) -> Result<Vec<(f32, f32)>, Js8DecodeError> {
    if !lower_frequency_hz.is_finite()
        || !upper_frequency_hz.is_finite()
        || lower_frequency_hz >= upper_frequency_hz
    {
        return Err(Js8DecodeError::InvalidFrequencySearchRange);
    }
    if !step_hz.is_finite() || step_hz <= 0.0 {
        return Err(Js8DecodeError::InvalidFrequencySearchStep);
    }
    if max_hypotheses == 0 {
        return Ok(Vec::new());
    }

    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    let minimum_samples = expected_samples + samples_per_symbol - 1;
    if samples.len() < minimum_samples {
        return Err(Js8DecodeError::InvalidTimingWindow {
            expected: minimum_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    let fft_len = (samples_per_symbol * 4).next_power_of_two();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(fft_len);
    let mut spectra: Vec<Vec<f32>> = Vec::with_capacity(mode.costas().len() * COSTAS_SYMBOLS);
    let window_scale = std::f32::consts::TAU / samples_per_symbol as f32;
    let window: Vec<f32> = (0..samples_per_symbol)
        .map(|index| 0.5 - 0.5 * (window_scale * index as f32).cos())
        .collect();
    let mut input = vec![Complex32::new(0.0, 0.0); fft_len];
    for (block, sequence) in mode.costas().iter().enumerate() {
        let symbol_block_start = block * (COSTAS_SYMBOLS + 29) * samples_per_symbol;
        for symbol in 0..sequence.len() {
            let start = symbol_block_start + symbol * samples_per_symbol;
            input.fill(Complex32::new(0.0, 0.0));
            for (index, sample) in samples[start..start + samples_per_symbol]
                .iter()
                .copied()
                .enumerate()
            {
                input[index].re = sample * window[index];
            }
            fft.process(&mut input);
            spectra.push(
                input[..fft_len / 2]
                    .iter()
                    .map(Complex32::norm_sqr)
                    .collect(),
            );
        }
    }

    let candidate_count = ((upper_frequency_hz - lower_frequency_hz) / step_hz).floor() as usize;
    let mut hypotheses = Vec::with_capacity(max_hypotheses);
    for candidate_index in 0..=candidate_count {
        let frequency = lower_frequency_hz + candidate_index as f32 * step_hz;
        let mut target_power = 0.0_f32;
        let mut rejected_power = 0.0_f32;
        for (spectrum, tone) in spectra.iter().zip(
            mode.costas()
                .iter()
                .flat_map(|sequence| sequence.iter())
                .copied(),
        ) {
            let target_frequency =
                frequency + f32::from(tone) * SAMPLE_RATE_HZ as f32 / samples_per_symbol as f32;
            let target_bin = (target_frequency * fft_len as f32 / SAMPLE_RATE_HZ as f32).round();
            let target_bin = usize::try_from(target_bin as i64).unwrap_or(fft_len);
            if target_bin >= spectrum.len() {
                continue;
            }
            target_power += spectrum[target_bin];
            for rejected_tone in 0_u8..8 {
                if rejected_tone != tone {
                    let rejected_frequency = frequency
                        + f32::from(rejected_tone) * SAMPLE_RATE_HZ as f32
                            / samples_per_symbol as f32;
                    let rejected_bin =
                        (rejected_frequency * fft_len as f32 / SAMPLE_RATE_HZ as f32).round();
                    let rejected_bin = usize::try_from(rejected_bin as i64).unwrap_or(fft_len);
                    if rejected_bin < spectrum.len() {
                        rejected_power += spectrum[rejected_bin];
                    }
                }
            }
        }
        let rejected_average = rejected_power / (spectra.len() as f32 * 7.0).max(1.0);
        let target_average = target_power / spectra.len() as f32;
        let quality = if target_average + rejected_average > 0.0 {
            ((target_average - rejected_average) / (target_average + rejected_average)).max(0.0)
        } else {
            0.0
        };
        retain_frequency_hypothesis(&mut hypotheses, quality, frequency, max_hypotheses);
    }
    Ok(hypotheses)
}

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

/// Estimate the absolute audio frequency of an aligned JS8 frame.
///
/// The search is intentionally coarse and bounded by the caller. It evaluates
/// the known Costas symbols at each frequency candidate and returns the
/// candidate with the greatest correlation score. Timing search and fine drift
/// tracking remain separate layers.
pub fn estimate_base_frequency(
    samples: &[f32],
    mode: Js8Mode,
    center_frequency_hz: f32,
    half_width_hz: f32,
    step_hz: f32,
) -> Result<f32, Js8DecodeError> {
    if !center_frequency_hz.is_finite() || !half_width_hz.is_finite() || half_width_hz < 0.0 {
        return Err(Js8DecodeError::InvalidFrequencySearchRange);
    }
    if !step_hz.is_finite() || step_hz <= 0.0 {
        return Err(Js8DecodeError::InvalidFrequencySearchStep);
    }

    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    if samples.len() != expected_samples {
        return Err(Js8DecodeError::InvalidSampleCount {
            expected: expected_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    let lower_bound = center_frequency_hz - half_width_hz;
    let candidate_count = (2.0 * half_width_hz / step_hz).floor() as usize;
    let mut best_frequency = lower_bound;
    let mut best_score = f32::NEG_INFINITY;
    for candidate_index in 0..=candidate_count {
        let candidate = lower_bound + candidate_index as f32 * step_hz;
        let score = costas_score(samples, mode, candidate);
        if score > best_score {
            best_score = score;
            best_frequency = candidate;
        }
    }

    Ok(best_frequency)
}

/// Estimate `(frequency, slope, curvature)` at frame start from the three
/// known Costas blocks using a bounded quadratic frequency model.
pub(crate) fn estimate_frequency_track(
    samples: &[f32],
    mode: Js8Mode,
    center_frequency_hz: f32,
    half_width_hz: f32,
    step_hz: f32,
) -> Result<(f32, f32, f32), Js8DecodeError> {
    if !center_frequency_hz.is_finite() || !half_width_hz.is_finite() || half_width_hz < 0.0 {
        return Err(Js8DecodeError::InvalidFrequencySearchRange);
    }
    if !step_hz.is_finite() || step_hz <= 0.0 {
        return Err(Js8DecodeError::InvalidFrequencySearchStep);
    }
    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    if samples.len() != expected_samples {
        return Err(Js8DecodeError::InvalidSampleCount {
            expected: expected_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    let mut frequencies = [center_frequency_hz; 3];
    let mut times = [0.0_f32; 3];
    for block in 0..3 {
        let start = block * (COSTAS_SYMBOLS + 29) * samples_per_symbol;
        let end = start + COSTAS_SYMBOLS * samples_per_symbol;
        frequencies[block] = estimate_block_frequency(
            &samples[start..end],
            &mode.costas()[block],
            samples_per_symbol,
            center_frequency_hz,
            half_width_hz,
            step_hz,
        );
        times[block] =
            (start + (COSTAS_SYMBOLS * samples_per_symbol) / 2) as f32 / SAMPLE_RATE_HZ as f32;
    }

    let mean_time = times.iter().sum::<f32>() / 3.0;
    let mean_frequency = frequencies.iter().sum::<f32>() / 3.0;
    let x = times.map(|time| time - mean_time);
    let y = frequencies.map(|frequency| frequency - mean_frequency);
    let denominator = (x[0] - x[1]) * (x[0] - x[2]) * (x[1] - x[2]);
    if denominator.abs() <= f32::EPSILON {
        return Ok((mean_frequency, 0.0, 0.0));
    }
    let mut curvature =
        (x[2] * (y[1] - y[0]) + x[1] * (y[0] - y[2]) + x[0] * (y[2] - y[1])) / denominator;
    // Costas frequencies are quantized by the search step. Suppress tiny
    // curvature that is below that quantization noise and use the more stable
    // endpoint slope instead.
    let slope = if curvature.abs() < step_hz / 20.0 {
        curvature = 0.0;
        (frequencies[2] - frequencies[0]) / (times[2] - times[0])
    } else {
        (y[1] - y[0] - curvature * (x[1].powi(2) - x[0].powi(2))) / (x[1] - x[0])
    };
    let start_frequency = mean_frequency - slope * mean_time - curvature * mean_time.powi(2);
    let start_slope = slope - 2.0 * curvature * mean_time;
    Ok((start_frequency, start_slope, curvature))
}

pub(crate) fn estimate_timing_drift_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
) -> Result<(f32, f32), Js8DecodeError> {
    if !base_frequency_hz.is_finite()
        || !drift_hz_per_second.is_finite()
        || !curvature_hz_per_second2.is_finite()
    {
        return Err(Js8DecodeError::InvalidBaseFrequency);
    }
    let samples_per_symbol = mode.samples_per_symbol();
    let expected_samples = SYMBOL_COUNT * samples_per_symbol;
    if samples.len() < expected_samples {
        return Err(Js8DecodeError::InvalidSampleCount {
            expected: expected_samples,
            actual: samples.len(),
        });
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(Js8DecodeError::NonFiniteSample);
    }

    const COARSE_SYMBOLS: [usize; 3] = [0, 3, 6];
    const FULL_SYMBOLS: [usize; COSTAS_SYMBOLS] = [0, 1, 2, 3, 4, 5, 6];
    let score = |rate: f32, timing_curvature: f32, symbols: &[usize]| {
        let mut total = 0.0_f32;
        for (block, sequence) in mode.costas().iter().enumerate() {
            let block_start = block * (COSTAS_SYMBOLS + 29) * samples_per_symbol;
            for &symbol in symbols {
                let tone = sequence[symbol];
                let nominal_start = block_start + symbol * samples_per_symbol;
                let time = nominal_start as f32 / SAMPLE_RATE_HZ as f32;
                let start = nominal_start as f32 + rate * time + timing_curvature * time * time;
                let Some(tone_score) = correlate_tone_at(
                    samples,
                    start,
                    tone,
                    samples_per_symbol,
                    base_frequency_hz
                        + drift_hz_per_second * time
                        + curvature_hz_per_second2 * time * time,
                ) else {
                    return f32::NEG_INFINITY;
                };
                total += tone_score;
            }
        }
        total
    };

    let mut best_rate = 0.0_f32;
    let mut best_curvature = 0.0_f32;
    let mut best_score = f32::NEG_INFINITY;
    for rate_index in -256..=256 {
        let rate = rate_index as f32 * 0.25;
        let candidate_score = score(rate, 0.0, &COARSE_SYMBOLS);
        if candidate_score > best_score {
            best_score = candidate_score;
            best_rate = rate;
        }
    }
    let coarse_rate = best_rate;
    for rate_index in -8..=8 {
        let rate = coarse_rate + rate_index as f32;
        for curvature_index in -8..=8 {
            let timing_curvature = curvature_index as f32 * 0.5;
            let candidate_score = score(rate, timing_curvature, &COARSE_SYMBOLS);
            if candidate_score > best_score {
                best_score = candidate_score;
                best_rate = rate;
                best_curvature = timing_curvature;
            }
        }
    }
    let coarse_rate = best_rate;
    let coarse_curvature = best_curvature;
    best_score = f32::NEG_INFINITY;
    for rate_index in -8..=8 {
        let rate = coarse_rate + rate_index as f32 * 0.25;
        for curvature_index in -4..=4 {
            let timing_curvature = coarse_curvature + curvature_index as f32 * 0.25;
            let candidate_score = score(rate, timing_curvature, &FULL_SYMBOLS);
            if candidate_score > best_score {
                best_score = candidate_score;
                best_rate = rate;
                best_curvature = timing_curvature;
            }
        }
    }
    Ok((best_rate, best_curvature))
}

fn estimate_block_frequency(
    samples: &[f32],
    costas: &[u8; 7],
    samples_per_symbol: usize,
    center_frequency_hz: f32,
    half_width_hz: f32,
    step_hz: f32,
) -> f32 {
    let lower_bound = center_frequency_hz - half_width_hz;
    let candidate_count = (2.0 * half_width_hz / step_hz).floor() as usize;
    let mut best_frequency = center_frequency_hz;
    let mut best_score = f32::NEG_INFINITY;
    for candidate_index in 0..=candidate_count {
        let candidate = lower_bound + candidate_index as f32 * step_hz;
        let mut score = 0.0;
        for (symbol, &tone) in costas.iter().enumerate() {
            let start = symbol * samples_per_symbol;
            score += correlate_tone(
                &samples[start..start + samples_per_symbol],
                tone,
                samples_per_symbol,
                candidate,
            );
        }
        if score > best_score {
            best_score = score;
            best_frequency = candidate;
        }
    }
    best_frequency
}

fn costas_score(samples: &[f32], mode: Js8Mode, base_frequency_hz: f32) -> f32 {
    let samples_per_symbol = mode.samples_per_symbol();
    let mut score = 0.0_f32;
    for (block, sequence) in mode.costas().iter().enumerate() {
        let symbol_start = block * (COSTAS_SYMBOLS + 29) * samples_per_symbol;
        for (symbol, &tone) in sequence.iter().enumerate() {
            let start = symbol_start + symbol * samples_per_symbol;
            score += correlate_tone(
                &samples[start..start + samples_per_symbol],
                tone,
                samples_per_symbol,
                base_frequency_hz,
            );
        }
    }
    score
}

#[inline]
pub(crate) fn correlate_tone(
    samples: &[f32],
    tone: u8,
    samples_per_symbol: usize,
    base_frequency_hz: f32,
) -> f32 {
    correlate_tone_offset(
        samples,
        i32::from(tone),
        samples_per_symbol,
        base_frequency_hz,
    )
}

#[inline]
pub(crate) fn correlate_tone_offset(
    samples: &[f32],
    tone: i32,
    samples_per_symbol: usize,
    base_frequency_hz: f32,
) -> f32 {
    let phase_step =
        TAU * (base_frequency_hz / SAMPLE_RATE_HZ as f32 + tone as f32 / samples_per_symbol as f32);
    let (sin_step, cos_step) = phase_step.sin_cos();
    let (mut cos_phase, mut sin_phase) = (1.0_f32, 0.0_f32);
    let (mut in_phase, mut quadrature) = (0.0_f32, 0.0_f32);
    for &sample in samples {
        in_phase += sample * cos_phase;
        quadrature += sample * sin_phase;
        let next_cos = cos_phase * cos_step - sin_phase * sin_step;
        sin_phase = sin_phase * cos_step + cos_phase * sin_step;
        cos_phase = next_cos;
    }
    in_phase.mul_add(in_phase, quadrature * quadrature)
}

/// Correlate a tone against a symbol beginning at a fractional sample.
/// Linear interpolation keeps timing-rate acquisition smooth below one sample.
#[inline]
pub(crate) fn correlate_tone_at(
    samples: &[f32],
    start: f32,
    tone: u8,
    samples_per_symbol: usize,
    base_frequency_hz: f32,
) -> Option<f32> {
    if !start.is_finite() || start < 0.0 {
        return None;
    }
    let lower_start = start.floor() as usize;
    let fraction = start - lower_start as f32;
    if lower_start + samples_per_symbol > samples.len()
        || (fraction > f32::EPSILON && lower_start + samples_per_symbol >= samples.len())
    {
        return None;
    }
    let phase_step = TAU
        * (base_frequency_hz / SAMPLE_RATE_HZ as f32 + f32::from(tone) / samples_per_symbol as f32);
    let (sin_step, cos_step) = phase_step.sin_cos();
    let (mut cos_phase, mut sin_phase) = (1.0_f32, 0.0_f32);
    let (mut in_phase, mut quadrature) = (0.0_f32, 0.0_f32);
    for sample_index in 0..samples_per_symbol {
        let lower = lower_start + sample_index;
        let upper = (lower + 1).min(samples.len() - 1);
        let sample = samples[lower].mul_add(1.0 - fraction, samples[upper] * fraction);
        in_phase += sample * cos_phase;
        quadrature += sample * sin_phase;
        let next_cos = cos_phase * cos_step - sin_phase * sin_step;
        sin_phase = sin_phase * cos_step + cos_phase * sin_step;
        cos_phase = next_cos;
    }
    Some(in_phase.mul_add(in_phase, quadrature * quadrature))
}

#[cfg(test)]
fn correlate_tone_stride(
    samples: &[f32],
    tone: u8,
    samples_per_symbol: usize,
    base_frequency_hz: f32,
    stride: usize,
) -> f32 {
    let phase_step = TAU
        * (base_frequency_hz / SAMPLE_RATE_HZ as f32 + f32::from(tone) / samples_per_symbol as f32)
        * stride as f32;
    let (sin_step, cos_step) = phase_step.sin_cos();
    let (mut cos_phase, mut sin_phase) = (1.0_f32, 0.0_f32);
    let (mut in_phase, mut quadrature) = (0.0_f32, 0.0_f32);
    for sample_index in (0..samples.len()).step_by(stride) {
        in_phase += samples[sample_index] * cos_phase;
        quadrature += samples[sample_index] * sin_phase;
        let next_cos = cos_phase * cos_step - sin_phase * sin_step;
        sin_phase = sin_phase * cos_step + cos_phase * sin_step;
        cos_phase = next_cos;
    }
    in_phase.mul_add(in_phase, quadrature * quadrature)
}

#[cfg(test)]
mod tests {
    use super::{demodulate_aligned, estimate_base_frequency, find_symbol_boundary};
    use crate::{synthesize, Js8DecodeError, Js8Mode};

    #[test]
    fn recovers_a_clean_aligned_normal_waveform() {
        let source = [0, 7, 2, 6, 1, 4, 3, 5]
            .into_iter()
            .cycle()
            .take(79)
            .collect::<Vec<_>>();
        let tones: [u8; 79] = source.try_into().expect("79 generated tones");
        let audio = synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        assert_eq!(
            demodulate_aligned(&audio, Js8Mode::Normal, 1500.0).unwrap(),
            tones
        );
    }

    #[test]
    fn rejects_incomplete_or_invalid_windows() {
        assert_eq!(
            demodulate_aligned(&[], Js8Mode::Fast, 1500.0),
            Err(Js8DecodeError::InvalidSampleCount {
                expected: 79 * 1200,
                actual: 0,
            })
        );
        assert_eq!(
            demodulate_aligned(&[f32::NAN; 79 * 1200], Js8Mode::Fast, 1500.0),
            Err(Js8DecodeError::NonFiniteSample)
        );
        assert_eq!(
            demodulate_aligned(&[0.0; 79 * 1200], Js8Mode::Fast, f32::INFINITY),
            Err(Js8DecodeError::InvalidBaseFrequency)
        );
    }

    #[test]
    fn finds_a_costas_boundary_with_leading_offset() {
        let tones = crate::encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap();
        let audio = crate::synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        let leading_offset = 137;
        let mut window = vec![0.0; leading_offset];
        window.extend_from_slice(&audio);
        window.resize(audio.len() + Js8Mode::Normal.samples_per_symbol() - 1, 0.0);

        assert_eq!(
            find_symbol_boundary(&window, Js8Mode::Normal, 1500.0).unwrap(),
            leading_offset
        );
        let frame = &window[leading_offset..leading_offset + audio.len()];
        assert_eq!(
            demodulate_aligned(frame, Js8Mode::Normal, 1500.0).unwrap(),
            tones
        );
    }

    #[test]
    fn rejects_a_short_timing_window() {
        let expected = 79 * 384 + 383;
        assert_eq!(
            find_symbol_boundary(&vec![0.0; expected - 1], Js8Mode::Ultra, 1500.0),
            Err(Js8DecodeError::InvalidTimingWindow {
                expected,
                actual: expected - 1,
            })
        );
    }

    #[test]
    fn estimates_a_coarse_frequency_offset() {
        let tones = crate::encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap();
        let audio = crate::synthesize(&tones, Js8Mode::Normal, 1503.0).unwrap();
        let estimate = estimate_base_frequency(&audio, Js8Mode::Normal, 1500.0, 5.0, 0.5).unwrap();

        assert_eq!(estimate, 1503.0);
        assert_eq!(
            demodulate_aligned(&audio, Js8Mode::Normal, estimate).unwrap(),
            tones
        );
    }

    #[test]
    fn coarse_costas_ratio_rejects_a_common_noise_floor() {
        let tones = crate::encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap();
        let audio = crate::synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        let symbol = &audio[..Js8Mode::Normal.samples_per_symbol()];
        let target = super::correlate_tone_stride(
            symbol,
            tones[0],
            Js8Mode::Normal.samples_per_symbol(),
            1500.0,
            2,
        );
        let rejected = super::correlate_tone_stride(
            symbol,
            (tones[0] + 1) % 8,
            Js8Mode::Normal.samples_per_symbol(),
            1500.0,
            2,
        );
        assert!(target > rejected * 1.1);
    }

    #[test]
    fn rejects_invalid_frequency_search_parameters() {
        let audio = vec![0.0; 79 * 384];
        assert_eq!(
            estimate_base_frequency(&audio, Js8Mode::Ultra, 1500.0, -1.0, 0.5),
            Err(Js8DecodeError::InvalidFrequencySearchRange)
        );
        assert_eq!(
            estimate_base_frequency(&audio, Js8Mode::Ultra, 1500.0, 1.0, 0.0),
            Err(Js8DecodeError::InvalidFrequencySearchStep)
        );
    }
}
