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

/// Hysteretic speech gate.
///
/// Two thresholds and a hangover, because naive level gating chatters: a single
/// threshold flickers on breath and consonant gaps, which in Voice-Activated
/// mode reads as the script stuttering. Quiet speech must cross the low
/// threshold on consecutive frames to open the gate, anything clearly loud
/// opens it at once, and the gate then stays open through the hangover so
/// inter-word pauses do not close it.
#[derive(Debug, Clone)]
pub struct VoiceActivityDetector {
    activation_level: f32,
    immediate_activation_level: f32,
    required_active_frames: u32,
    hangover: f64,
    consecutive_active_frames: u32,
    active_until: Option<f64>,
}

impl Default for VoiceActivityDetector {
    fn default() -> Self {
        Self::new(0.012, 0.04, 2, 0.75)
    }
}

impl VoiceActivityDetector {
    /// * `activation_level` — sustained level that counts as speech.
    /// * `immediate_activation_level` — level that opens the gate instantly.
    /// * `required_active_frames` — frames above `activation_level` needed.
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
        }
    }

    /// Feeds one metered frame. `timestamp` is in seconds, monotonic.
    pub fn process(&mut self, level: f32, timestamp: f64) {
        if level >= self.immediate_activation_level {
            self.consecutive_active_frames = self.required_active_frames;
            self.extend(timestamp);
        } else if level >= self.activation_level {
            self.consecutive_active_frames += 1;
            if self.consecutive_active_frames >= self.required_active_frames {
                self.extend(timestamp);
            }
        } else {
            self.consecutive_active_frames = 0;
        }
    }

    /// Whether speech is considered active at `timestamp`.
    pub fn is_active(&self, timestamp: f64) -> bool {
        self.active_until.is_some_and(|until| timestamp < until)
    }

    pub fn reset(&mut self) {
        self.consecutive_active_frames = 0;
        self.active_until = None;
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
