use crate::costas::{MODIFIED_COSTAS, ORIGINAL_COSTAS};

/// JS8 mode parameters needed by the pure frame layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Js8Mode {
    /// Normal JS8-A, using the original Costas arrays.
    Normal,
    /// Fast JS8-B, using the modified Costas arrays.
    Fast,
    /// Turbo JS8-C, using the modified Costas arrays.
    Turbo,
    /// Slow JS8-E, using the modified Costas arrays.
    Slow,
    /// Ultra JS8-I, using the modified Costas arrays.
    Ultra,
}

impl Js8Mode {
    /// Nominal transmission duration for this mode.
    pub const fn tx_seconds(self) -> u32 {
        match self {
            Self::Normal => 15,
            Self::Fast => 10,
            Self::Turbo => 6,
            Self::Slow => 30,
            Self::Ultra => 4,
        }
    }

    /// Number of 12 kHz audio samples occupied by each channel symbol.
    pub const fn samples_per_symbol(self) -> usize {
        match self {
            Self::Normal => 1_920,
            Self::Fast => 1_200,
            Self::Turbo => 600,
            Self::Slow => 3_840,
            Self::Ultra => 384,
        }
    }

    pub(crate) const fn costas(self) -> &'static [[u8; 7]; 3] {
        match self {
            Self::Normal => &ORIGINAL_COSTAS,
            Self::Fast | Self::Turbo | Self::Slow | Self::Ultra => &MODIFIED_COSTAS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Js8Mode;

    #[test]
    fn mode_durations_match_oracle_constants() {
        assert_eq!(Js8Mode::Normal.tx_seconds(), 15);
        assert_eq!(Js8Mode::Fast.tx_seconds(), 10);
        assert_eq!(Js8Mode::Turbo.tx_seconds(), 6);
        assert_eq!(Js8Mode::Slow.tx_seconds(), 30);
        assert_eq!(Js8Mode::Ultra.tx_seconds(), 4);
    }

    #[test]
    fn mode_sample_counts_match_oracle_constants() {
        assert_eq!(Js8Mode::Normal.samples_per_symbol(), 1_920);
        assert_eq!(Js8Mode::Fast.samples_per_symbol(), 1_200);
        assert_eq!(Js8Mode::Turbo.samples_per_symbol(), 600);
        assert_eq!(Js8Mode::Slow.samples_per_symbol(), 3_840);
        assert_eq!(Js8Mode::Ultra.samples_per_symbol(), 384);
    }
}
