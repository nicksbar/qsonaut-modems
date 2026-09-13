use crate::costas::{MODIFIED_COSTAS, ORIGINAL_COSTAS};

/// JS8 mode parameters needed by the pure frame layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// All JS8 speeds enabled by the current JS8Call mentor implementation.
    pub const ALL: [Self; 5] = [
        Self::Slow,
        Self::Normal,
        Self::Fast,
        Self::Turbo,
        Self::Ultra,
    ];

    /// Current operator-facing name used by JS8Call.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Fast => "Fast",
            Self::Turbo => "JS8 40",
            Self::Slow => "Slow",
            Self::Ultra => "JS8 60",
        }
    }

    /// Transmission period from one slot boundary to the next.
    pub const fn period_seconds(self) -> u32 {
        match self {
            Self::Normal => 15,
            Self::Fast => 10,
            Self::Turbo => 6,
            Self::Slow => 30,
            Self::Ultra => 4,
        }
    }

    /// Backward-compatible alias for [`Self::period_seconds`].
    ///
    /// This value is the complete slot period, not the occupied tone duration.
    pub const fn tx_seconds(self) -> u32 {
        self.period_seconds()
    }

    /// Mentor-defined delay from the slot boundary to the first tone.
    pub const fn start_delay_ms(self) -> u32 {
        match self {
            Self::Normal | Self::Slow => 500,
            Self::Fast => 200,
            Self::Turbo | Self::Ultra => 100,
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

    /// Number of occupied 12 kHz samples in one 79-symbol frame.
    pub const fn frame_samples(self) -> usize {
        79 * self.samples_per_symbol()
    }

    /// Occupied 8-FSK bandwidth in hertz.
    pub const fn bandwidth_hz(self) -> u32 {
        (8 * crate::SAMPLE_RATE_HZ as usize / self.samples_per_symbol()) as u32
    }

    /// Decoder SNR acceptance threshold used by the current mentor source.
    pub const fn rx_snr_threshold_db(self) -> i32 {
        match self {
            Self::Slow => -28,
            Self::Normal => -24,
            Self::Fast => -22,
            Self::Turbo => -20,
            Self::Ultra => -18,
        }
    }

    /// Whether the mentor permits this speed on the heartbeat network.
    pub const fn supports_heartbeat_networking(self) -> bool {
        matches!(self, Self::Slow | Self::Normal | Self::Fast)
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

    #[test]
    fn current_mentor_mode_metadata_is_explicit() {
        assert_eq!(Js8Mode::ALL.len(), 5);
        assert_eq!(Js8Mode::Turbo.display_name(), "JS8 40");
        assert_eq!(Js8Mode::Ultra.display_name(), "JS8 60");
        assert_eq!(Js8Mode::Normal.start_delay_ms(), 500);
        assert_eq!(Js8Mode::Fast.start_delay_ms(), 200);
        assert_eq!(Js8Mode::Ultra.start_delay_ms(), 100);
        assert_eq!(Js8Mode::Slow.bandwidth_hz(), 25);
        assert_eq!(Js8Mode::Normal.bandwidth_hz(), 50);
        assert_eq!(Js8Mode::Fast.bandwidth_hz(), 80);
        assert_eq!(Js8Mode::Turbo.bandwidth_hz(), 160);
        assert_eq!(Js8Mode::Ultra.bandwidth_hz(), 250);
        assert!(Js8Mode::Normal.supports_heartbeat_networking());
        assert!(!Js8Mode::Turbo.supports_heartbeat_networking());
        assert!(!Js8Mode::Ultra.supports_heartbeat_networking());
    }
}
