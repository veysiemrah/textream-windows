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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use prompt_core::normalized_rms;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::session::SessionState;
use crate::speech::Recognizer;

/// Capacity of the callback → worker queue, in chunks.
///
/// Bounded on purpose. If the worker ever falls behind real time, dropping
/// audio is the correct failure: the presenter keeps speaking either way, and
/// an unbounded queue would just grow until the transcript is arbitrarily far
/// behind what is being said.
const QUEUE_CHUNKS: usize = 64;

/// How often the UI is told about the microphone level.
///
/// Fast enough that a waveform looks live, slow enough that reporting is not
/// competing with speech decoding for the same thread.
const BROADCAST_INTERVAL: Duration = Duration::from_millis(50);

/// What the capture path is actually doing.
///
/// Recognition failing because audio was dropped and recognition failing
/// because the model is weak look identical from the outside — words go missing
/// at random either way. These numbers are what tells them apart.
#[derive(Default)]
pub struct Diagnostics {
    /// Chunks the worker could not keep up with.
    dropped: AtomicUsize,
    /// Times the network actually ran.
    decodes: AtomicUsize,
    /// The most recent transcript, verbatim.
    heard: Mutex<String>,
    /// Capture rate and channel count, as opened.
    format: Mutex<String>,
}

/// Diagnostics as the UI reads them.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsView {
    pub dropped_chunks: usize,
    pub decodes: usize,
    /// What the recogniser last transcribed, before any matching.
    pub heard: String,
    pub input_format: String,
}

impl Diagnostics {
    fn view(&self) -> DiagnosticsView {
        DiagnosticsView {
            dropped_chunks: self.dropped.load(Ordering::Relaxed),
            decodes: self.decodes.load(Ordering::Relaxed),
            heard: self.heard.lock().unwrap().clone(),
            input_format: self.format.lock().unwrap().clone(),
        }
    }
}

/// A microphone as offered in Settings.
///
/// `id` is cpal's own [`cpal::DeviceId`], serialised through its `Display`
/// implementation (`"<host>:<device>"`) — stable across relaunches and
/// reboots, unlike the display name, which some drivers reorder or rename
/// between sessions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Every usable input device cpal can currently see.
///
/// Called fresh each time the dropdown opens rather than cached, so plugging
/// in a headset after the app has started shows up without a restart. A
/// device that fails to report a default config (output-only, or a driver
/// returning empty capabilities) is left out: listing it would only offer a
/// choice that fails the moment it is opened.
pub fn list_input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok());

    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    devices
        .filter_map(|device| {
            let id = device.id().ok()?;
            device.default_input_config().ok()?;
            let name = device
                .description()
                .ok()
                .map(|description| description.name().to_string())
                .unwrap_or_else(|| id.to_string());
            let is_default = default_id.as_ref() == Some(&id);
            Some(InputDevice {
                id: id.to_string(),
                name,
                is_default,
            })
        })
        .collect()
}

/// Opens the requested device, falling back to the system default when it is
/// missing.
///
/// A saved device id that no longer matches anything — unplugged, or never
/// having existed on this machine — must never stop the app from opening a
/// microphone at all; it just means the choice quietly reverts to whatever
/// Windows currently calls default.
fn resolve_device(host: &cpal::Host, device_id: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(id) = device_id {
        if let Ok(parsed) = cpal::DeviceId::from_str(id) {
            if let Some(device) = host.device_by_id(&parsed) {
                return Ok(device);
            }
        }
    }
    host.default_input_device()
        .ok_or_else(|| "No microphone was found. Plug one in and try again.".to_string())
}

/// The device's human-readable name, or a generic fallback when even that
/// cannot be read (the device disappearing between resolution and this call).
fn device_label(device: &cpal::Device) -> String {
    device
        .description()
        .ok()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|| "microphone".to_string())
}

