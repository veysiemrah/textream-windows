//! Microphone capture and the speech worker.
//!
//! Audio lives in Rust rather than in the webview because recognition needs raw
//! PCM, and opening the same input device twice — once for WebAudio metering,
//! once for the recogniser — is a good way to fail on exclusive-mode hardware.
//! One capture path feeds both the level meter and the transcriber.
//!
//! Three threads are involved, which is one more than it looks like it needs:
//!
//! * The **cpal callback** runs on a realtime audio thread. It downmixes and
//!   sends. Nothing that can block or allocate unboundedly happens there.
//! * The **capture thread** owns the `cpal::Stream`, which is `!Send` on
//!   Windows and therefore cannot simply be parked in Tauri's state.
//! * The **worker thread** does the expensive part — metering, decoding, and
//!   talking to the session behind its mutex.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use prompt_core::normalized_rms;
use tauri::{AppHandle, Manager};

use crate::session::SessionState;
use crate::speech::Recognizer;

/// Capacity of the callback → worker queue, in chunks.
///
/// Bounded on purpose. If the worker ever falls behind real time, dropping the
/// oldest audio is the correct failure: the presenter keeps speaking either
/// way, and an unbounded queue would just grow until the transcript is
/// arbitrarily far behind what is being said.
const QUEUE_CHUNKS: usize = 32;

/// A running capture session. Dropping it stops the microphone.
pub struct AudioEngine {
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    capture: Option<JoinHandle<()>>,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Opens the default input device and starts metering, optionally
    /// transcribing through `recognizer`.
    pub fn start(app: AppHandle, recognizer: Option<Recognizer>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("no microphone is available")?;
        let config = device
            .default_input_config()
            .map_err(|error| format!("could not read the microphone's format: {error}"))?;

        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let (sender, receiver) = sync_channel::<Vec<f32>>(QUEUE_CHUNKS);

        let stop = Arc::new(AtomicBool::new(false));
        let muted = Arc::new(AtomicBool::new(false));
        let worker = {
            let app = app.clone();
            let stop = stop.clone();
            let muted = muted.clone();
            std::thread::Builder::new()
                .name("textream-speech".into())
                .spawn(move || run_worker(app, receiver, recognizer, sample_rate, stop, muted))
                .map_err(|error| error.to_string())?
        };

        let capture = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("textream-capture".into())
                .spawn(move || {
                    let format = config.sample_format();
                    let stream_config: cpal::StreamConfig = config.into();
                    let stream = build_stream(&device, stream_config, format, channels, sender);
                    let Ok(stream) = stream else { return };
                    if stream.play().is_err() {
                        return;
                    }
                    // The stream stays alive exactly as long as this thread
                    // does; `cpal::Stream` is `!Send`, so it cannot be handed
                    // back to the caller to hold.
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    drop(stream);
                })
                .map_err(|error| error.to_string())?
        };

        Ok(Self {
            stop,
            muted,
            capture: Some(capture),
            worker: Some(worker),
        })
    }

    /// Stops metering and transcribing without closing the device.
    ///
    /// The stream stays open so unmuting is instant — reopening a WASAPI
    /// capture device mid-take costs hundreds of milliseconds and can fail if
    /// something else grabbed it in the meantime.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Capture first: it owns the sender, and the worker only exits once
        // that sender is dropped and the channel closes.
        if let Some(handle) = self.capture.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// Builds an input stream for whichever sample format the device offers.
fn build_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    format: SampleFormat,
    channels: usize,
    sender: SyncSender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let on_error = |error| eprintln!("microphone stream error: {error}");

    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| forward(data.iter().copied(), channels, &sender),
            on_error,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                forward(
                    data.iter().map(|&s| s as f32 / -(i16::MIN as f32)),
                    channels,
                    &sender,
                )
            },
            on_error,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                forward(
                    data.iter().map(|&s| (s as f32 - 32_768.0) / 32_768.0),
                    channels,
                    &sender,
                )
            },
            on_error,
            None,
        ),
        other => return Err(format!("unsupported microphone sample format: {other:?}")),
    };

    stream.map_err(|error| format!("could not open the microphone: {error}"))
}

/// Downmixes to mono and hands the chunk to the worker.
fn forward(samples: impl Iterator<Item = f32>, channels: usize, sender: &SyncSender<Vec<f32>>) {
    let mono: Vec<f32> = if channels <= 1 {
        samples.collect()
    } else {
        let all: Vec<f32> = samples.collect();
        all.chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    if mono.is_empty() {
        return;
    }
    // Never block the audio thread. A full queue means the worker is behind,
    // and stalling here would underrun the device rather than help.
    match sender.try_send(mono) {
        Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
    }
}

/// Meters every chunk and, when a recogniser is present, transcribes it.
fn run_worker(
    app: AppHandle,
    receiver: Receiver<Vec<f32>>,
    mut recognizer: Option<Recognizer>,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
) {
    let started = Instant::now();
    let mut last_transcript = String::new();

    while let Ok(chunk) = receiver.recv() {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let is_muted = muted.load(Ordering::Relaxed);
        // Reporting silence rather than skipping the update keeps the speech
        // gate closing on its own timer and the waveform reading zero, so a
        // muted microphone looks muted instead of frozen.
        let level = if is_muted {
            0.0
        } else {
            normalized_rms(&chunk)
        };
        let timestamp = started.elapsed().as_secs_f64();

        let progress = {
            let state = app.state::<SessionState>();
            let mut session = state.0.lock().unwrap();
            session.feed_audio_level(level, timestamp)
        };
        crate::broadcast(&app, progress);

        if is_muted {
            continue;
        }

        let Some(recognizer) = recognizer.as_mut() else {
            continue;
        };

        recognizer.accept(sample_rate, &chunk);
        let update = recognizer.poll();

        // Decoding runs on every chunk but the transcript only changes when a
        // token is emitted, so re-matching an unchanged string would burn the
        // matcher's work for nothing.
        if !update.text.is_empty() && update.text != last_transcript {
            last_transcript = update.text.clone();
            let progress = {
                let state = app.state::<SessionState>();
                let mut session = state.0.lock().unwrap();
                session.feed_transcript(&update.text)
            };
            crate::broadcast(&app, progress);
        }

        if update.endpoint {
            // Rebase rather than restart: the position is kept, and the next
            // transcript window is measured from here instead of from a stale
            // origin that would drag the highlight backwards.
            recognizer.reset();
            last_transcript.clear();
            let state = app.state::<SessionState>();
            state.0.lock().unwrap().rebase_transcript_window();
        }
    }
}
