use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModemId(pub &'static str);

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeEvent {
    pub modem: ModemId,
    pub message: String,
    pub snr_db: Option<f32>,
    pub delta_time_seconds: Option<f32>,
    pub audio_frequency_hz: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DecodeTelemetry {
    pub elapsed: Duration,
    pub input_samples: usize,
    pub decoded_events: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeBatch {
    pub events: Vec<DecodeEvent>,
    pub telemetry: DecodeTelemetry,
}

impl DecodeBatch {
    pub fn empty(input_samples: usize, elapsed: Duration) -> Self {
        Self {
            events: Vec::new(),
            telemetry: DecodeTelemetry {
                elapsed,
                input_samples,
                decoded_events: 0,
            },
        }
    }

    pub fn finish(input_samples: usize, started: Instant, events: Vec<DecodeEvent>) -> Self {
        let decoded_events = events.len();
        Self {
            events,
            telemetry: DecodeTelemetry {
                elapsed: started.elapsed(),
                input_samples,
                decoded_events,
            },
        }
    }
}
