use crate::{AudioBlock, ModemId};

/// Direction of a voice-modem audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceDirection {
    Receive,
    Transmit,
}

/// Stable, consumer-facing description of a voice modem variant.
///
/// `label` and `development` are metadata for presentation and diagnostics;
/// they do not create a GUI or product-specific mode split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceModemCapabilities {
    pub modem: ModemId,
    pub label: &'static str,
    pub modem_input_rate_hz: u32,
    pub modem_output_rate_hz: u32,
    pub speech_input_rate_hz: u32,
    pub speech_output_rate_hz: u32,
    pub supports_receive: bool,
    pub supports_transmit: bool,
    pub development: bool,
}

/// Normalized state reported by a streaming voice receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSyncState {
    Searching,
    Synchronized,
    EndOfOver,
}

/// Diagnostics associated with the current voice receive state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceRxStatus {
    pub state: VoiceSyncState,
    pub snr_db: Option<f32>,
    pub audio_frequency_offset_hz: Option<f32>,
    pub frame_index: Option<u64>,
}

impl VoiceRxStatus {
    pub const fn searching() -> Self {
        Self {
            state: VoiceSyncState::Searching,
            snr_db: None,
            audio_frequency_offset_hz: None,
            frame_index: None,
        }
    }
}

/// A decoded speech block and the status observed while producing it.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceRxBlock {
    pub modem: ModemId,
    pub audio: AudioBlock,
    pub status: VoiceRxStatus,
}

/// A validated speech block supplied to a voice modem transmitter.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceTxBlock {
    pub modem: ModemId,
    pub audio: AudioBlock,
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADE_V1: ModemId = ModemId("rade-v1");

    #[test]
    fn development_metadata_does_not_change_the_voice_surface() {
        let v1 = VoiceModemCapabilities {
            modem: RADE_V1,
            label: "RADE V1",
            modem_input_rate_hz: 8_000,
            modem_output_rate_hz: 8_000,
            speech_input_rate_hz: 16_000,
            speech_output_rate_hz: 16_000,
            supports_receive: true,
            supports_transmit: true,
            development: false,
        };
        let v2 = VoiceModemCapabilities {
            label: "RADE V2 (upstream development)",
            development: true,
            ..v1
        };

        assert_eq!(v1.modem, v2.modem);
        assert!(v2.development);
        assert_eq!(VoiceDirection::Receive, VoiceDirection::Receive);
    }

    #[test]
    fn rx_and_tx_blocks_reuse_validated_audio_blocks() {
        let audio = AudioBlock::new(16_000, vec![0.0; 160]).unwrap();
        let rx = VoiceRxBlock {
            modem: RADE_V1,
            audio: audio.clone(),
            status: VoiceRxStatus::searching(),
        };
        let tx = VoiceTxBlock {
            modem: RADE_V1,
            audio,
        };

        assert_eq!(rx.audio.sample_rate_hz, tx.audio.sample_rate_hz);
        assert_eq!(rx.status.state, VoiceSyncState::Searching);
    }
}
