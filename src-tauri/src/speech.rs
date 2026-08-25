//! Streaming on-device speech recognition.
//!
//! sherpa-onnx ships a streaming (online) recogniser, but `sherpa-rs` only
//! wraps the *offline* one — its `TransducerRecognizer` decodes a finished
//! buffer. A teleprompter cannot wait for the presenter to stop talking, so the
//! online C API is called directly here.
//!
//! Nothing leaves the machine. That is the whole reason for carrying an ONNX
//! runtime instead of calling a cloud speech API that would be more accurate
//! and one HTTP request away.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use sherpa_rs::sherpa_rs_sys as sys;

/// Sample rate the models expect. `AcceptWaveform` resamples anything else
/// internally, so the capture device is free to run at its native rate.
pub const MODEL_SAMPLE_RATE: i32 = 16_000;

/// Filter-bank dimension the shipped models were trained with.
const FEATURE_DIM: i32 = 80;

/// Decode threads. Two keeps up with real time on a modern CPU while leaving
/// headroom for whatever the presenter is streaming with.
const NUM_THREADS: i32 = 2;

/// The four files that make up a streaming transducer.
#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

impl ModelPaths {
    /// Resolves the standard layout inside a model directory.
    pub fn in_directory(directory: &Path, files: &ModelFiles) -> Self {
        Self {
            encoder: directory.join(files.encoder),
            decoder: directory.join(files.decoder),
            joiner: directory.join(files.joiner),
            tokens: directory.join(files.tokens),
        }
    }

    pub fn all_present(&self) -> bool {
        [&self.encoder, &self.decoder, &self.joiner, &self.tokens]
            .iter()
            .all(|path| path.is_file())
    }
}

/// File names within a published model repository.
#[derive(Debug, Clone, Copy)]
pub struct ModelFiles {
    pub encoder: &'static str,
    pub decoder: &'static str,
    pub joiner: &'static str,
    pub tokens: &'static str,
}

/// What the recogniser has produced so far.
#[derive(Debug, Clone, Default)]
pub struct Update {
    /// Everything decoded since the last [`Recognizer::reset`].
    pub text: String,
    /// The recogniser considers the utterance finished.
    pub endpoint: bool,
}

/// A live streaming recogniser and its single stream.
pub struct Recognizer {
    recognizer: *const sys::SherpaOnnxOnlineRecognizer,
    stream: *const sys::SherpaOnnxOnlineStream,
}

// SAFETY: sherpa-onnx guards the recogniser internally, and this type owns its
// stream exclusively — it is created on the audio worker and never shared.
unsafe impl Send for Recognizer {}

impl Recognizer {
    /// Loads a model and opens a stream.
    pub fn new(paths: &ModelPaths) -> Result<Self, String> {
        if !paths.all_present() {
            return Err("speech model files are missing".into());
        }

        let encoder = cstring(&paths.encoder)?;
        let decoder = cstring(&paths.decoder)?;
        let joiner = cstring(&paths.joiner)?;
        let tokens = cstring(&paths.tokens)?;
        let provider = CString::new("cpu").unwrap();
        // Greedy decoding: beam search buys accuracy this app does not need,
        // because the script is already known and the matcher tolerates
        // mis-recognised words. It would only cost latency.
        let decoding_method = CString::new("greedy_search").unwrap();

        // SAFETY: every pointer below outlives the create call, which copies
        // the strings it is given. Fields this app does not use are zeroed,
        // matching how sherpa-onnx expects optional configs to be left.
        let recognizer = unsafe {
            let config = sys::SherpaOnnxOnlineRecognizerConfig {
                feat_config: sys::SherpaOnnxFeatureConfig {
                    sample_rate: MODEL_SAMPLE_RATE,
                    feature_dim: FEATURE_DIM,
                },
                model_config: sys::SherpaOnnxOnlineModelConfig {
                    transducer: sys::SherpaOnnxOnlineTransducerModelConfig {
                        encoder: encoder.as_ptr(),
                        decoder: decoder.as_ptr(),
                        joiner: joiner.as_ptr(),
                    },
                    tokens: tokens.as_ptr(),
                    num_threads: NUM_THREADS,
                    provider: provider.as_ptr(),
                    debug: 0,
                    paraformer: std::mem::zeroed(),
                    zipformer2_ctc: std::mem::zeroed(),
                    model_type: std::mem::zeroed(),
                    modeling_unit: std::mem::zeroed(),
                    bpe_vocab: std::mem::zeroed(),
                    tokens_buf: std::mem::zeroed(),
                    tokens_buf_size: 0,
                    nemo_ctc: std::mem::zeroed(),
                },
                decoding_method: decoding_method.as_ptr(),
                max_active_paths: 4,
                // Endpointing bounds how long a single stream grows. Each
                // endpoint rebases the matcher's window instead of losing the
                // reading position, so frequent endpoints are harmless and an
                // ever-growing transcript is not.
                enable_endpoint: 1,
                rule1_min_trailing_silence: 2.4,
                rule2_min_trailing_silence: 1.2,
                rule3_min_utterance_length: 20.0,
                hotwords_file: std::mem::zeroed(),
                hotwords_score: 0.0,
                ctc_fst_decoder_config: std::mem::zeroed(),
                rule_fsts: std::mem::zeroed(),
                rule_fars: std::mem::zeroed(),
                blank_penalty: 0.0,
                hotwords_buf: std::mem::zeroed(),
                hotwords_buf_size: 0,
                hr: std::mem::zeroed(),
            };

            let recognizer = sys::SherpaOnnxCreateOnlineRecognizer(&config);
            if recognizer.is_null() {
                return Err("sherpa-onnx rejected the speech model".into());
            }
            recognizer
        };

        // SAFETY: `recognizer` was just checked non-null.
        let stream = unsafe { sys::SherpaOnnxCreateOnlineStream(recognizer) };
        if stream.is_null() {
            // SAFETY: undo the successful create before bailing out.
            unsafe { sys::SherpaOnnxDestroyOnlineRecognizer(recognizer) };
            return Err("could not open a speech stream".into());
        }

        Ok(Self { recognizer, stream })
    }

