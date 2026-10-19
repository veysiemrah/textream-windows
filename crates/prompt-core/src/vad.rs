//! Microphone level metering and voice activity detection.

/// Root-mean-square level of a buffer, normalised to `0.0..=1.0`.
///
/// Non-finite samples are dropped rather than poisoning the sum — a single NaN
/// from a glitching capture device would otherwise make the level NaN and every
/// downstream comparison false, silently wedging Voice-Activated mode shut.
pub fn normalized_rms(samples: &[f32]) -> f32 {
    let mut squared_total = 0.0f64;
    let mut count = 0usize;
    for &sample in samples {
        if !sample.is_finite() {
            continue;
        }
        squared_total += (sample as f64) * (sample as f64);
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    ((squared_total / count as f64).sqrt() as f32).min(1.0)
}

/// Converts interleaved 16-bit PCM to normalised RMS.
pub fn normalized_rms_i16(samples: &[i16]) -> f32 {
    let converted: Vec<f32> = samples.iter().map(|&s| s as f32 / 32_768.0).collect();
    normalized_rms(&converted)
}

/// The background level a session starts out assuming: a quiet room on a
/// typical microphone.
const INITIAL_NOISE_FLOOR: f32 = 0.002;

/// How fast the noise floor follows the level, in seconds. It falls quickly,
/// into every pause between words, and rises slowly, so speech barely moves
/// it while steady noise — a fan, air conditioning — becomes the floor within
/// a few seconds.
const FLOOR_FALL_SECONDS: f64 = 0.15;
const FLOOR_RISE_SECONDS: f64 = 4.0;

/// How far above the noise floor speech must be, as a level ratio: sustained
/// (about +10 dB) and to open the gate at once (about +18 dB).
const SUSTAINED_ABOVE_FLOOR: f32 = 3.0;
const IMMEDIATE_ABOVE_FLOOR: f32 = 8.0;

/// Hysteretic speech gate over an adaptive noise floor.
///
/// Two thresholds and a hangover, because naive level gating chatters: a single
/// threshold flickers on breath and consonant gaps, which in Voice-Activated
/// mode reads as the script stuttering. Quiet speech must cross the low
/// threshold on consecutive frames to open the gate, anything clearly loud
/// opens it at once, and the gate then stays open through the hangover so
/// inter-word pauses do not close it.
///
/// Both thresholds sit relative to a tracked noise floor, never below fixed
/// minimums. Fixed thresholds alone fail both ways: a low-gain microphone
/// never reaches them, and a noisy room never drops below them, so Voice-
/// Activated either never scrolls or never stops. Upstream Textream made the
/// same change in f/textream#61.
#[derive(Debug, Clone)]
pub struct VoiceActivityDetector {
    activation_level: f32,
    immediate_activation_level: f32,
    required_active_frames: u32,
    hangover: f64,
    consecutive_active_frames: u32,
    active_until: Option<f64>,
    noise_floor: f32,
    last_timestamp: Option<f64>,
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        Self::new(0.006, 0.04, 2, 0.75)
    }
}

impl VoiceActivityDetector {
    /// * `activation_level` — the lowest sustained level that counts as speech,
    ///   however quiet the background.
    /// * `immediate_activation_level` — the lowest level that opens the gate
    ///   instantly.
    /// * `required_active_frames` — frames above the sustained level needed.
    /// * `hangover` — seconds the gate stays open after the last active frame.
    pub fn new(
        activation_level: f32,
        immediate_activation_level: f32,
        required_active_frames: u32,
        hangover: f64,
    ) -> Self {
        Self {
            activation_level,
            immediate_activation_level,
            required_active_frames: required_active_frames.max(1),
            hangover,
            consecutive_active_frames: 0,
            active_until: None,
            noise_floor: INITIAL_NOISE_FLOOR,
            last_timestamp: None,
        }
    }

    /// Feeds one metered frame. `timestamp` is in seconds, monotonic.
    pub fn process(&mut self, level: f32, timestamp: f64) {
        // Judged against the floor as it stood before this frame, so a sudden
        // loud frame is not partly absorbed into the floor it is measured by.
        let sustained = self
            .activation_level
            .max(self.noise_floor * SUSTAINED_ABOVE_FLOOR);
        let immediate = self
            .immediate_activation_level
            .max(self.noise_floor * IMMEDIATE_ABOVE_FLOOR);

        if level >= immediate {
            self.consecutive_active_frames = self.required_active_frames;
            self.extend(timestamp);
        } else if level >= sustained {
            self.consecutive_active_frames += 1;
            if self.consecutive_active_frames >= self.required_active_frames {
                self.extend(timestamp);
            }
        } else {
            self.consecutive_active_frames = 0;
        }

        self.track_floor(level, timestamp);
    }

    /// Moves the noise floor towards `level`, at a rate set by time rather
    /// than by frame count, so it behaves the same whatever size of chunks the
    /// audio device delivers.
    fn track_floor(&mut self, level: f32, timestamp: f64) {
        let elapsed = self
            .last_timestamp
            .map_or(0.0, |last| (timestamp - last).clamp(0.0, 0.5));
        self.last_timestamp = Some(timestamp);
        if !level.is_finite() || elapsed == 0.0 {
            return;
        }
        let seconds = if level < self.noise_floor {
            FLOOR_FALL_SECONDS
        } else {
            FLOOR_RISE_SECONDS
        };
        let step = 1.0 - (-elapsed / seconds).exp();
        self.noise_floor += (level - self.noise_floor) * step as f32;
    }