/// A running capture session. Dropping it stops the microphone.
pub struct AudioEngine {
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    diagnostics: Arc<Diagnostics>,
    capture: Option<JoinHandle<()>>,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Opens `device_id` (or the system default, when `None` or missing) and
    /// starts metering, optionally transcribing through `recognizer`.
    pub fn start(
        app: AppHandle,
        recognizer: Option<Recognizer>,
        device_id: Option<String>,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = resolve_device(&host, device_id.as_deref())?;
        let device_name = device_label(&device);
        let config = device.default_input_config().map_err(|error| {
            format!("Could not read the microphone's format ({error}). Try a different microphone.")
        })?;

        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let (sender, receiver) = sync_channel::<Vec<f32>>(QUEUE_CHUNKS);

        let diagnostics = Arc::new(Diagnostics::default());
        *diagnostics.format.lock().unwrap() = format!(
            "{device_name} · {} Hz · {} channel{} · {:?}",
            sample_rate,
            channels,
            if channels == 1 { "" } else { "s" },
            config.sample_format()
        );

        // Best-effort: a device dropping out mid-session is worth a trace in
        // the crash log, but failing to resolve that log's own path is not a
        // reason to refuse to open the microphone at all.
        let log_path = crate::data_root(&app)
            .ok()
            .map(|root| root.join("textream.log"));

        let stop = Arc::new(AtomicBool::new(false));
        let muted = Arc::new(AtomicBool::new(false));

        let worker = {
            let app = app.clone();
            let stop = stop.clone();
            let muted = muted.clone();
            let diagnostics = diagnostics.clone();
            std::thread::Builder::new()
                .name("textream-speech".into())
                .spawn(move || {
                    run_worker(
                        app,
                        receiver,
                        recognizer,
                        sample_rate,
                        stop,
                        muted,
                        diagnostics,
                    )
                })
                .map_err(|error| error.to_string())?
        };

        let capture = {
            let stop = stop.clone();
            let diagnostics = diagnostics.clone();
            std::thread::Builder::new()
                .name("textream-capture".into())
                .spawn(move || {
                    let format = config.sample_format();
                    let stream_config: cpal::StreamConfig = config.into();
                    let stream = build_stream(
                        &device,
                        stream_config,
                        format,
                        channels,
                        sender,
                        diagnostics,
                        log_path,
                    );
                    let Ok(stream) = stream else { return };
                    if stream.play().is_err() {
                        return;
                    }
                    // The stream stays alive exactly as long as this thread
                    // does; `cpal::Stream` is `!Send`, so it cannot be handed
                    // back to the caller to hold.
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    drop(stream);
                })
                .map_err(|error| error.to_string())?
        };

        Ok(Self {
            stop,
            muted,
            diagnostics,
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

    pub fn diagnostics(&self) -> DiagnosticsView {
        self.diagnostics.view()
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
    diagnostics: Arc<Diagnostics>,
    log_path: Option<PathBuf>,
) -> Result<cpal::Stream, String> {
    // A stream error is a device dropping out mid-session — driver crash,
    // unplugged cable — the kind of thing a presenter needs to have a trace in
    // the crash log, but failing to resolve that log's own path is not a
    // reason to refuse to open the microphone at all.
    let on_error = move |error| {
        if let Some(path) = &log_path {
            crate::diagnostics::append(path, &format!("microphone stream error: {error}"));
        }
    };

    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| forward(data.iter().copied(), channels, &sender, &diagnostics),
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
                    &diagnostics,
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
                    &diagnostics,
                )
            },
            on_error,
            None,
        ),
        other => {
            return Err(format!(
                "This microphone's audio format ({other:?}) is not supported. Try a different microphone."
            ))
        }
    };

    stream.map_err(|error| {
        format!(
            "Could not open the microphone ({error}). Check that no other app has it exclusively, \
             and that Windows has granted microphone access under Settings > Privacy > Microphone."
        )
    })
}

