<h1 align="center">Textream for Windows</h1>

<p align="center">
  <strong>A free Windows teleprompter with real-time word tracking, classic auto-scroll, and voice-activated scrolling.</strong>
</p>

<p align="center">
  Built for streamers, interviewers, presenters, and podcasters.
</p>

---

> **Status: early development.** The engine, the app shell and the overlay work.
> Classic and Voice-Activated modes run today; Word Tracking is waiting on the
> on-device speech engine.

This is the Windows counterpart to [Textream](https://github.com/f/textream), the
macOS teleprompter by [Fatih Kadir Akın](https://fka.dev). It is a ground-up
implementation rather than a fork — the macOS app is SwiftUI and AppKit through
and through, so nothing ports across as code. What carries over is the product
and the algorithms, with the original author's blessing to use the name and icon.

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
├── session.rs          Live session state and the webview-facing DTOs
└── lib.rs              Commands, events, tray icon

src/                    Svelte UI
├── App.svelte          Editor and controls
├── Overlay.svelte      The prompter pill
└── lib/                Typed command wrappers, microphone metering
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

### The three guidance modes

| Mode | Driven by | Microphone |
|---|---|---|
| **Word Tracking** | `PromptMatcher` fed transcript windows | Required |
| **Classic** | `PaceScroller`, gate held open | Not needed |
| **Voice-Activated** | `PaceScroller` gated by `VoiceActivityDetector` | Required |

## Building

Requires Rust 1.80+ with the MSVC toolchain, Node 20+, and WebView2 (preinstalled
on Windows 11).

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
- [ ] On-device streaming speech recognition → unlocks Word Tracking
- [ ] Tap a word to jump; scroll to catch up
- [ ] Remote connection: local HTTP + WebSocket view with QR pairing
- [ ] `.textream` files, PowerPoint notes import, multi-page scripts
- [ ] Font, size and colour settings; mirror output for prompter rigs
- [ ] Global shortcuts

## Credits

Original idea by [Semih Kışlar](https://x.com/semihdev).
macOS Textream by [Fatih Kadir Akın](https://fka.dev).
Windows version by [CRTkafa](https://github.com/CRTkafa).

## License

MIT