    /// Whether speech is considered active at `timestamp`.
    pub fn is_active(&self, timestamp: f64) -> bool {
        self.active_until.is_some_and(|until| timestamp < until)
    }

    pub fn reset(&mut self) {
        self.consecutive_active_frames = 0;
        self.active_until = None;
        self.noise_floor = INITIAL_NOISE_FLOOR;
        self.last_timestamp = None;
    }

    fn extend(&mut self, timestamp: f64) {
        let until = timestamp + self.hangover;
        self.active_until = Some(match self.active_until {
            Some(current) => current.max(until),
            None => until,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_measures_zero() {
        assert_eq!(normalized_rms(&[0.0; 128]), 0.0);
        assert_eq!(normalized_rms(&[]), 0.0);
    }

    #[test]
    fn full_scale_measures_one() {
        assert_eq!(normalized_rms(&[1.0, -1.0, 1.0, -1.0]), 1.0);
    }

    #[test]
    fn non_finite_samples_are_ignored() {
        let level = normalized_rms(&[f32::NAN, 0.5, f32::INFINITY, 0.5]);
        assert!((level - 0.5).abs() < 1e-6);
    }

    #[test]
    fn all_non_finite_degrades_to_silence() {
        assert_eq!(normalized_rms(&[f32::NAN, f32::INFINITY]), 0.0);
    }

    #[test]
    fn i16_conversion_matches_float() {
        assert!(normalized_rms_i16(&[16_384, -16_384]) > 0.49);
    }

    #[test]
    fn quiet_speech_needs_consecutive_frames() {
        let mut vad = VoiceActivityDetector::default();
        vad.process(0.02, 0.0);
        assert!(
            !vad.is_active(0.0),
            "one quiet frame should not open the gate"
        );
        vad.process(0.02, 0.1);
        assert!(vad.is_active(0.1));
    }

    #[test]
    fn loud_speech_opens_the_gate_immediately() {
        let mut vad = VoiceActivityDetector::default();
        vad.process(0.5, 0.0);
        assert!(vad.is_active(0.0));
    }

    #[test]
    fn hangover_bridges_gaps_between_words() {
        let mut vad = VoiceActivityDetector::default();
        vad.process(0.5, 1.0);
        assert!(vad.is_active(1.5), "still inside the 0.75 s hangover");
        assert!(!vad.is_active(1.8), "hangover elapsed");
    }

    #[test]
    fn a_gap_below_threshold_restarts_the_frame_count() {
        let mut vad = VoiceActivityDetector::default();
        vad.process(0.02, 0.0);
        vad.process(0.0, 0.1); // drop out
        vad.process(0.02, 0.2);
        assert!(!vad.is_active(0.2), "the counter should have restarted");
    }

    /// Feeds `level` every 10 ms from `start` for `seconds`; returns the end.
    fn feed(vad: &mut VoiceActivityDetector, level: f32, start: f64, seconds: f64) -> f64 {
        let frames = (seconds / 0.01).round() as usize;
        for frame in 0..frames {
            vad.process(level, start + frame as f64 * 0.01);
        }
        start + frames as f64 * 0.01
    }

    #[test]
    fn a_low_gain_microphone_still_opens_the_gate() {
        // A quiet room on a mic with little gain: speech peaks well under the
        // level a typical mic speaks at, but far above its own background.
        let mut vad = VoiceActivityDetector::default();
        let now = feed(&mut vad, 0.0008, 0.0, 3.0);
        let now = feed(&mut vad, 0.009, now, 0.2);
        assert!(vad.is_active(now));
    }

    #[test]
    fn steady_background_noise_does_not_hold_the_gate_open() {
        // A fan or air conditioning, louder than quiet speech on some mics.
        let mut vad = VoiceActivityDetector::default();
        let now = feed(&mut vad, 0.02, 0.0, 10.0);
        assert!(!vad.is_active(now), "the noise became the floor");

        // Speech over that noise still gets through.
        let now = feed(&mut vad, 0.09, now, 0.2);
        assert!(vad.is_active(now));
    }

    #[test]
    fn speech_with_pauses_between_words_keeps_the_gate_working() {
        // Words at 0.05 with short dips: the floor follows the dips, not the
        // words, so a long passage does not shut the gate on its own speaker.
        let mut vad = VoiceActivityDetector::default();
        let mut now = feed(&mut vad, 0.001, 0.0, 1.0);
        for _ in 0..30 {
            now = feed(&mut vad, 0.05, now, 0.35);
            now = feed(&mut vad, 0.002, now, 0.12);
        }
        now = feed(&mut vad, 0.05, now, 0.1);
        assert!(vad.is_active(now));
    }

    #[test]
    fn starts_closed_and_reset_closes_it_again() {
        let mut vad = VoiceActivityDetector::default();
        assert!(!vad.is_active(0.0));
        vad.process(0.9, 0.0);
        assert!(vad.is_active(0.1));
        vad.reset();
        assert!(!vad.is_active(0.1));
    }
}
