use std::f32::consts::TAU;

use crate::{
    errors::Js8DecodeError,
    frame::SYMBOL_COUNT,
    sync::{correlate_tone, correlate_tone_at, correlate_tone_offset},
    Js8Mode,
};

/// Normalized correlation powers for each of the eight tones in each frame
/// symbol. Rows sum to approximately one for non-silent symbols.
pub type Js8ToneMetrics = [[f32; 8]; SYMBOL_COUNT];

/// Log-likelihood ratios for the 174 JS8 codeword bits. Positive values favor
/// zero and negative values favor one; the first 87 bits are parity bits.
pub type Js8BitLlrs = [f32; 174];

/// Demodulate an aligned frame into normalized soft tone metrics.
///
/// These are symbol-level likelihood proxies, not yet bit LLRs. The later FEC
/// layer can combine the three tone bits and feed them into LDPC decoding.
pub fn demodulate_soft(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<Js8ToneMetrics, Js8DecodeError> {
    demodulate_soft_with_frequency_curve(samples, mode, base_frequency_hz, 0.0, 0.0)
}

pub(crate) fn demodulate_soft_with_frequency_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
) -> Result<Js8ToneMetrics, Js8DecodeError> {
    demodulate_soft_with_frequency_curve_and_timing(
        samples,
        mode,
        base_frequency_hz,
        drift_hz_per_second,
        curvature_hz_per_second2,
        0.0,
    )
}