    /// Feeds captured audio. `samples` must be mono in `-1.0..=1.0`.
    pub fn accept(&mut self, sample_rate: u32, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        // SAFETY: the slice is valid for the duration of the call and
        // sherpa-onnx copies what it needs.
        unsafe {
            sys::SherpaOnnxOnlineStreamAcceptWaveform(
                self.stream,
                sample_rate as i32,
                samples.as_ptr(),
                samples.len() as i32,
            );
        }
    }

    /// Decodes whatever is ready and reports the transcript so far.
    pub fn poll(&mut self) -> Update {
        // SAFETY: both handles are non-null for the lifetime of `self`, and
        // the result pointer is destroyed before it goes out of scope.
        unsafe {
            while sys::SherpaOnnxIsOnlineStreamReady(self.recognizer, self.stream) == 1 {
                sys::SherpaOnnxDecodeOnlineStream(self.recognizer, self.stream);
            }

            let result = sys::SherpaOnnxGetOnlineStreamResult(self.recognizer, self.stream);
            let text = if result.is_null() {
                String::new()
            } else {
                let text = CStr::from_ptr((*result).text)
                    .to_string_lossy()
                    .into_owned();
                sys::SherpaOnnxDestroyOnlineRecognizerResult(result);
                text
            };

            let endpoint = sys::SherpaOnnxOnlineStreamIsEndpoint(self.recognizer, self.stream) == 1;

            Update { text, endpoint }
        }
    }

    /// Clears the stream so the next transcript starts from nothing.
    pub fn reset(&mut self) {
        // SAFETY: both handles are non-null for the lifetime of `self`.
        unsafe { sys::SherpaOnnxOnlineStreamReset(self.recognizer, self.stream) };
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        // SAFETY: destroying the stream before its recogniser is the order
        // sherpa-onnx requires.
        unsafe {
            sys::SherpaOnnxDestroyOnlineStream(self.stream);
            sys::SherpaOnnxDestroyOnlineRecognizer(self.recognizer);
        }
    }
}

/// Converts a path for the C API, which takes UTF-8 and no interior nulls.
fn cstring(path: &Path) -> Result<CString, String> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| format!("path contains a null byte: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: ModelFiles = ModelFiles {
        encoder: "encoder.onnx",
        decoder: "decoder.onnx",
        joiner: "joiner.onnx",
        tokens: "tokens.txt",
    };

    #[test]
    fn paths_follow_the_published_layout() {
        let paths = ModelPaths::in_directory(Path::new("C:/models/en"), &FILES);
        assert!(paths.encoder.ends_with("encoder.onnx"));
        assert!(paths.tokens.ends_with("tokens.txt"));
    }

    #[test]
    fn a_missing_model_is_detected_without_touching_the_c_api() {
        let paths = ModelPaths::in_directory(Path::new("C:/definitely/not/here"), &FILES);
        assert!(!paths.all_present());
        assert!(Recognizer::new(&paths).is_err());
    }

    #[test]
    fn paths_convert_for_the_c_api() {
        assert!(cstring(Path::new("C:/models/en/encoder.onnx")).is_ok());
    }

    /// Loads a real model and runs audio through it.
    ///
    /// Ignored by default because it needs a downloaded model, but it is the
    /// only test that exercises the FFI config for real. The struct layout here
    /// is hand-written against sherpa-onnx's headers and the unused fields are
    /// zeroed — get a field wrong and this is where it shows up, rather than in
    /// front of a presenter mid-take.
    ///
    /// ```text
    /// TEXTREAM_TEST_MODEL_DIR=<dir> cargo test -p textream -- --ignored
    /// ```
    #[test]
    #[ignore = "needs a downloaded speech model"]
    fn a_real_model_loads_and_decodes() {
        let Ok(directory) = std::env::var("TEXTREAM_TEST_MODEL_DIR") else {
            panic!("set TEXTREAM_TEST_MODEL_DIR to a directory holding a model");
        };
        let files = ModelFiles {
            encoder: "encoder-epoch-99-avg-1.int8.onnx",
            decoder: "decoder-epoch-99-avg-1.int8.onnx",
            joiner: "joiner-epoch-99-avg-1.int8.onnx",
            tokens: "tokens.txt",
        };
        let paths = ModelPaths::in_directory(Path::new(&directory), &files);
        assert!(paths.all_present(), "model files missing in {directory}");

        let mut recognizer = Recognizer::new(&paths).expect("recogniser should load");

        // Two seconds of silence at the model's own rate. Silence must decode
        // to nothing rather than hallucinate, and must not trip an endpoint
        // before the trailing-silence rule is satisfied.
        let silence = vec![0.0f32; MODEL_SAMPLE_RATE as usize * 2];
        recognizer.accept(MODEL_SAMPLE_RATE as u32, &silence);
        let update = recognizer.poll();
        assert!(
            update.text.trim().is_empty(),
            "silence decoded to {:?}",
            update.text
        );

        // A tone is not speech either, but it proves the feature pipeline runs
        // over real signal without tearing down the stream.
        let tone: Vec<f32> = (0..MODEL_SAMPLE_RATE)
            .map(|i| (i as f32 * 0.05).sin() * 0.2)
            .collect();
        recognizer.accept(MODEL_SAMPLE_RATE as u32, &tone);
        let _ = recognizer.poll();

        recognizer.reset();
        assert!(recognizer.poll().text.trim().is_empty());
    }
}
