<h1 align="center">Textream for Windows</h1>

<p align="center">
  <strong>A free Windows teleprompter with real-time word tracking, classic auto-scroll, and voice-activated scrolling.</strong>
</p>

<p align="center">
  Built for streamers, interviewers, presenters, and podcasters.
</p>

<p align="center">
  The Windows port of <a href="https://github.com/f/textream"><strong>Textream</strong></a>
  by <a href="https://fka.dev">Fatih Kadir Akın</a>
</p>

---

> **Status: early development.** All three guidance modes work. Speech
> recognition covers several languages but not Turkish yet — see
> [Languages](#languages).

## The original

[**Textream**](https://github.com/f/textream) is a free, open-source macOS
teleprompter by [Fatih Kadir Akın](https://fka.dev), from an original idea by
[Semih Kışlar](https://x.com/semihdev). It shows your script in a Dynamic
Island–style overlay at the top of the screen, highlights each word as you say
it using on-device speech recognition, and stays invisible to your audience.

**On a Mac? Use the original — it is the more complete app.**
[github.com/f/textream](https://github.com/f/textream)

This project exists because that app is macOS-only, and it is built with the
original author's blessing to use the Textream name and icon.

### Why a separate repository and not a fork

There is no shared code to fork. The macOS app is SwiftUI and AppKit throughout:
the notch overlay, the settings UI, the text layout, and the speech engine are
all Apple frameworks, and none of it compiles anywhere else. A fork would carry
a git history no commit here ever touches, while GitHub hides forks from
repository search — costing discoverability for nothing in return.

What carries over is the product and the algorithms, reimplemented in Rust.

### What is the same, and what is not

| | macOS Textream | Textream for Windows |
|---|---|---|
| Guidance modes | Word Tracking, Classic, Voice-Activated | same three |
| Cue syntax | `[stage directions]` shown but never spoken | same |
| Privacy | fully on-device | same, deliberately |
| Overlay home | Dynamic Island under the notch | pill on the top edge — see below |
| Speech engine | Apple's recogniser, dozens of languages | sherpa-onnx, a handful of downloadable models |
| Extras | Sidecar, PowerPoint import, remote view, mirror output | out of scope — see below |

## Design decisions

**The overlay sits at the top of the screen, not on the taskbar.** The whole
point of the macOS notch overlay is that your eyes stay near the webcam. On
Windows the camera is above the monitor and the taskbar is pinned to the bottom,
so a bottom-docked prompter would have you reading off your own lap. The taskbar
strip is reserved for transport controls, and the app lives in the system tray.

**Everything stays on the machine.** Speech recognition runs locally. There is no
account, no telemetry, and no cloud speech API — matching the original's promise
rather than quietly trading it away for better accuracy.

**Word tracking is an alignment problem, not a transcription problem.** The
script is already known, so the job is finding how far into it you have got. A
modest on-device recogniser plus a forgiving matcher beats a large model, and it
runs comfortably alongside OBS.

## Architecture

```
crates/prompt-core/     Platform-neutral engine — no OS calls, no audio device
├── text.rs             CJK-aware tokenisation, RTL direction inference
├── alignment.rs        [cue] handling and the commit policy
├── script.rs           Parsed script: words, ranges, progress conversions
├── matcher.rs          Character- + word-level speech alignment
├── vad.rs              Level metering and the hysteretic speech gate
└── scroll.rs           Constant-pace scrolling for Classic / Voice-Activated

src-tauri/              Windows shell
├── window_effects.rs   NOACTIVATE, click-through, exclude-from-capture
├── overlay.rs          Placement and multi-monitor geometry
├── audio.rs            Microphone capture and the speech worker
├── speech.rs           Streaming recogniser over the sherpa-onnx C API
├── model.rs            Model registry and first-run download
├── document.rs         .textream file format — read/write, macOS-compatible
├── backdrop.rs         Mica or blur behind the frameless window
├── settings.rs         Persisted preferences
├── shortcuts.rs        Global shortcuts for hands-free control
├── diagnostics.rs      Crash and background-error logging
├── session.rs          Live session state and the webview-facing DTOs
└── lib.rs              Commands, events, tray icon

src/                    Svelte UI
├── App.svelte          Editor, settings panel and transport dock
├── Chrome.svelte       Title bar and resize grips for the frameless window
├── LiquidStart.svelte  The voice-reactive transport control
├── Overlay.svelte      The prompter pill
└── lib/                Typed command wrappers, spring integrator, file dialogs, toasts
```

The engine deliberately knows nothing about Windows. That keeps it testable
without a microphone and without launching the app.

### Windows specifics

| Concern | Mechanism |
|---|---|
| Never steals focus | `WS_EX_NOACTIVATE`, applied once at startup |
| Out of Alt-Tab | `WS_EX_TOOLWINDOW` |
| Clicks pass through to OBS | `WS_EX_TRANSPARENT` |
| Invisible to screen share | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, needs Windows 10 2004+ |
| Sits above the taskbar | Plain always-on-top window, **not** a registered AppBar — an AppBar shrinks every maximised window and leaves the work area wrong if the app dies |
| Never runs twice | `tauri-plugin-single-instance`, registered first — a second launch just refocuses the first |
| Closing tucks the editor away | `CloseRequested` intercepted on the main window only; the overlay and any running session are untouched |

### The three guidance modes

| Mode | Driven by | Microphone |
|---|---|---|
| **Word Tracking** | `PromptMatcher` fed transcript windows | Required |
| **Classic** | `PaceScroller`, gate held open | Not needed |
| **Voice-Activated** | `PaceScroller` gated by `VoiceActivityDetector` | Required |

## Speech recognition

Word Tracking runs a streaming Zipformer transducer through sherpa-onnx. The
model is downloaded on first use into the app data directory and never ships in
the installer.

`sherpa-rs` only wraps sherpa-onnx's *offline* recogniser, which decodes a
finished buffer. A teleprompter cannot wait for the presenter to stop talking,
so [`speech.rs`](src-tauri/src/speech.rs) calls the online C API directly.

Audio capture lives in Rust rather than in the webview. Recognition needs raw
PCM, and opening the same input device twice — once for WebAudio metering, once
for the recogniser — fails on exclusive-mode hardware. One capture path feeds
both the level meter and the transcriber.

### Languages

The macOS app asks the operating system which languages it can transcribe and
offers those, which on a Mac is dozens — Apple ships them. Windows cannot be
asked the same question usefully: its own recogniser covers English, French,
German, Japanese, Mandarin and Spanish, and nothing else. Copying the
architecture would mean *fewer* languages, not more.

So the model registry plays that role instead, and a language is available
exactly when somebody has published a streaming model for it:

| | Size |
|---|---|
| English (small) — fastest, the default | 42 MB |
| English, German, French | 71 MB each |
| Spanish | 156 MB |
| Chinese (Mandarin) | 77 MB |
| Arabic, Indonesian, Japanese, Russian, Thai, Vietnamese (one model) | 339 MB |

**Turkish is not available yet.** A model exists — Kroko publishes a Turkish
community streaming model — but only in their own packaging, and nobody has
converted it to the ONNX layout sherpa-onnx loads. That conversion is the whole
job; the app already handles everything after it.

Adding a language is a few lines in [`model.rs`](src-tauri/src/model.rs) and a
published model to point at.

## Script files

`.textream` files are the same format the macOS app writes: a JSON array of
page strings, nothing else. A script saved on one platform opens cleanly on
the other — the format was kept exactly as-is rather than inventing a Windows
one, since there was no reason to diverge from something this simple.

Multi-page scripts are out of scope here (see the roadmap), so opening a file
with more than one page flattens them into the single script this app edits,
each former page separated by a blank line. Nothing is dropped — a script
written on a Mac with several pages still opens here in full, just without the
page boundaries.

The script itself is always autosaved as part of your settings, so `.textream`
files are for moving a script between machines or sharing it with someone
else, not the only copy of anything.

## Building

Requires:

- Rust 1.80+ with the MSVC toolchain
- Node 20+
- WebView2 (preinstalled on Windows 11)
- **LLVM** — `sherpa-rs-sys` generates its bindings with bindgen, which needs
  `libclang.dll`

```bash
winget install LLVM.LLVM
```

The default install location is enough — bindgen finds `libclang.dll` there on
its own, with no environment variable. If yours lives elsewhere, point
`LIBCLANG_PATH` at the directory containing it.

```bash
npm install
```

Run the app in development:

```bash
npm run app
```

Build the installer:

```bash
npm run app:build
```

Run the test suite:

```bash
cargo test --workspace
```

## Roadmap

- [x] `prompt-core` — engine, cue handling, matcher, VAD, pacing
- [x] Tauri shell: top-centre pill, floating, fullscreen, transport strip
- [x] Hide-from-capture, click-through, non-activating overlay
- [x] System tray
- [x] Classic and Voice-Activated modes end to end
- [x] On-device streaming speech recognition — Word Tracking works
- [x] A model registry covering several languages
- [ ] Turkish, once a streaming model is converted for it
- [x] Settings that persist across launches
- [x] Font, size, colour and opacity
- [x] Pause, hold, and mute from the transport dock
- [x] Tap a word to jump; scroll to catch up
- [x] `.textream` files — the same format the macOS app writes
- [x] Global shortcuts — start, hold and mute without leaving the camera
- [x] First-run welcome, and error messages that say what to do
- [x] Closes to the tray instead of quitting; a second launch refocuses it
- [x] A toast when a shortcut-triggered start fails with the window hidden
- [x] A crash log, since a release build has no console to catch one
- [ ] Update check

### Deliberately out of scope

The macOS app has years of features this port is not chasing. Skipped on
purpose, not forgotten:

- **Remote connection** — the browser view with QR pairing. If you want your
  script on a phone, the original does it well.
- **PowerPoint notes import**, multi-page scripts, mirror output for prompter
  rigs, and Sidecar-style external display modes.

The goal here is a complete, solid teleprompter on Windows, not feature parity.

## Credits

Textream is [Fatih Kadir Akın](https://fka.dev)'s work, from an original idea by
[Semih Kışlar](https://x.com/semihdev). This port exists because that app is
good enough to want on another platform, and it carries the Textream name and
icon with his blessing. The cue syntax, the three guidance modes, and the
approach to word tracking are all his design.

- Original macOS app: [github.com/f/textream](https://github.com/f/textream)
- Windows port: [CRTkafa](https://github.com/CRTkafa)

Speech recognition uses [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) by
the k2-fsa team, with streaming models published by
[csukuangfj](https://huggingface.co/csukuangfj).

The dyslexia-friendly typeface is
[OpenDyslexic](https://opendyslexic.org) by Abbie Gonzalez, bundled under the
SIL Open Font License — see
[the licence](src/assets/fonts/OpenDyslexic-OFL.txt).

## License

MIT, matching the original. Portions of the script-alignment, cue-handling and
voice-activity logic are reimplementations of algorithms from
[f/textream](https://github.com/f/textream); see [LICENSE](LICENSE).
