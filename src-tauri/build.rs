use std::path::{Path, PathBuf};

/// Native libraries the app has to ship alongside its executable.
const RUNTIME_LIBRARIES: &[&str] = &[
    "onnxruntime.dll",
    "onnxruntime_providers_shared.dll",
    "sherpa-onnx-c-api.dll",
    "sherpa-onnx-cxx-api.dll",
    "cargs.dll",
];

fn main() {
    stage_runtime_libraries();
    tauri_build::build()
}

/// Copies the sherpa-onnx runtime into a fixed folder for the bundler.
///
/// `sherpa-rs-sys` drops its DLLs straight into `target/<profile>/`, which is
/// the right place for `cargo run` but not something `tauri.conf.json` can
/// point at: the profile is not known to the config, and naming one directly
/// means a debug build tries to bundle release artefacts that a fresh clone has
/// never produced. Staging them under a stable path keeps the resource list
/// profile-agnostic.
fn stage_runtime_libraries() {
    let Some(profile_dir) = profile_directory() else {
        return;
    };
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime");
    if std::fs::create_dir_all(&destination).is_err() {
        return;
    }

    for name in RUNTIME_LIBRARIES {
        let source = profile_dir.join(name);
        let target = destination.join(name);
        println!("cargo:rerun-if-changed={}", source.display());

        if !source.is_file() {
            // The dependency's build script has not run yet on a cold build.
            // Leaving whatever is already staged in place is better than
            // failing, and the next build picks the libraries up.
            continue;
        }
        if up_to_date(&source, &target) {
            continue;
        }
        let _ = std::fs::copy(&source, &target);
    }
}

/// `target/<profile>`, derived from `OUT_DIR`
/// (`target/<profile>/build/<crate>-<hash>/out`).
fn profile_directory() -> Option<PathBuf> {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").ok()?);
    Some(out_dir.ancestors().nth(3)?.to_path_buf())
}

/// Skips copying when the destination is already current.
///
/// Overwriting a DLL that the previously built executable still has mapped
/// fails with a sharing violation, so an unnecessary copy is not merely wasted
/// work — it breaks the build.
fn up_to_date(source: &Path, target: &Path) -> bool {
    let (Ok(source), Ok(target)) = (source.metadata(), target.metadata()) else {
        return false;
    };
    if source.len() != target.len() {
        return false;
    }
    match (source.modified(), target.modified()) {
        (Ok(source), Ok(target)) => target >= source,
        _ => false,
    }
}
