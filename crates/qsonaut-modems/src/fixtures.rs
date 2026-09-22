use crate::{AudioBlock, DecodeBatch, ModemId};

/// Provenance for a deterministic modem fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureProvenance {
    pub source: &'static str,
    pub license: &'static str,
}

/// The normalized decode identity expected from a fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureDecode {
    pub modem: ModemId,
    pub message: &'static str,
}

/// Consumer-neutral metadata for an externally stored modem fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModemFixture {
    pub name: &'static str,
    pub modem: ModemId,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub provenance: FixtureProvenance,
    pub expected_decodes: &'static [FixtureDecode],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FixtureError {
    #[error("fixture audio rate {actual_hz} Hz does not match {expected_hz} Hz")]
    SampleRateMismatch { expected_hz: u32, actual_hz: u32 },
    #[error("fixture channels must be mono")]
    UnsupportedChannels,
    #[error("fixture decode results do not match the expected normalized identities")]
    UnexpectedDecodes,
}

impl ModemFixture {
    /// Validate fixture audio and normalized result identities without owning
    /// the decoder, fixture files, timing policy, or persistence.
    pub fn validate(&self, audio: &AudioBlock, batch: &DecodeBatch) -> Result<(), FixtureError> {
        if self.channels != 1 {
            return Err(FixtureError::UnsupportedChannels);
        }
        if audio.sample_rate_hz != self.sample_rate_hz {
            return Err(FixtureError::SampleRateMismatch {
                expected_hz: self.sample_rate_hz,
                actual_hz: audio.sample_rate_hz,
            });
        }
        if batch.events.len() == self.expected_decodes.len()
            && batch
                .events
                .iter()
                .zip(self.expected_decodes)
                .all(|(actual, expected)| {
                    actual.modem == expected.modem && actual.message == expected.message
                })
        {
            Ok(())
        } else {
            Err(FixtureError::UnexpectedDecodes)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{DecodeEvent, DecodeTelemetry};

    use super::*;

    const EXPECTED: &[FixtureDecode] = &[FixtureDecode {
        modem: ModemId("ft8"),
        message: "CQ N7UF DN26",
    }];

    fn fixture() -> ModemFixture {
        ModemFixture {
            name: "ft8-generated",
            modem: ModemId("ft8"),
            sample_rate_hz: 12_000,
            channels: 1,
            provenance: FixtureProvenance {
                source: "generated",
                license: "CC0-1.0",
            },
            expected_decodes: EXPECTED,
        }
    }

    fn batch(message: &str) -> DecodeBatch {
        DecodeBatch {
            events: vec![DecodeEvent {
                modem: ModemId("ft8"),
                message: message.into(),
                snr_db: None,
                delta_time_seconds: None,
                audio_frequency_hz: None,
            }],
            telemetry: DecodeTelemetry {
                elapsed: Duration::ZERO,
                input_samples: 0,
                decoded_events: 1,
            },
        }
    }

    #[test]
    fn validates_expected_decode_identity() {
        let audio = AudioBlock::new(12_000, vec![]).unwrap();
        assert_eq!(fixture().validate(&audio, &batch("CQ N7UF DN26")), Ok(()));
    }

    #[test]
    fn rejects_invalid_audio_and_result_contracts() {
        let wrong_rate = AudioBlock::new(48_000, vec![]).unwrap();
        assert_eq!(
            fixture().validate(&wrong_rate, &batch("CQ N7UF DN26")),
            Err(FixtureError::SampleRateMismatch {
                expected_hz: 12_000,
                actual_hz: 48_000,
            })
        );
        let audio = AudioBlock::new(12_000, vec![]).unwrap();
        assert_eq!(
            fixture().validate(&audio, &batch("CQ OTHER AA00")),
            Err(FixtureError::UnexpectedDecodes)
        );
    }
}
