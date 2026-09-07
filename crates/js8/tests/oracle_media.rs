//! Optional integration checks against the JS8Call media test corpus.
//!
//! Run with `JS8CALL_MEDIA_TESTS=/path/to/js8call/media/tests cargo test -p
//! qsonaut-js8 --test oracle_media -- --nocapture`. For faster targeted scans,
//! also set `JS8CALL_SCAN_STEP`, `JS8CALL_SCAN_MAX_CANDIDATES`,
//! `JS8CALL_SCAN_FREQUENCY_HALF_WIDTH`, `JS8CALL_SCAN_FREQUENCY_STEP`,
//! `JS8CALL_MINIMUM_SYNC_QUALITY`, `JS8CALL_SCAN_START_SECONDS`, and
//! `JS8CALL_SCAN_DURATION_SECONDS`.
//! The corpus is intentionally
//! not vendored: it belongs to the JS8Call oracle checkout and contains full
//! recordings, while this crate's public adapter currently consumes one frame.

use std::{env, fs, path::PathBuf};

use qsonaut_js8::{scan_audio_block_detailed, AudioBlock, Js8Mode, Js8RxConfig, Js8ScanConfig};

#[test]
fn js8call_recordings_scan_without_crashing() {
    let Some(root) = env::var_os("JS8CALL_MEDIA_TESTS") else {
        eprintln!("JS8CALL_MEDIA_TESTS not set; skipping external JS8Call corpus");
        return;
    };
    let mut files = fs::read_dir(root)
        .expect("JS8CALL_MEDIA_TESTS must point to JS8Call media/tests")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect::<Vec<_>>();
    files.sort();
    if let Some(only) = env::var_os("JS8CALL_MEDIA_FILE") {
        files.retain(|path| path.file_name().is_some_and(|name| name == only));
    }
    assert!(!files.is_empty(), "JS8Call corpus contains no WAV files");

    let mut decoded = 0;
    let scan_step_samples = env::var("JS8CALL_SCAN_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(12_000);
    let scan_max_candidates = env::var("JS8CALL_SCAN_MAX_CANDIDATES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64);
    let scan_frequency_half_width = env::var("JS8CALL_SCAN_FREQUENCY_HALF_WIDTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000.0);
    let scan_frequency_step = env::var("JS8CALL_SCAN_FREQUENCY_STEP")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(25.0);
    let minimum_sync_quality = env::var("JS8CALL_MINIMUM_SYNC_QUALITY")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.01);
    let scan_start_seconds = env::var("JS8CALL_SCAN_START_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.0)
        .max(0.0);
    let scan_duration_seconds = env::var("JS8CALL_SCAN_DURATION_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    for path in &files {
        let (rate, samples) = read_pcm16_wav(path);
        let mode = if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("E_")
        {
            Js8Mode::Slow
        } else {
            Js8Mode::Normal
        };
        let frame_samples = 79 * mode.samples_per_symbol();
        let start_delay = match mode {
            Js8Mode::Normal | Js8Mode::Slow => 6_000,
            _ => 0,
        };
        if samples.len() < start_delay + frame_samples + mode.samples_per_symbol() - 1 {
            panic!("{} is shorter than one JS8 frame", path.display());
        }
        let start_sample = start_delay
            .saturating_add((scan_start_seconds * rate as f32).round() as usize)
            .min(samples.len());
        let end_sample = scan_duration_seconds
            .map(|duration| {
                start_sample
                    .saturating_add((duration * rate as f32).round() as usize)
                    .min(samples.len())
            })
            .unwrap_or(samples.len());
        let block = AudioBlock::new(rate, samples[start_sample..end_sample].to_vec()).unwrap();
        let results = scan_audio_block_detailed(
            &block,
            Js8RxConfig {
                mode,
                center_frequency_hz: 1500.0,
                frequency_half_width_hz: 20.0,
                frequency_step_hz: 1.0,
                max_fec_iterations: 30,
            },
            Js8ScanConfig {
                step_samples: scan_step_samples,
                max_candidates: scan_max_candidates,
                dedup_samples: frame_samples,
                minimum_sync_quality,
                sync_frequency_half_width_hz: scan_frequency_half_width,
                sync_frequency_step_hz: scan_frequency_step,
                ..Js8ScanConfig::default()
            },
        );
        match results {
            Ok(results) if !results.is_empty() => {
                decoded += results.len();
                eprintln!(
                    "{}: decoded {} frame(s): {:?}",
                    path.display(),
                    results.len(),
                    results
                        .iter()
                        .map(|result| &result.result.frame)
                        .collect::<Vec<_>>()
                );
            }
            Ok(_) => eprintln!("{}: no recording decode", path.display()),
            Err(error) => eprintln!("{}: scan failed ({error})", path.display()),
        }
    }
    eprintln!(
        "decoded {decoded} frame(s) across {} JS8Call recordings",
        files.len()
    );
}

fn read_pcm16_wav(path: &PathBuf) -> (u32, Vec<f32>) {
    let bytes = fs::read(path).expect("read WAV");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut offset = 12;
    let mut rate = None;
    let mut samples = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let data_start = offset + 8;
        let data_end = data_start + size;
        assert!(data_end <= bytes.len(), "truncated WAV chunk");
        match id {
            b"fmt " => {
                assert_eq!(
                    u16::from_le_bytes(bytes[data_start..data_start + 2].try_into().unwrap()),
                    1
                );
                assert_eq!(
                    u16::from_le_bytes(bytes[data_start + 2..data_start + 4].try_into().unwrap()),
                    1
                );
                rate = Some(u32::from_le_bytes(
                    bytes[data_start + 4..data_start + 8].try_into().unwrap(),
                ));
                assert_eq!(
                    u16::from_le_bytes(bytes[data_start + 14..data_start + 16].try_into().unwrap()),
                    16
                );
            }
            b"data" => {
                samples = Some(
                    bytes[data_start..data_end]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32_768.0)
                        .collect(),
                );
            }
            _ => {}
        }
        offset = data_end + (size & 1);
    }
    (
        rate.expect("WAV fmt chunk"),
        samples.expect("WAV data chunk"),
    )
}
