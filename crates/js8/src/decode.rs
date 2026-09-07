use crate::{
    alphabet::ALPHABET, crc::crc12, errors::Js8DecodeError, fec::decode_codeword,
    metrics::Js8BitLlrs, Js8Mode,
};

/// The decoded JS8 payload and its three-bit frame type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Js8DecodedFrame {
    pub message: String,
    pub frame_type: u8,
}

/// Decode a systematic JS8 codeword and validate its embedded CRC.
pub fn decode_frame(codeword: &[u8; 174]) -> Result<Js8DecodedFrame, Js8DecodeError> {
    let mut bytes = [0_u8; 11];
    for (index, &value) in codeword[87..].iter().enumerate() {
        if value > 1 {
            return Err(Js8DecodeError::InvalidCodewordBit { index, value });
        }
        if value != 0 {
            bytes[index / 8] |= 1 << (7 - index % 8);
        }
    }

    let frame_type = bytes[9] >> 5;
    let actual_crc = (u16::from(bytes[9] & 0x1f) << 7) | u16::from(bytes[10] >> 1);
    let mut crc_input = bytes;
    crc_input[9] &= 0xe0;
    crc_input[10] = 0;
    let expected_crc = crc12(&crc_input);
    if actual_crc != expected_crc {
        return Err(Js8DecodeError::CrcMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }

    let mut message = String::with_capacity(12);
    for group in bytes[..9].as_chunks::<3>().0 {
        let packed = (u32::from(group[0]) << 16) | (u32::from(group[1]) << 8) | u32::from(group[2]);
        for shift in [18, 12, 6, 0] {
            message.push(ALPHABET[((packed >> shift) & 0x3f) as usize] as char);
        }
    }

    Ok(Js8DecodedFrame {
        message,
        frame_type,
    })
}

/// Decode aligned audio LLRs through LDPC and CRC validation.
pub fn decode_llrs(
    llrs: &Js8BitLlrs,
    max_iterations: usize,
) -> Result<Js8DecodedFrame, Js8DecodeError> {
    let mut last_error = None;
    for erased_bits in [0, 24, 48] {
        let mut pass = *llrs;
        if erased_bits != 0 {
            pass[..erased_bits].fill(0.0);
        }
        match decode_codeword(&pass, max_iterations).and_then(|codeword| decode_frame(&codeword)) {
            Ok(frame) => return Ok(frame),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("the decode pass list is non-empty"))
}

/// Decode an aligned audio frame through soft metrics, LDPC, and CRC.
pub fn decode_audio(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    max_iterations: usize,
) -> Result<Js8DecodedFrame, Js8DecodeError> {
    decode_audio_with_frequency_curve(samples, mode, base_frequency_hz, 0.0, 0.0, max_iterations)
}

pub(crate) fn decode_audio_with_frequency_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    max_iterations: usize,
) -> Result<Js8DecodedFrame, Js8DecodeError> {
    decode_llrs(
        &crate::metrics::demodulate_bit_llrs_with_frequency_curve(
            samples,
            mode,
            base_frequency_hz,
            drift_hz_per_second,
            curvature_hz_per_second2,
        )?,
        max_iterations,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_audio_with_frequency_curve_and_timing_curve(
    samples: &[f32],
    mode: Js8Mode,
    base_frequency_hz: f32,
    drift_hz_per_second: f32,
    curvature_hz_per_second2: f32,
    timing_drift_samples_per_second: f32,
    timing_curvature_samples_per_second2: f32,
    max_iterations: usize,
) -> Result<Js8DecodedFrame, Js8DecodeError> {
    let primary_metrics = crate::metrics::demodulate_soft_with_frequency_curve_and_timing_curve(
        samples,
        mode,
        base_frequency_hz,
        drift_hz_per_second,
        curvature_hz_per_second2,
        timing_drift_samples_per_second,
        timing_curvature_samples_per_second2,
        false,
    )?;
    let primary_error = match decode_llrs(
        &crate::metrics::tone_metrics_to_bit_llrs(&primary_metrics),
        max_iterations,
    ) {
        Ok(frame) => return Ok(frame),
        Err(error) => error,
    };

    let Ok(adaptive_metrics) =
        crate::metrics::demodulate_soft_with_frequency_curve_and_timing_curve(
            samples,
            mode,
            base_frequency_hz,
            drift_hz_per_second,
            curvature_hz_per_second2,
            timing_drift_samples_per_second,
            timing_curvature_samples_per_second2,
            true,
        )
    else {
        return Err(primary_error);
    };
    decode_llrs(
        &crate::metrics::tone_metrics_to_bit_llrs(&adaptive_metrics),
        max_iterations,
    )
    .map_err(|_| primary_error)
}

#[cfg(test)]
mod tests {
    use super::{decode_audio, decode_frame, Js8DecodedFrame};
    use crate::{
        alphabet::bit,
        crc::crc12,
        errors::Js8DecodeError,
        fec::{parity_word, INFORMATION_BITS},
        synthesize, Js8Mode,
    };

    #[test]
    fn decodes_a_clean_audio_frame_end_to_end() {
        let tones = crate::encode_tones("0123456789AB", 5, Js8Mode::Fast).unwrap();
        let audio = synthesize(&tones, Js8Mode::Fast, 1500.0).unwrap();
        assert_eq!(
            decode_audio(&audio, Js8Mode::Fast, 1500.0, 10).unwrap(),
            Js8DecodedFrame {
                message: "0123456789AB".to_owned(),
                frame_type: 5,
            }
        );
    }

    #[test]
    fn rejects_a_frame_with_a_bad_crc() {
        let mut bytes = [0_u8; 11];
        bytes[..9].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11]);
        bytes[9] = 3 << 5;
        let crc = crc12(&bytes);
        bytes[9] |= (crc >> 7) as u8;
        bytes[10] = (crc as u8 & 0x7f) << 1;

        let mut codeword = [0_u8; 174];
        for row in 0..INFORMATION_BITS {
            codeword[row] = parity_word(&bytes, row);
            codeword[87 + row] = bit(&bytes, row);
        }
        codeword[87] ^= 1;
        assert!(matches!(
            decode_frame(&codeword),
            Err(Js8DecodeError::CrcMismatch { .. })
        ));
    }
}
