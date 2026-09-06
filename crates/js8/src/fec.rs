use crate::{alphabet::bit, errors::Js8DecodeError, metrics::Js8BitLlrs};

pub(crate) const INFORMATION_BITS: usize = 87;

pub(crate) fn parity_word(bytes: &[u8; 11], row: usize) -> u8 {
    let mut parity = 0_u8;
    for column in 0..INFORMATION_BITS {
        if parity_bit(row, column) != 0 && bit(bytes, column) != 0 {
            parity ^= 1;
        }
    }
    parity
}

/// Decode a systematic JS8 `(174,87)` codeword with layered min-sum belief
/// propagation. LLRs are positive for zero and negative for one.
pub fn decode_codeword(
    channel_llrs: &Js8BitLlrs,
    max_iterations: usize,
) -> Result<[u8; 174], Js8DecodeError> {
    if max_iterations == 0 {
        return Err(Js8DecodeError::InvalidFecIterations);
    }
    if channel_llrs.iter().any(|llr| !llr.is_finite()) {
        return Err(Js8DecodeError::NonFiniteLogLikelihood);
    }

    let mut messages = [[0.0_f32; 174]; INFORMATION_BITS];
    for iteration in 1..=max_iterations {
        let mut updated = [[0.0_f32; 174]; INFORMATION_BITS];
        for (row, updated_row) in updated.iter_mut().enumerate() {
            let mut variables = [0_usize; INFORMATION_BITS + 1];
            let mut degree = 1;
            variables[0] = row;
            for column in 0..INFORMATION_BITS {
                if parity_bit(row, column) != 0 {
                    variables[degree] = INFORMATION_BITS + column;
                    degree += 1;
                }
            }

            let mut q = [0.0_f32; INFORMATION_BITS + 1];
            for edge in 0..degree {
                let variable = variables[edge];
                q[edge] = channel_llrs[variable];
                if variable >= INFORMATION_BITS {
                    for (check, message) in messages.iter().enumerate() {
                        if check != row {
                            q[edge] += message[variable];
                        }
                    }
                }
            }

            let mut sign_product = 1.0_f32;
            let mut smallest = f32::INFINITY;
            let mut second_smallest = f32::INFINITY;
            let mut smallest_edge = 0;
            for (edge, &value) in q.iter().enumerate().take(degree) {
                sign_product *= if value.is_sign_negative() { -1.0 } else { 1.0 };
                let magnitude = value.abs();
                if magnitude < smallest {
                    second_smallest = smallest;
                    smallest = magnitude;
                    smallest_edge = edge;
                } else if magnitude < second_smallest {
                    second_smallest = magnitude;
                }
            }

            for (edge, &value) in q.iter().enumerate().take(degree) {
                let sign = sign_product * if value.is_sign_negative() { -1.0 } else { 1.0 };
                let magnitude = if edge == smallest_edge {
                    second_smallest
                } else {
                    smallest
                };
                updated_row[variables[edge]] = sign * magnitude;
            }
        }
        messages = updated;

        let decoded = hard_decision(channel_llrs, &messages);
        if satisfies_checks(&decoded) {
            return Ok(decoded);
        }

        if iteration == max_iterations {
            return Err(Js8DecodeError::FecDecodeFailed {
                iterations: iteration,
            });
        }
    }

    unreachable!("positive max_iterations always enters the loop")
}

fn hard_decision(
    channel_llrs: &Js8BitLlrs,
    messages: &[[f32; 174]; INFORMATION_BITS],
) -> [u8; 174] {
    let mut decoded = [0_u8; 174];
    for variable in 0..174 {
        let mut posterior = channel_llrs[variable];
        for message in messages {
            posterior += message[variable];
        }
        decoded[variable] = u8::from(posterior.is_sign_negative());
    }
    decoded
}

fn satisfies_checks(decoded: &[u8; 174]) -> bool {
    for row in 0..INFORMATION_BITS {
        let mut parity = decoded[row];
        for column in 0..INFORMATION_BITS {
            if parity_bit(row, column) != 0 {
                parity ^= decoded[INFORMATION_BITS + column];
            }
        }
        if parity != 0 {
            return false;
        }
    }
    true
}

fn parity_bit(row: usize, column: usize) -> u8 {
    ((PARITY_ROWS[row] >> (86 - column)) & 1) as u8
}

