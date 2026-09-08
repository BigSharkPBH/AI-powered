//! Injected PCM capture. Tests inject frames and sidecar health; no external process is spawned.

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::pcm::{PcmRing, downsample_48k_to_16k};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioError {
    SidecarFailed,
}

impl AudioError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SidecarFailed => "SESSION_SIDECAR_FAILED",
        }
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AudioError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarPoll {
    Alive,
    Exited,
}

pub trait PlaybackSink {
    fn play_pcm(&mut self, pcm: &[u8], sample_rate: u32);
    fn cancel(&mut self);
}

#[derive(Debug, Default)]
pub struct NoopSink;

impl PlaybackSink for NoopSink {
    fn play_pcm(&mut self, _pcm: &[u8], _sample_rate: u32) {}

    fn cancel(&mut self) {}
}

#[derive(Debug, Default)]
pub struct RecordingSink {
    frames: Vec<u8>,
    sample_rate: Option<u32>,
    cancelled: bool,
}

impl RecordingSink {
    pub fn recorded(&self) -> &[u8] {
        &self.frames
    }

    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled
    }
}

impl PlaybackSink for RecordingSink {
    fn play_pcm(&mut self, pcm: &[u8], sample_rate: u32) {
        self.frames.extend_from_slice(pcm);
        self.sample_rate = Some(sample_rate);
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

pub fn parse_level_peak(line: &str) -> Option<f64> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if value.get("type")?.as_str()? != "level" {
        return None;
    }
    value.get("peak")?.as_f64()
}

#[derive(Debug)]
struct CaptureState {
    ring: PcmRing,
    last_peak: f64,
}

#[derive(Debug)]
pub struct AudioCapture {
    state: Arc<Mutex<CaptureState>>,
    sidecar_dead: Arc<AtomicBool>,
    restarts: u8,
}

impl AudioCapture {
    pub fn from_injected() -> Self {
        Self::empty()
    }

    pub fn push_pcm(&mut self, pcm: &[u8]) {
        self.lock().ring.push(pcm);
    }

    pub fn ingest_event_line(&mut self, line: &str) {
        if let Some(peak) = parse_level_peak(line) {
            self.lock().last_peak = peak;
        }
    }

    pub fn last_peak(&self) -> f64 {
        self.lock().last_peak
    }

    pub fn snapshot_48k(&self) -> Vec<u8> {
        self.lock().ring.snapshot()
    }

    pub fn pcm_for_asr(&self) -> Vec<u8> {
        downsample_48k_to_16k(&self.snapshot_48k())
    }

    pub fn overrun_count(&self) -> u32 {
        self.lock().ring.overrun_count()
    }

    pub fn poll_sidecar(&self) -> Result<SidecarPoll, AudioError> {
        if self.sidecar_dead.load(Ordering::SeqCst) {
            Ok(SidecarPoll::Exited)
        } else {
            Ok(SidecarPoll::Alive)
        }
    }

    pub fn restart_once(&mut self) -> Result<(), AudioError> {
        if self.restarts >= 1 {
            return Err(AudioError::SidecarFailed);
        }
        self.restarts += 1;
        self.sidecar_dead.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn empty() -> Self {
        Self {
            state: Arc::new(Mutex::new(CaptureState {
                ring: PcmRing::new(),
                last_peak: 0.0,
            })),
            sidecar_dead: Arc::new(AtomicBool::new(false)),
            restarts: 0,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CaptureState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    pub(crate) fn mark_sidecar_exited(&self) {
        self.sidecar_dead.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioCapture, AudioError, NoopSink, PlaybackSink, RecordingSink, SidecarPoll,
        parse_level_peak,
    };
    use crate::audio::pcm::RING_CAPACITY_BYTES;

    fn le_i16(samples: &[i16]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn from_injected_stores_pcm_in_memory_ring() {
        let mut capture = AudioCapture::from_injected();
        let pcm = le_i16(&[100, 200, 300]);
        capture.push_pcm(&pcm);
        assert_eq!(capture.snapshot_48k(), pcm);
        assert_eq!(capture.overrun_count(), 0);
        assert_eq!(capture.pcm_for_asr(), le_i16(&[200]));
    }

    #[test]
    fn injected_overrun_drops_oldest() {
        let mut capture = AudioCapture::from_injected();
        capture.push_pcm(&vec![0x10; RING_CAPACITY_BYTES]);
        capture.push_pcm(&[0xFE, 0xFF]);
        assert_eq!(capture.overrun_count(), 1);
        let snap = capture.snapshot_48k();
        assert_eq!(snap.len(), RING_CAPACITY_BYTES);
        assert_eq!(&snap[RING_CAPACITY_BYTES - 2..], &[0xFE, 0xFF]);
    }

    #[test]
    fn parses_level_json_and_ignores_other_events() {
        let mut capture = AudioCapture::from_injected();
        capture.ingest_event_line(r#"{"type":"level","sequence":0,"peak":0.42}"#);
        assert!((capture.last_peak() - 0.42).abs() < 1e-9);
        capture.ingest_event_line(r#"{"type":"ready","sequence":1}"#);
        assert!((capture.last_peak() - 0.42).abs() < 1e-9);
        assert_eq!(
            parse_level_peak(r#"{"type":"level","sequence":2,"peak":0.75}"#),
            Some(0.75)
        );
        assert_eq!(parse_level_peak(r#"{"type":"ready","sequence":3}"#), None);
    }

    #[test]
    fn restart_once_then_second_crash_fails() {
        let mut capture = AudioCapture::from_injected();
        capture
            .restart_once()
            .expect("first sidecar restart is allowed");
        let error = capture
            .restart_once()
            .expect_err("second crash is terminal");
        assert_eq!(error, AudioError::SidecarFailed);
        assert_eq!(error.code(), "SESSION_SIDECAR_FAILED");
    }

    #[test]
    fn poll_sidecar_exit_allows_restart_once_then_fails() {
        let mut capture = AudioCapture::from_injected();
        assert_eq!(
            capture.poll_sidecar().expect("injected starts alive"),
            SidecarPoll::Alive
        );
        capture.mark_sidecar_exited();
        assert_eq!(
            capture.poll_sidecar().expect("injected exit is visible"),
            SidecarPoll::Exited
        );
        capture
            .restart_once()
            .expect("first sidecar restart is allowed");
        assert_eq!(
            capture.poll_sidecar().expect("restart clears exit"),
            SidecarPoll::Alive
        );
        capture.mark_sidecar_exited();
        assert_eq!(
            capture.poll_sidecar().expect("second crash is visible"),
            SidecarPoll::Exited
        );
        let error = capture
            .restart_once()
            .expect_err("second crash is terminal");
        assert_eq!(error, AudioError::SidecarFailed);
        assert_eq!(error.code(), "SESSION_SIDECAR_FAILED");
    }

    #[test]
    fn recording_sink_stores_bytes_and_cancel() {
        let mut sink = RecordingSink::default();
        sink.play_pcm(&[0x11, 0x22, 0x33, 0x44], 24_000);
        sink.cancel();
        assert_eq!(sink.recorded(), &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(sink.sample_rate(), Some(24_000));
        assert!(sink.cancelled());
    }

    #[test]
    fn default_sink_is_noop() {
        let mut sink = NoopSink;
        sink.play_pcm(&[0x01, 0x02], 16_000);
        sink.cancel();
    }
}
