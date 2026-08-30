use std::time::Duration;

use crate::ModemId;

/// Timing information required by a slot-oriented modem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotSpec {
    pub modem: ModemId,
    pub slot: Duration,
    pub decode_after: Duration,
    pub sample_rate_hz: u32,
}

impl SlotSpec {
    pub fn samples_for(&self, duration: Duration) -> usize {
        (duration.as_secs_f64() * self.sample_rate_hz as f64).round() as usize
    }
}

/// Prevents startup and mid-slot decode attempts for slot-oriented consumers.
#[derive(Debug, Default)]
pub struct SlotGate {
    observed_slot: Option<u64>,
    ready: bool,
    decoded_slot: Option<u64>,
}

impl SlotGate {
    pub fn observe(
        &mut self,
        slot: u64,
        position: Duration,
        spec: SlotSpec,
        buffer_ready: bool,
    ) -> bool {
        match self.observed_slot {
            None => {
                self.observed_slot = Some(slot);
                false
            }
            Some(previous) if previous != slot => {
                self.observed_slot = Some(slot);
                self.ready = true;
                self.decoded_slot = None;
                false
            }
            Some(_)
                if self.ready
                    && self.decoded_slot != Some(slot)
                    && position >= spec.decode_after
                    && buffer_ready =>
            {
                self.decoded_slot = Some(slot);
                true
            }
            Some(_) => false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FT8: ModemId = ModemId("ft8");

    #[test]
    fn waits_for_boundary_decode_time_and_buffer() {
        let spec = SlotSpec {
            modem: FT8,
            slot: Duration::from_secs(15),
            decode_after: Duration::from_secs(6),
            sample_rate_hz: 12_000,
        };
        let mut gate = SlotGate::default();
        assert!(!gate.observe(1, Duration::from_secs(7), spec, true));
        assert!(!gate.observe(2, Duration::from_secs(1), spec, true));
        assert!(!gate.observe(2, Duration::from_secs(7), spec, false));
        assert!(gate.observe(2, Duration::from_secs(7), spec, true));
        assert!(!gate.observe(2, Duration::from_secs(8), spec, true));
    }
}