// The 87 rows below are the JS8Call parity matrix encoded as 87-bit values.
// The final unused bit from each upstream 88-bit hexadecimal row is removed.
const PARITY_ROWS: [u128; INFORMATION_BITS] = [
    0x11ddd418711db5b7a84c17,
    0x0fc72aed10c62ef9984829,
    0x653d990be6c95eacd2d710,
    0x2b7bc189a9be87a1c14b27,
    0x35f1cb5af1740cf1b99a06,
    0x149aa4509c42c19457a108,
    0x65b6357e6e145d9fbe3743,
    0x1f95437ae2de912e4b08a8,
    0x424ee96b1b39a40c307b16,
    0x2b66d76373d70a5a1ff777,
    0x0277ae7d1bb35d3bc7a2d2,
    0x6292d725ea7b1399051cba,
    0x20feca90597255f597cc4e,
    0x3fd9b612042d1a6c60ede2,
    0x207e1f225dbe95d93ab722,
    0x69c55850e972954761de3b,
    0x1e87c94f79ca4dec26a39a,
    0x22e9c0a7a820327c02a4d7,
    0x78a6df931c12e85e82582f,
    0x6db8a7c7b274563d78d3b7,
    0x46813a6f38f3e0d402af58,
    0x28fc0ab9eea024d8416f0a,
    0x6c7c9bf98c1172be2b11b8,
    0x5b29bfa0bf30e8d384299b,
    0x765ebe39dce69a61b90645,
    0x1e8c47523bfb7d2098bd27,
    0x0d623395aa4e6b6dd3cde6,
    0x51bb929bb9f533c1b3e1fb,
    0x06dec0b7dd0aa1fb90ee39,
    0x6520c36ea261890ab2e7ae,
    0x14e14edd4e2a2f133bb17f,
    0x0b0b6bc00c685a3a2e5079,
    0x7f1bc014a0eb36ef015cce,
    0x54fd47285e5819642f1982,
    0x41fb2078d245475e0221f5,
    0x1bbb57aa667dd748b57ef3,
    0x547e4834bb61ab34f3ce70,
    0x784548fd970fbc14830cd4,
    0x664ed2aff02368659d3b86,
    0x69b6b31534d7125ba6e5ec,
    0x20483d80940781e0191ca3,
    0x681bedc128baec28f9d780,
    0x0df8a48303e2a01933076f,
    0x057bb918b0f61118405f43,
    0x7654d7d07b580ec9182f6e,
    0x3d46f63cd28f45629c4011,
    0x482cefd15d9077bf7b9d6a,
    0x355d9096cb9cefe012c079,
    0x7b56a4125c3e4075fe7233,
    0x6ba3dfe2feb2f7b87decde,
    0x3097b1d66012db55a3b7be,
    0x02904d055da985cf3f1a58,
    0x22dbd5b1215bba3a6cf88d,
    0x361406950291ece25e2ca3,
    0x78b13b80d16b497eca24f3,
    0x46c838dbf3d351776b4b2f,
    0x5fa7ab7039938fb55a5fc0,
    0x607e1f627dbe95d93ab322,
    0x2bed3689e5cb53b44d93c8,
    0x54fd1777d37c3cb51aabb9,
    0x0b266430deec01e2a3f956,
    0x6636f2cbaaa10492fc8769,
    0x5060019d2955b14cc017e9,
    0x593a6dc55e9e379cb751ab,
    0x4bea0b4e599f3a1ab8c6c8,
    0x40e7e378c61ad8f0f8b88a,
    0x240d1506fc511ac1fc16b6,
    0x040e14d086a34666de765b,
    0x1620a15fa1580f3883b566,
    0x532b9f9ee458b64e8cfba3,
    0x643d7cd2ea90355e529954,
    0x0096f710cc75d4158cd0ed,
    0x58e5275171e8b9dd6a1bce,
    0x599f64bdf41e7209fcd664,
    0x2d87bba15e5435c009304d,
    0x1bec7057c92c5cf462fcd9,
    0x1ad69fd87d75af8d86186e,
    0x308a704241821fe9f9c545,
    0x66c90fefacf441341bb1fb,
    0x4af22f66809ad654eb7357,
    0x172a3eebd02fb2cbd5628b,
    0x0a6687b217e062ff1d32e5,
    0x1d050efebf7714e17413f0,
    0x645aeffe19a84aee6e5795,
    0x1ee80d2cec31883a1f63a9,
    0x455edc44f7df1cd288508c,
    0x1f918f90902a9b8e79f151,
];

#[cfg(test)]
mod tests {
    use super::{decode_codeword, parity_word, INFORMATION_BITS};
    use crate::{alphabet::bit, errors::Js8DecodeError, metrics::Js8BitLlrs};

    fn codeword_for(bytes: &[u8; 11]) -> [u8; 174] {
        let mut codeword = [0_u8; 174];
        for row in 0..INFORMATION_BITS {
            codeword[row] = parity_word(bytes, row);
            codeword[INFORMATION_BITS + row] = bit(bytes, row);
        }
        codeword
    }

    fn strong_llrs(codeword: &[u8; 174]) -> Js8BitLlrs {
        let mut llrs = [8.0_f32; 174];
        for (llr, &bit) in llrs.iter_mut().zip(codeword) {
            if bit == 1 {
                *llr = -8.0;
            }
        }
        llrs
    }

    #[test]
    fn decodes_a_clean_systematic_codeword() {
        let bytes = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x40,
        ];
        let expected = codeword_for(&bytes);
        assert_eq!(
            decode_codeword(&strong_llrs(&expected), 10).unwrap(),
            expected
        );
    }

    #[test]
    fn corrects_a_single_hard_bit_error() {
        let bytes = [
            0xa5, 0x5a, 0x3c, 0xc3, 0x96, 0x69, 0x0f, 0xf0, 0x81, 0x7e, 0x20,
        ];
        let expected = codeword_for(&bytes);
        let mut llrs = strong_llrs(&expected);
        llrs[3] = -llrs[3];
        assert_eq!(decode_codeword(&llrs, 30).unwrap(), expected);
    }

    #[test]
    fn rejects_invalid_decoder_inputs() {
        let llrs = [0.0_f32; 174];
        assert_eq!(
            decode_codeword(&llrs, 0),
            Err(Js8DecodeError::InvalidFecIterations)
        );
        let mut invalid = llrs;
        invalid[12] = f32::NAN;
        assert_eq!(
            decode_codeword(&invalid, 1),
            Err(Js8DecodeError::NonFiniteLogLikelihood)
        );
    }
}
