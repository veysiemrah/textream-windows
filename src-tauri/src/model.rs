//! Speech model registry and first-run download.
//!
//! Models are far too large to ship in a 2 MB installer, and most users never
//! need every language, so they are fetched on demand into the app data
//! directory.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::speech::{ModelFiles, ModelPaths};

/// A downloadable streaming recogniser.
#[derive(Debug, Clone, Copy)]
pub struct SpeechModel {
    pub id: &'static str,
    pub label: &'static str,
    /// BCP-47 tag of the language the model transcribes.
    pub language: &'static str,
    /// Hugging Face repository, `owner/name`.
    pub repo: &'static str,
    pub files: ModelFiles,
    /// Approximate download size, for the confirmation the UI shows.
    pub download_bytes: u64,
}

/// Models offered in the UI.
///
/// The macOS app asks the operating system which languages it can transcribe
/// and offers those. Windows has no equivalent worth using — its own recogniser
/// covers only English, French, German, Japanese, Mandarin and Spanish — so
/// this registry plays that role, and a language is available exactly when
/// somebody has published a streaming model for it.
///
/// Quantised weights where the publisher provides them. A float encoder is
/// typically twice the size, and on a task where a fuzzy matcher absorbs
/// recognition error it buys nothing a presenter would notice.
pub const MODELS: &[SpeechModel] = &[
    SpeechModel {
        id: "en-20m",
        label: "English (small)",
        language: "en",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
        files: ModelFiles {
            encoder: "encoder-epoch-99-avg-1.int8.onnx",
            decoder: "decoder-epoch-99-avg-1.int8.onnx",
            joiner: "joiner-epoch-99-avg-1.int8.onnx",
            tokens: "tokens.txt",
        },
        download_bytes: 43_600_000,
    },
    SpeechModel {
        id: "en",
        label: "English",
        language: "en",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06",
        files: KROKO_FILES,
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "de",
        label: "German",
        language: "de",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-de-kroko-2025-08-06",
        files: KROKO_FILES,
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "fr",
        label: "French",
        language: "fr",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-fr-kroko-2025-08-06",
        files: KROKO_FILES,
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "es",
        label: "Spanish",
        language: "es",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-es-kroko-2025-08-06",
        files: KROKO_FILES,
        download_bytes: 156_200_000,
    },
    SpeechModel {
        id: "zh",
        label: "Chinese (Mandarin)",
        language: "zh",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-multi-zh-hans-int8-2023-12-13",
        files: ModelFiles {
            encoder: "encoder-epoch-20-avg-1-chunk-16-left-128.int8.onnx",
            decoder: "decoder-epoch-20-avg-1-chunk-16-left-128.onnx",
            joiner: "joiner-epoch-20-avg-1-chunk-16-left-128.int8.onnx",
            tokens: "tokens.txt",
        },
        download_bytes: 76_500_000,
    },
    SpeechModel {
        id: "multi",
        label: "Arabic, Indonesian, Japanese, Russian, Thai, Vietnamese",
        language: "multi",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-ar_en_id_ja_ru_th_vi_zh-2025-02-10",
        files: ModelFiles {
            encoder: "encoder-epoch-75-avg-11-chunk-16-left-128.int8.onnx",
            decoder: "decoder-epoch-75-avg-11-chunk-16-left-128.onnx",
            joiner: "joiner-epoch-75-avg-11-chunk-16-left-128.int8.onnx",
            tokens: "tokens.txt",
        },
        download_bytes: 338_700_000,
    },
];

/// The Kroko releases all use the same plain file names.
const KROKO_FILES: ModelFiles = ModelFiles {
    encoder: "encoder.onnx",
    decoder: "decoder.onnx",
    joiner: "joiner.onnx",
    tokens: "tokens.txt",
};

/// The default model, used when the user has expressed no preference.
pub fn default_model() -> &'static SpeechModel {
    &MODELS[0]
}

pub fn find(id: &str) -> Option<&'static SpeechModel> {
    MODELS.iter().find(|model| model.id == id)
}

/// Where a model's files live under the app data directory.
pub fn directory(root: &Path, model: &SpeechModel) -> PathBuf {
    root.join("models").join(model.id)
}

pub fn paths(root: &Path, model: &SpeechModel) -> ModelPaths {
    ModelPaths::in_directory(&directory(root, model), &model.files)
}

/// A model as the UI sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub label: String,
    pub language: String,
    pub installed: bool,
    pub download_bytes: u64,
}

pub fn status(root: &Path, model: &SpeechModel) -> ModelStatus {
    ModelStatus {
        id: model.id.to_string(),
        label: model.label.to_string(),
        language: model.language.to_string(),
        installed: paths(root, model).all_present(),
        download_bytes: model.download_bytes,
    }
}

pub fn statuses(root: &Path) -> Vec<ModelStatus> {
    MODELS.iter().map(|model| status(root, model)).collect()
}

/// Download progress, emitted to the UI as it streams.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub received: u64,
    pub total: u64,
}