pub(crate) fn demodulate_soft_with_frequency_curve_and_timing(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    timing_drift_samples_per_second: f32,
) -> Result<Js8ToneMetrics, Js8DecodeError> {
    demodulate_soft_with_frequency_curve_and_timing_curve(
        samples,
        mode,
        base_frequency_hz,
        drift_hz_per_second,
        curvature_hz_per_second2,
        timing_drift_samples_per_second,
        0.0,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn demodulate_soft_with_frequency_curve_and_timing_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    timing_drift_samples_per_second: f32,
    timing_curvature_samples_per_second2: f32,
    adaptive_baseline: bool,
) -> Result<Js8ToneMetrics, Js8DecodeError> {
    demodulate_soft_with_frequency_curve_and_timing_baseline(
        samples,
        mode,
        base_frequency_hz,
        drift_hz_per_second,
        curvature_hz_per_second2,
        timing_drift_samples_per_second,
        timing_curvature_samples_per_second2,
        adaptive_baseline,
    )
}

#[allow(clippy::too_many_arguments)]
fn demodulate_soft_with_frequency_curve_and_timing_baseline(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    timing_drift_samples_per_second: f32,
    timing_curvature_samples_per_second2: f32,
    adaptive_baseline: bool,
) -> Result<Js8ToneMetrics, Js8DecodeError> {
    if !base_frequency_hz.is_finite() {
        return Err(Js8DecodeError::InvalidBaseFrequency);
    }
    if !drift_hz_per_second.is_finite()
        || !curvature_hz_per_second2.is_finite()
        || !timing_drift_samples_per_second.is_finite()
        || !timing_curvature_samples_per_second2.is_finite()
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

    let mut metrics = [[0.0_f32; 8]; SYMBOL_COUNT];
    let mut guard_metrics = [[0.0_f32; 8]; SYMBOL_COUNT];
    for (symbol_index, symbol_metrics) in metrics.iter_mut().enumerate() {
        let nominal_start = symbol_index * samples_per_symbol;
        let time = nominal_start as f32 / 12_000.0;
        let start = nominal_start as f32 + timing_drift_samples_per_second * time;
        let start = start + timing_curvature_samples_per_second2 * time * time;
        if !start.is_finite() || start < 0.0 || start as usize + samples_per_symbol > samples.len()
        {
            return Err(Js8DecodeError::InvalidSampleCount {
                expected: expected_samples,
                actual: samples.len(),
            });
        }
        let frequency =
            base_frequency_hz + drift_hz_per_second * time + curvature_hz_per_second2 * time * time;
        for tone in 0_u8..8 {
            let power = if start.fract().abs() < f32::EPSILON {
                correlate_tone(
                    &samples[start as usize..start as usize + samples_per_symbol],
                    tone,
                    samples_per_symbol,
                    frequency,
                )
            } else {
                correlate_tone_at(samples, start, tone, samples_per_symbol, frequency).ok_or(
                    Js8DecodeError::InvalidSampleCount {
                        expected: expected_samples,
                        actual: samples.len(),
                    },
                )?
            };
            symbol_metrics[tone as usize] = power;
        }
        for (index, tone) in (-4_i32..0).chain(8..12).enumerate() {
            guard_metrics[symbol_index][index] = if start.fract().abs() < f32::EPSILON {
                correlate_tone_offset(
                    &samples[start as usize..start as usize + samples_per_symbol],
                    tone,
                    samples_per_symbol,
                    frequency,
                )
            } else {
                correlate_tone_at_offset(samples, start, tone, samples_per_symbol, frequency)
                    .ok_or(Js8DecodeError::InvalidSampleCount {
                        expected: expected_samples,
                        actual: samples.len(),
                    })?
            };
        }
    }

    let baseline = estimate_frame_baseline(&metrics, &guard_metrics);
    for (symbol_index, symbol_metrics) in metrics.iter_mut().enumerate() {
        let baseline = if adaptive_baseline {
            let original_metrics = *symbol_metrics;
            estimate_symbol_baseline(&original_metrics, &guard_metrics[symbol_index])
        } else {
            baseline
        };
        let mut total_power = 0.0_f32;
        for metric in symbol_metrics.iter_mut() {
            *metric = (*metric - baseline).max(0.0);
            total_power += *metric;
        }
        if total_power > 0.0 {
            for metric in symbol_metrics {
                *metric /= total_power;
            }
        }
    }

    Ok(metrics)
}

fn estimate_frame_baseline(metrics: &Js8ToneMetrics, guards: &[[f32; 8]; SYMBOL_COUNT]) -> f32 {
    let mut values = [0.0_f32; 58 * 16];
    let mut count = 0;
    for symbol_metrics in metrics[7..36].iter().chain(&metrics[43..72]) {
        for &metric in symbol_metrics {
            values[count] = metric;
            count += 1;
        }
    }
    for symbol_metrics in guards[7..36].iter().chain(&guards[43..72]) {
        for &metric in symbol_metrics {
            values[count] = metric;
            count += 1;
        }
    }
    values[..count].sort_unstable_by(|left, right| left.total_cmp(right));
    values[count / 10]
}

fn estimate_symbol_baseline(metrics: &[f32; 8], guards: &[f32; 8]) -> f32 {
    let mut values = [0.0_f32; 16];
    values[..8].copy_from_slice(metrics);
    values[8..].copy_from_slice(guards);
    values.sort_unstable_by(|left, right| left.total_cmp(right));
    values[1]
}

#[inline]
fn correlate_tone_at_offset(
    samples: &[f32],
    start: f32,
    tone: i32,
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
    let phase_step = TAU * (base_frequency_hz / 12_000.0 + tone as f32 / samples_per_symbol as f32);
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

/// Convert normalized 8-FSK tone likelihoods into systematic-codeword bit
/// LLRs. Costas symbols remain unused and therefore have no output entry.
pub fn tone_metrics_to_bit_llrs(metrics: &Js8ToneMetrics) -> Js8BitLlrs {
    let mut llrs = [0.0_f32; 174];
    for word in 0..29 {
        let parity = metrics[7 + word];
        let information = metrics[43 + word];
        for bit in 0..3 {
            llrs[3 * word + bit] = tone_bit_llr_max(&parity, bit);
            llrs[87 + 3 * word + bit] = tone_bit_llr_max(&information, bit);
        }
    }
    normalize_llrs(&mut llrs[..87]);
    normalize_llrs(&mut llrs[87..]);
    llrs
}

/// Demodulate an aligned frame directly into codeword bit LLRs.
pub fn demodulate_bit_llrs(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<Js8BitLlrs, Js8DecodeError> {
    Ok(tone_metrics_to_bit_llrs(&demodulate_soft(
        samples,
        mode,
        base_frequency_hz,
    )?))
}

pub(crate) fn demodulate_bit_llrs_with_frequency_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
) -> Result<Js8BitLlrs, Js8DecodeError> {
    demodulate_bit_llrs_with_frequency_curve_and_timing(
        samples,
        mode,
        base_frequency_hz,
        drift_hz_per_second,
        curvature_hz_per_second2,
        0.0,
    )
}

pub(crate) fn demodulate_bit_llrs_with_frequency_curve_and_timing(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    timing_drift_samples_per_second: f32,
) -> Result<Js8BitLlrs, Js8DecodeError> {
    Ok(tone_metrics_to_bit_llrs(
        &demodulate_soft_with_frequency_curve_and_timing(
            samples,
            mode,
            base_frequency_hz,
            drift_hz_per_second,
            curvature_hz_per_second2,
            timing_drift_samples_per_second,
        )?,
    ))
}

/// Estimate a decoder-derived signal-to-noise ratio from the data symbols.
///
/// This is a relative channel metric, not a calibrated receiver noise-floor
/// measurement. It compares the strongest tone power in each data symbol with
/// the average power of the seven rejected tones.
pub fn estimate_snr_db(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
) -> Result<Option<f32>, Js8DecodeError> {
    estimate_snr_db_with_frequency_curve(samples, mode, base_frequency_hz, 0.0, 0.0)
}

pub(crate) fn estimate_snr_db_with_frequency_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
) -> Result<Option<f32>, Js8DecodeError> {
    if !base_frequency_hz.is_finite()
        || !drift_hz_per_second.is_finite()
        || !curvature_hz_per_second2.is_finite()
    {
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

    let mut signal_power = 0.0_f32;
    let mut noise_power = 0.0_f32;
    let mut symbol_count = 0_u32;
    for symbol_index in 7..36 {
        let time = (symbol_index * samples_per_symbol) as f32 / 12_000.0;
        accumulate_symbol_snr(
            &samples[symbol_index * samples_per_symbol..(symbol_index + 1) * samples_per_symbol],
            samples_per_symbol,
            base_frequency_hz + drift_hz_per_second * time + curvature_hz_per_second2 * time * time,
            &mut signal_power,
            &mut noise_power,
        );
        symbol_count += 1;
    }
    for symbol_index in 43..72 {
        let time = (symbol_index * samples_per_symbol) as f32 / 12_000.0;
        accumulate_symbol_snr(
            &samples[symbol_index * samples_per_symbol..(symbol_index + 1) * samples_per_symbol],
            samples_per_symbol,
            base_frequency_hz + drift_hz_per_second * time + curvature_hz_per_second2 * time * time,
            &mut signal_power,
            &mut noise_power,
        );
        symbol_count += 1;
    }
    if symbol_count == 0 || signal_power <= 0.0 {
        return Ok(None);
    }
    let signal = signal_power / symbol_count as f32;
    let noise = (noise_power / symbol_count as f32).max(f32::MIN_POSITIVE);
    Ok(Some((10.0 * (signal / noise).log10()).min(99.0)))
}

fn accumulate_symbol_snr(
    samples: &[f32],
    samples_per_symbol: usize,
    frequency: f32,
    signal_power: &mut f32,
    noise_power: &mut f32,
) {
    let mut powers = [0.0_f32; 8];
    for (tone, power) in powers.iter_mut().enumerate() {
        *power = correlate_tone(samples, tone as u8, samples_per_symbol, frequency);
    }
    let strongest = powers.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    *signal_power += strongest;
    *noise_power += (powers.iter().sum::<f32>() - strongest) / 7.0;
}

/// Match JS8Call's robust soft metric: compare the strongest tone in each
/// bit half rather than summing all competing tones. This remains useful when
/// a narrow-band interferer raises several non-winning tone bins.
fn tone_bit_llr_max(metrics: &[f32; 8], bit: usize) -> f32 {
    let mut zero = 0.0_f32;
    let mut one = 0.0_f32;
    for (tone, &power) in metrics.iter().enumerate() {
        if (tone >> (2 - bit)) & 1 == 0 {
            zero = zero.max(power);
        } else {
            one = one.max(power);
        }
    }
    zero - one
}

fn normalize_llrs(llrs: &mut [f32]) {
    if llrs.is_empty() {
        return;
    }
    let mean = llrs.iter().sum::<f32>() / llrs.len() as f32;
    let mean_square = llrs.iter().map(|value| value * value).sum::<f32>() / llrs.len() as f32;
    let variance = (mean_square - mean * mean).max(0.0);
    let scale = variance.sqrt().max(f32::MIN_POSITIVE);
    for value in llrs {
        *value = (*value / scale) * 2.83;
    }
}

#[cfg(test)]
mod tests {
    use super::{demodulate_bit_llrs, demodulate_soft, estimate_snr_db, tone_metrics_to_bit_llrs};
    use crate::{synthesize, Js8DecodeError, Js8Mode};

    #[test]
    fn clean_waveform_has_confident_normalized_tone_metrics() {
        let tones = [0_u8, 1, 2, 3, 4, 5, 6, 7]
            .into_iter()
            .cycle()
            .take(79)
            .collect::<Vec<_>>()
            .try_into()
            .expect("79 generated tones");
        let audio = synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        let metrics = demodulate_soft(&audio, Js8Mode::Normal, 1500.0).unwrap();

        for (index, &tone) in tones.iter().enumerate() {
            let sum: f32 = metrics[index].iter().sum();
            assert!((sum - 1.0).abs() < 1e-6, "symbol {index}");
            assert!(metrics[index][tone as usize] > 0.99, "symbol {index}");
        }
    }

    #[test]
    fn silent_waveform_has_no_preferred_tone() {
        let metrics = demodulate_soft(&[0.0; 79 * 384], Js8Mode::Ultra, 1500.0).unwrap();
        assert!(metrics.iter().flatten().all(|metric| *metric == 0.0));
    }

    #[test]
    fn rejects_invalid_soft_metric_input() {
        assert_eq!(
            demodulate_soft(&[], Js8Mode::Fast, 1500.0),
            Err(Js8DecodeError::InvalidSampleCount {
                expected: 79 * 1200,
                actual: 0,
            })
        );
        assert_eq!(
            demodulate_soft(&[f32::NAN; 79 * 1200], Js8Mode::Fast, 1500.0),
            Err(Js8DecodeError::NonFiniteSample)
        );
    }

    #[test]
    fn clean_waveform_produces_codeword_bit_signs() {
        let tones = [0_u8, 1, 2, 3, 4, 5, 6, 7]
            .into_iter()
            .cycle()
            .take(79)
            .collect::<Vec<_>>()
            .try_into()
            .expect("79 generated tones");
        let audio = synthesize(&tones, Js8Mode::Fast, 1500.0).unwrap();
        let llrs = demodulate_bit_llrs(&audio, Js8Mode::Fast, 1500.0).unwrap();
        let metrics = demodulate_soft(&audio, Js8Mode::Fast, 1500.0).unwrap();
        assert_eq!(llrs, tone_metrics_to_bit_llrs(&metrics));

        for word in 0..29 {
            for bit in 0..3 {
                let parity_bit = (tones[7 + word] >> (2 - bit)) & 1;
                let information_bit = (tones[43 + word] >> (2 - bit)) & 1;
                assert_eq!(llrs[3 * word + bit].is_sign_negative(), parity_bit == 1);
                assert_eq!(
                    llrs[87 + 3 * word + bit].is_sign_negative(),
                    information_bit == 1
                );
            }
        }
    }

    #[test]
    fn clean_waveform_reports_a_positive_decoder_snr() {
        let tones = crate::encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap();
        let audio = synthesize(&tones, Js8Mode::Normal, 1500.0).unwrap();
        let snr = estimate_snr_db(&audio, Js8Mode::Normal, 1500.0)
            .unwrap()
            .expect("clean frame has signal");
        assert!(snr.is_finite() && snr > 10.0, "decoder SNR: {snr}");
    }

    #[test]
    fn silent_waveform_has_no_decoder_snr() {
        assert_eq!(
            estimate_snr_db(&[0.0; 79 * 384], Js8Mode::Ultra, 1500.0).unwrap(),
            None
        );
    }

    #[test]
    fn adaptive_baseline_uses_the_local_lower_floor() {
        let metrics = [100.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let guards = [1.0, 2.0, 3.0, 11.0, 12.0, 13.0, 14.0, 15.0];
        assert_eq!(super::estimate_symbol_baseline(&metrics, &guards), 2.0);
    }
}
