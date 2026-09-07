use crate::errors::Js8EncodeError;

pub(crate) const ALPHABET: &[u8; 64] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-+";

pub(crate) fn pack_payload(message: &[u8]) -> Result<[u8; 11], Js8EncodeError> {
    if message.len() != 12 {
        return Err(Js8EncodeError::InvalidLength {
            actual: message.len(),
        });
    }

    let mut bytes = [0_u8; 11];
    for (index, chunk) in message.as_chunks::<4>().0.iter().enumerate() {
        let words = (alphabet_word(chunk[0], index * 4)? << 18)
            | (alphabet_word(chunk[1], index * 4 + 1)? << 12)
            | (alphabet_word(chunk[2], index * 4 + 2)? << 6)
            | alphabet_word(chunk[3], index * 4 + 3)?;
        let offset = index * 3;
        bytes[offset] = (words >> 16) as u8;
        bytes[offset + 1] = (words >> 8) as u8;
        bytes[offset + 2] = words as u8;
    }
    Ok(bytes)
}

fn alphabet_word(byte: u8, index: usize) -> Result<u32, Js8EncodeError> {
    ALPHABET
        .iter()
        .position(|candidate| *candidate == byte)
        .map(|word| word as u32)
        .ok_or(Js8EncodeError::InvalidCharacter { index, byte })
}

pub(crate) fn bit(bytes: &[u8; 11], index: usize) -> u8 {
    (bytes[index / 8] >> (7 - index % 8)) & 1
}

#[cfg(test)]
mod tests {
    use super::{pack_payload, ALPHABET};
    use crate::errors::Js8EncodeError;

    #[test]
    fn rejects_invalid_payloads() {
        assert_eq!(
            pack_payload(b"short"),
            Err(Js8EncodeError::InvalidLength { actual: 5 })
        );
        assert_eq!(
            pack_payload(b"0123456789A!"),
            Err(Js8EncodeError::InvalidCharacter {
                index: 11,
                byte: b'!',
            })
        );
    }

    #[test]
    fn packs_the_full_js8_alphabet() {
        let payload = &ALPHABET[..12];
        assert!(pack_payload(payload).is_ok());
    }
}