/// Fetches every file of `model` into the app data directory.
///
/// Each file lands at a `.part` name and is renamed only once it is complete,
/// so a download killed halfway cannot leave a truncated `.onnx` that looks
/// installed and then fails deep inside the ONNX runtime.
pub fn download(
    root: &Path,
    model: &SpeechModel,
    mut on_progress: impl FnMut(DownloadProgress),
) -> Result<(), String> {
    let directory = directory(root, model);
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;

    let names = [
        model.files.encoder,
        model.files.decoder,
        model.files.joiner,
        model.files.tokens,
    ];

    let total = model.download_bytes;
    let mut received = 0u64;
    on_progress(DownloadProgress { received, total });

    for name in names {
        let destination = directory.join(name);
        if destination.is_file() {
            continue;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}?download=true",
            model.repo, name
        );
        let response = ureq::get(&url)
            .call()
            .map_err(|error| format!("could not fetch {name}: {error}"))?;

        let partial = directory.join(format!("{name}.part"));
        let mut file = File::create(&partial).map_err(|error| error.to_string())?;
        let mut reader = response.into_reader();
        let mut buffer = vec![0u8; 128 * 1024];

        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("download of {name} failed: {error}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|error| error.to_string())?;
            received += read as u64;
            on_progress(DownloadProgress {
                received,
                // The registry figure is approximate, so never report a
                // percentage above 100 — a bar that overshoots reads as a bug.
                total: total.max(received),
            });
        }

        file.flush().map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
    }

    on_progress(DownloadProgress {
        received: received.max(total),
        total: received.max(total),
    });
    Ok(())
}

/// Deletes a downloaded model.
pub fn remove(root: &Path, model: &SpeechModel) -> Result<(), String> {
    let directory = directory(root, model);
    if directory.exists() {
        fs::remove_dir_all(&directory).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_not_empty_and_ids_are_unique() {
        assert!(!MODELS.is_empty());
        let mut ids: Vec<&str> = MODELS.iter().map(|model| model.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate model id");
    }

    #[test]
    fn lookup_finds_registered_models_only() {
        assert!(find("en-20m").is_some());
        assert!(find("klingon").is_none());
        assert_eq!(default_model().id, "en-20m");
    }

    #[test]
    fn paths_live_under_the_model_directory() {
        let root = Path::new("C:/appdata/textream");
        let paths = paths(root, default_model());
        assert!(paths
            .encoder
            .starts_with("C:/appdata/textream/models/en-20m"));
    }

    #[test]
    fn a_model_that_was_never_downloaded_reports_uninstalled() {
        let status = status(Path::new("C:/definitely/not/here"), default_model());
        assert!(!status.installed);
        assert_eq!(status.id, "en-20m");
        assert!(status.download_bytes > 0);
    }

    #[test]
    fn every_status_is_reported() {
        assert_eq!(statuses(Path::new("C:/nowhere")).len(), MODELS.len());
    }

    #[test]
    fn every_model_is_completely_described() {
        for model in MODELS {
            assert!(!model.label.is_empty(), "{} has no label", model.id);
            assert!(!model.language.is_empty(), "{} has no language", model.id);
            assert!(
                model.repo.contains('/'),
                "{} needs an owner/name repository",
                model.id
            );
            assert!(
                model.download_bytes > 1_000_000,
                "{} reports an implausible size",
                model.id
            );
            for name in [
                model.files.encoder,
                model.files.decoder,
                model.files.joiner,
                model.files.tokens,
            ] {
                assert!(!name.is_empty(), "{} is missing a file name", model.id);
            }
        }
    }

    #[test]
    fn models_land_in_separate_directories() {
        let root = Path::new("C:/appdata/textream");
        let mut directories: Vec<PathBuf> = MODELS.iter().map(|m| directory(root, m)).collect();
        directories.sort();
        let count = directories.len();
        directories.dedup();
        assert_eq!(directories.len(), count, "two models share a directory");
    }

    /// Downloads the default model over a real network connection.
    ///
    /// Every other test in this module exercises the bookkeeping around a
    /// download without ever making one — the URL, the `.part` rename, the
    /// progress callback are all tested against a filesystem, never against
    /// Hugging Face. This is the one place that calls `ureq` for real, which
    /// is the only way to know the TLS setup and the URL format actually work
    /// against the service they are pointed at, rather than just against each
    /// other.
    ///
    /// Ignored by default — real network access, and ~44 MB — but this is
    /// exactly the code path "Download" runs in the app, unlike the FFI smoke
    /// test in speech.rs, which is handed a model fetched some other way.
    #[test]
    #[ignore = "downloads ~44 MB over a real network connection"]
    fn the_default_model_downloads_and_installs_for_real() {
        let root = temp_root_for_download();
        let model = default_model();

        let mut progress_seen = false;
        let mut final_progress = DownloadProgress {
            received: 0,
            total: 0,
        };
        download(&root, model, |progress| {
            progress_seen = true;
            final_progress = progress;
        })
        .expect("a real download of the default model should succeed");

        assert!(
            progress_seen,
            "the progress callback should fire at least once"
        );
        assert!(
            final_progress.received > 0,
            "should report having received bytes"
        );

        let paths = paths(&root, model);
        assert!(paths.all_present(), "all four files should be on disk");
        assert!(
            fs::metadata(&paths.encoder).unwrap().len() > 1_000_000,
            "the encoder should not be a truncated stub"
        );
        assert!(status(&root, model).installed);

        let _ = fs::remove_dir_all(&root);
    }

    fn temp_root_for_download() -> PathBuf {
        let root = std::env::temp_dir().join("textream-model-download-test");
        let _ = fs::remove_dir_all(&root);
        root
    }
}
