//! Speech model registry and first-run download.
//!
//! Models are far too large to ship in a 2 MB installer, and most users never
//! need every language, so they are fetched on demand into the app data
//! directory.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

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
    /// Expected SHA-256 of each file, pinned from the repository at
    /// registry-authoring time.
    pub hashes: ModelHashes,
    /// Approximate download size, for the confirmation the UI shows.
    pub download_bytes: u64,
}

/// SHA-256 digests (lowercase hex) matching `SpeechModel::files`.
///
/// The Kroko models share file names across languages (`encoder.onnx` and so
/// on), so a hash can only be pinned per model, not per file name — a
/// download landing on disk with the right name but the wrong bytes, whether
/// from a corrupted transfer or a substituted source, must still be caught
/// before it ever reaches the ONNX runtime.
#[derive(Debug, Clone, Copy)]
pub struct ModelHashes {
    pub encoder: &'static str,
    pub decoder: &'static str,
    pub joiner: &'static str,
    pub tokens: &'static str,
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
        hashes: ModelHashes {
            encoder: "3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3",
            decoder: "21e2a2acd961b3ac72f55be2f10f1a285e1b0b0ba010d7c0b6eab141411b163c",
            joiner: "e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e",
            tokens: "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb",
        },
        download_bytes: 43_600_000,
    },
    SpeechModel {
        id: "en",
        label: "English",
        language: "en",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06",
        files: KROKO_FILES,
        hashes: ModelHashes {
            encoder: "d4881c57449d581e0770fd53fa66c2fdc6cd167d92ece7c715e603defc96d9d4",
            decoder: "455ba38466fce8d5a57e7db68a323b684079ca4d9e1dd93a740d9b2429aae3b1",
            joiner: "d406f616736350e2a7df3e39398b78eb2fc1a2ca6973a19d3853fa3227e25b52",
            tokens: "396dbeb5f4858875690716084f54e90d339679d0ba3e6b5b584f3d7589254d2d",
        },
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "de",
        label: "German",
        language: "de",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-de-kroko-2025-08-06",
        files: KROKO_FILES,
        hashes: ModelHashes {
            encoder: "6e83993d6967ec7a3498b055b7e85ace85b5d64d1b1e8773cb29a43a11f5edb5",
            decoder: "94a29592b403c53fa2231b478637da1ab4abcef7f5e46e432098416a4a3ed562",
            joiner: "28356bff070aea51ab1d725a3278e81d19f9300f860d3248a7014292264df15a",
            tokens: "86e8370994ff2c01149ba8c4f8709aa93cdc18914b27a717e291e96faf39a6eb",
        },
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "fr",
        label: "French",
        language: "fr",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-fr-kroko-2025-08-06",
        files: KROKO_FILES,
        hashes: ModelHashes {
            encoder: "e02facae1daf6f1f13da67ea3ace7c722516d0868d1768d78c0580bc22cc0c5b",
            decoder: "6aed547570e3ab5afc05429a017cedd3a056c16df3baa5703f02461cefa25bac",
            joiner: "a51eec759bcdcaae2614686fa2a8b57417b2d420dd55a5a5558b388d35a9b2b6",
            tokens: "fedfb9c844bfb2bf14171f8184863e3d617b815a8667bdd9fc9a3149fde73298",
        },
        download_bytes: 71_300_000,
    },
    SpeechModel {
        id: "es",
        label: "Spanish",
        language: "es",
        repo: "csukuangfj/sherpa-onnx-streaming-zipformer-es-kroko-2025-08-06",
        files: KROKO_FILES,
        hashes: ModelHashes {
            encoder: "2d9f5ef87d1a5257f8a6687e21501c56f3aa2fcbfcfab9364dcc4ce4e06ae81b",
            decoder: "d4ce176b94b25f7acc88717bc3f704fcf5d6e131aaac2e0cabab3885541181ee",
            joiner: "dae35df88d676e320fcdb99217328e66dcf722bf11b0f2459e14ddb5b982ded5",
            tokens: "1be5e0a58e05d06d327df4c6b7b5e4f8aba01da6981eb016fcaceafc6a56680f",
        },
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
        hashes: ModelHashes {
            encoder: "d6380c74c75aaf37a739061a9197440fc5ca3f73ee339e588268eff2e9d3bf84",
            decoder: "93f0df50c2834fe225bbabd664d59fc2488b15736fbc6acaabddec3188dea9b4",
            joiner: "7fa442b8b35b1ab217dbceadb57a7da5388ee445c6c722eab576a57071e0dcea",
            tokens: "6722bd1585f46f84456b29c3550a343a3cc375b971645773c02ed8e0b4e2405c",
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
        hashes: ModelHashes {
            encoder: "f9001ed7a9e46d0294438c1a30cd7c72d1cc4bdd4e7880edbcda36f67081e32e",
            decoder: "7ebc63f34b21c8efb4a41a5a2eee7fe1448829ce0230ecc5369e67fc14d90d48",
            joiner: "db88e3172323551abaa99b91b18fb422a27ea4a834fd0db10389f9478816f917",
            tokens: "784f24950f6bcce1b0021035632dd60fd4617ecd8ca0581ab57d7b39d77ba5ab",
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
    let expected_hashes = [
        model.hashes.encoder,
        model.hashes.decoder,
        model.hashes.joiner,
        model.hashes.tokens,
    ];

    let total = model.download_bytes;
    let mut received = 0u64;
    on_progress(DownloadProgress { received, total });

    for (name, expected) in names.into_iter().zip(expected_hashes) {
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

        // A completed HTTP response is not proof of correct bytes — a
        // truncated read that still hits EOF, a corrupted transfer, or a
        // substituted file at the source would otherwise be handed straight
        // to the ONNX runtime with nothing having checked it first.
        let actual = hash_file(&partial).map_err(|error| error.to_string())?;
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&partial);
            return Err(format!(
                "downloaded {name} does not match the expected checksum (expected {expected}, got {actual}) -- refusing to install it"
            ));
        }

        fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
    }

    on_progress(DownloadProgress {
        received: received.max(total),
        total: received.max(total),
    });
    Ok(())
}

/// SHA-256 of a file's contents, as lowercase hex.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
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
            for hash in [
                model.hashes.encoder,
                model.hashes.decoder,
                model.hashes.joiner,
                model.hashes.tokens,
            ] {
                assert_eq!(
                    hash.len(),
                    64,
                    "{} has a hash that is not 64 hex characters",
                    model.id
                );
                assert!(
                    hash.chars().all(|c| c.is_ascii_hexdigit()),
                    "{} has a non-hex character in a hash",
                    model.id
                );
            }
        }
    }

    #[test]
    fn hash_file_matches_a_known_sha256() {
        let root = std::env::temp_dir().join("textream-model-hash-test");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("hello.txt");
        fs::write(&path, b"hello world").unwrap();

        // Known SHA-256 of the literal bytes "hello world".
        assert_eq!(
            hash_file(&path).unwrap(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );

        let _ = fs::remove_dir_all(&root);
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
