pub(crate) fn crc12(bytes: &[u8; 11]) -> u16 {
    // This is Boost's direct augmented update used by JS8Call: the incoming
    // byte bits are shifted into the twelve-bit remainder and the truncated
    // polynomial is applied when the prior high bit is set. JS8Call then XORs
    // the result with 42 in its CRC12 helper.
    let mut remainder = 0_u16;
    for byte in bytes {
        for bit in (0..8).rev() {
            let quotient = remainder & 0x800;
            remainder = (remainder << 1) | u16::from((byte >> bit) & 1);
            if quotient != 0 {
                remainder ^= 0xc06;
            }
        }
    }
    (remainder & 0x0fff) ^ 42
}
