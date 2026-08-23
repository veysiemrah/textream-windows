/**
 * Microphone level metering for Voice-Activated mode.
 *
 * RMS is computed here rather than in Rust on purpose: shipping every audio
 * buffer across the IPC boundary to derive one scalar would cost far more than
 * the arithmetic saves. `prompt_core::normalized_rms` remains the reference
 * implementation for when audio capture moves native.
 */

export interface Meter {
  /** Latest level, 0..1. */
  level(): number;
  stop(): void;
}

/** Analyser window. 1024 samples is ~21 ms at 48 kHz — fine enough for the
 * two-frame activation the gate expects, coarse enough to stay cheap. */
const FFT_SIZE = 1024;

/**
 * Opens the default input device and starts metering.
 *
 * Throws if the user denies permission or no device exists; the caller is
 * expected to surface that rather than silently running a mode that needs a
 * microphone.
 */
export async function startMeter(): Promise<Meter> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      echoCancellation: false,
      noiseSuppression: false,
      autoGainControl: false,
    },
  });

  const context = new AudioContext();
  const source = context.createMediaStreamSource(stream);
  const analyser = context.createAnalyser();
  analyser.fftSize = FFT_SIZE;
  source.connect(analyser);

  const buffer = new Float32Array(analyser.fftSize);

  return {
    level() {
      analyser.getFloatTimeDomainData(buffer);
      let squaredTotal = 0;
      let count = 0;
      for (const sample of buffer) {
        if (!Number.isFinite(sample)) continue;
        squaredTotal += sample * sample;
        count += 1;
      }
      if (count === 0) return 0;
      return Math.min(1, Math.sqrt(squaredTotal / count));
    },
    stop() {
      source.disconnect();
      for (const track of stream.getTracks()) track.stop();
      void context.close();
    },
  };
}
