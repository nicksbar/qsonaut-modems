use crate::{
    alphabet::{bit, pack_payload},
    costas::COSTAS_SYMBOLS,
    crc::crc12,
    errors::Js8EncodeError,
    fec::{parity_word as calculate_parity_word, INFORMATION_BITS},
    Js8Mode,
};

pub(crate) const SYMBOL_COUNT: usize = 79;

/// Encode a twelve-character JS8 payload into its 79 three-bit channel tones.
///
/// The lower three bits of `frame_type` are inserted into the JS8 information
/// word, matching the upstream `JS8::encode` behavior. Higher bits are ignored.
/// Tone values are in the range `0..=7`; audio modulation is intentionally not
/// part of this pure frame API.
pub fn encode_tones(
    message: &str,
    frame_type: u8,
    mode: Js8Mode,
) -> Result<[u8; SYMBOL_COUNT], Js8EncodeError> {
    let mut bytes = pack_payload(message.as_bytes())?;
    bytes[9] = (frame_type & 0b111) << 5;
    let crc = crc12(&bytes);
    bytes[9] |= (crc >> 7) as u8 & 0x1f;
    bytes[10] = (crc as u8 & 0x7f) << 1;

    let mut tones = [0_u8; SYMBOL_COUNT];
    for (block, costas) in mode.costas().iter().enumerate() {
        let offset = block * (COSTAS_SYMBOLS + 29);
        tones[offset..offset + COSTAS_SYMBOLS].copy_from_slice(costas);
    }

    let mut parity_word = 0_u8;
    let mut output_word = 0_u8;
    let mut word_bits = 0;
    for bit_index in 0..INFORMATION_BITS {
        parity_word = (parity_word << 1) | calculate_parity_word(&bytes, bit_index);
        output_word = (output_word << 1) | bit(&bytes, bit_index);
        word_bits += 1;

        if word_bits == 3 {
            let word = bit_index / 3;
            tones[7 + word] = parity_word;
            tones[43 + word] = output_word;
            parity_word = 0;
            output_word = 0;
            word_bits = 0;
        }
    }

    Ok(tones)
}

#[cfg(test)]
mod tests {
    use super::encode_tones;
    use crate::Js8Mode;

    const NORMAL_GOLDEN: [u8; 79] = [
        4, 2, 5, 6, 1, 3, 0, 7, 0, 3, 5, 3, 6, 3, 4, 1, 3, 0, 2, 2, 1, 5, 6, 0, 1, 3, 1, 3, 2, 2,
        0, 3, 2, 6, 0, 7, 4, 2, 5, 6, 1, 3, 0, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 1,
        0, 1, 1, 1, 2, 1, 3, 0, 0, 0, 1, 4, 4, 2, 5, 6, 1, 3, 0,
    ];

    const FAST_GOLDEN: [u8; 79] = [
        0, 6, 2, 3, 5, 4, 1, 7, 0, 3, 5, 3, 6, 3, 4, 1, 3, 0, 2, 2, 1, 5, 6, 0, 1, 3, 1, 3, 2, 2,
        0, 3, 2, 6, 0, 7, 1, 5, 0, 2, 3, 6, 4, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 1,
        0, 1, 1, 1, 2, 1, 3, 0, 0, 0, 1, 4, 2, 5, 0, 6, 4, 1, 3,
    ];

    // Generated directly by JS8Call Improved revision
    // e8a6121d859ba3b678b3485e7a14ed07df1bbee4.
    const CURRENT_NORMAL_DATA_GOLDEN: [u8; 79] = [
        4, 2, 5, 6, 1, 3, 0, 6, 7, 0, 7, 3, 3, 5, 2, 4, 5, 4, 5, 6, 4, 7, 3, 3, 3, 4, 3, 4, 0, 5,
        3, 5, 3, 4, 5, 5, 4, 2, 5, 6, 1, 3, 0, 2, 6, 1, 6, 2, 7, 3, 5, 3, 0, 3, 3, 0, 2, 0, 0, 0,
        2, 0, 6, 1, 2, 1, 3, 4, 7, 4, 2, 4, 4, 2, 5, 6, 1, 3, 0,
    ];

    const CURRENT_MODIFIED_DATA_GOLDEN: [u8; 79] = [
        0, 6, 2, 3, 5, 4, 1, 6, 7, 0, 7, 3, 3, 5, 2, 4, 5, 4, 5, 6, 4, 7, 3, 3, 3, 4, 3, 4, 0, 5,
        3, 5, 3, 4, 5, 5, 1, 5, 0, 2, 3, 6, 4, 2, 6, 1, 6, 2, 7, 3, 5, 3, 0, 3, 3, 0, 2, 0, 0, 0,
        2, 0, 6, 1, 2, 1, 3, 4, 7, 4, 2, 4, 2, 5, 0, 6, 4, 1, 3,
    ];

    #[test]
    fn matches_oracle_normal_and_fast_tone_vectors() {
        assert_eq!(
            encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap(),
            NORMAL_GOLDEN
        );
        assert_eq!(
            encode_tones("0123456789AB", 0, Js8Mode::Fast).unwrap(),
            FAST_GOLDEN
        );
    }

    #[test]
    fn matches_current_mentor_data_vector_in_every_enabled_mode() {
        assert_eq!(
            encode_tones("MENTOR2026AB", 4, Js8Mode::Normal).unwrap(),
            CURRENT_NORMAL_DATA_GOLDEN
        );
        for mode in [Js8Mode::Fast, Js8Mode::Turbo, Js8Mode::Slow, Js8Mode::Ultra] {
            assert_eq!(
                encode_tones("MENTOR2026AB", 4, mode).unwrap(),
                CURRENT_MODIFIED_DATA_GOLDEN,
                "current mentor mismatch in {}",
                mode.display_name()
            );
        }
    }

    #[test]
    fn frame_type_uses_only_the_lower_three_bits() {
        assert_eq!(
            encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap(),
            encode_tones("0123456789AB", 0b1_000, Js8Mode::Normal).unwrap()
        );
        assert_ne!(
            encode_tones("0123456789AB", 0, Js8Mode::Normal).unwrap(),
            encode_tones("0123456789AB", 1, Js8Mode::Normal).unwrap()
        );
    }
}