/// Downmixes to mono and hands the chunk to the worker.
fn forward(
    samples: impl Iterator<Item = f32>,
    channels: usize,
    sender: &SyncSender<Vec<f32>>,
    diagnostics: &Diagnostics,
) {
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
    //
    // Dropped audio is counted rather than ignored: it is heard as the
    // recogniser missing words at random, which is impossible to tell apart
    // from a weak model unless the number is visible somewhere.
    if let Err(TrySendError::Full(_)) = sender.try_send(mono) {
        diagnostics.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// Meters every chunk and, when a recogniser is present, transcribes it.
#[allow(clippy::too_many_arguments)]
fn run_worker(
    app: AppHandle,
    receiver: Receiver<Vec<f32>>,
    mut recognizer: Option<Recognizer>,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
    diagnostics: Arc<Diagnostics>,
) {
    let started = Instant::now();
    let mut last_transcript = String::new();
    let mut last_broadcast = Instant::now() - BROADCAST_INTERVAL;

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

        // The gate is fed from every chunk, but the UI is not told about every
        // chunk. Audio arrives about a hundred times a second and each report
        // is a JSON payload across the IPC boundary — enough traffic to starve
        // the thread that has to decode speech in real time.
        if last_broadcast.elapsed() >= BROADCAST_INTERVAL {
            last_broadcast = Instant::now();
            crate::broadcast(&app, progress);
        }

        if is_muted {
            continue;
        }

        let Some(recognizer) = recognizer.as_mut() else {
            continue;
        };

        recognizer.accept(sample_rate, &chunk);
        // Only read the transcript when the network actually ran. Fetching it
        // on every chunk copies the whole string a hundred times a second for
        // a value that changes perhaps three times a second.
        if !recognizer.decode() {
            continue;
        }
        diagnostics.decodes.fetch_add(1, Ordering::Relaxed);

        let update = recognizer.result();
        if !update.text.is_empty() && update.text != last_transcript {
            last_transcript = update.text.clone();
            *diagnostics.heard.lock().unwrap() = update.text.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_device_id_falls_back_to_the_system_default() {
        let host = cpal::default_host();
        // Hardware-dependent: a build machine with no microphone at all has
        // nothing to fall back to, and that is not this test's concern.
        let Some(default_device) = host.default_input_device() else {
            return;
        };
        let default_id = default_device.id().unwrap();

        let resolved = resolve_device(&host, Some("wasapi:definitely-not-a-real-microphone-xyz"))
            .expect("falls back instead of failing");
        assert_eq!(resolved.id().unwrap(), default_id);
    }

    #[test]
    fn a_device_id_that_fails_to_parse_falls_back_to_the_system_default() {
        let host = cpal::default_host();
        let Some(default_device) = host.default_input_device() else {
            return;
        };
        let default_id = default_device.id().unwrap();

        // No "host:device" separator at all — a corrupt or hand-edited
        // settings file, not just an unplugged device.
        let resolved =
            resolve_device(&host, Some("not a device id")).expect("falls back instead of failing");
        assert_eq!(resolved.id().unwrap(), default_id);
    }

    #[test]
    fn no_device_id_resolves_to_the_system_default() {
        let host = cpal::default_host();
        let Some(default_device) = host.default_input_device() else {
            return;
        };
        let default_id = default_device.id().unwrap();

        let resolved = resolve_device(&host, None).expect("a default device is present");
        assert_eq!(resolved.id().unwrap(), default_id);
    }

    #[test]
    fn a_device_id_that_matches_a_real_device_resolves_to_it() {
        let host = cpal::default_host();
        let Some(default_device) = host.default_input_device() else {
            return;
        };
        let default_id = default_device.id().unwrap().to_string();

        let resolved =
            resolve_device(&host, Some(&default_id)).expect("the named device exists");
        assert_eq!(resolved.id().unwrap().to_string(), default_id);
    }

    #[test]
    fn listed_devices_are_flagged_default_at_most_once_each() {
        let devices = list_input_devices();
        let defaults = devices.iter().filter(|d| d.is_default).count();
        // Zero on a build machine with no microphone, one otherwise — never
        // more, since only one device can be the system default at a time.
        assert!(defaults <= 1);
    }

    fn captured_chunk(samples: &[f32], channels: usize) -> (Vec<f32>, usize) {
        let (sender, receiver) = sync_channel(1);
        let diagnostics = Diagnostics::default();
        forward(samples.iter().copied(), channels, &sender, &diagnostics);
        (
            receiver.try_recv().expect("forwarded audio chunk"),
            diagnostics.dropped.load(Ordering::Relaxed),
        )
    }

    #[test]
    fn stereo_high_rate_frames_are_downmixed_without_changing_frame_count() {
        // One millisecond of 192 kHz stereo input. The sample rate itself is
        // carried separately into the recogniser; this verifies that capture
        // does not reinterpret, resample, or discard high-rate frames while
        // downmixing them to mono.
        let frames = 192usize;
        let mut stereo = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let left = i as f32 / frames as f32;
            stereo.push(left);
            stereo.push(-left);
        }

        let (mono, dropped) = captured_chunk(&stereo, 2);
        assert_eq!(mono.len(), frames);
        assert!(mono.iter().all(|sample| sample.abs() < 1e-6));
        assert_eq!(dropped, 0);
    }

    #[test]
    fn multichannel_input_is_averaged_per_frame() {
        let input = [1.0, 0.5, -0.5, 0.0, -1.0, 0.5, 0.5, 0.0];
        let (mono, dropped) = captured_chunk(&input, 4);
        assert_eq!(mono, vec![0.25, 0.0]);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn a_full_audio_queue_drops_instead_of_blocking_the_callback() {
        let (sender, _receiver) = sync_channel(1);
        let diagnostics = Diagnostics::default();

        forward([0.1, 0.2].into_iter(), 1, &sender, &diagnostics);
        forward([0.3, 0.4].into_iter(), 1, &sender, &diagnostics);

        assert_eq!(diagnostics.dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn waveform_broadcast_interval_stays_at_twenty_hz() {
        assert_eq!(BROADCAST_INTERVAL, Duration::from_millis(50));
    }
}
