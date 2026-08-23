<h1 align="center">Textream for Windows</h1>

<p align="center">
  <strong>A free Windows teleprompter with real-time word tracking, classic auto-scroll, and voice-activated scrolling.</strong>
</p>

<p align="center">
  Built for streamers, interviewers, presenters, and podcasters.
</p>

---

> **Status: early development.** Nothing is shippable yet. The engine is done and
> tested; the Windows shell is next.

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
```

The engine deliberately knows nothing about Windows. That keeps it testable
without a microphone, and it means a Linux build is mostly a shell away.

### The three guidance modes

| Mode | Driven by | Microphone |
|---|---|---|
| **Word Tracking** | `PromptMatcher` fed transcript windows | Required |
| **Classic** | `PaceScroller`, gate held open | Not needed |
| **Voice-Activated** | `PaceScroller` gated by `VoiceActivityDetector` | Required |

## Building

Requires Rust 1.80+ and the MSVC toolchain.

```bash
cargo test --workspace
```

## Roadmap

- [x] `prompt-core` — engine, cue handling, matcher, VAD, pacing
- [ ] Tauri shell: top-centre overlay, floating window, fullscreen
- [ ] On-device streaming speech recognition
- [ ] Hide-from-capture, click-through, non-activating overlay windows
- [ ] Remote connection: local HTTP + WebSocket view with QR pairing
- [ ] Script editor, `.textream` files, PowerPoint notes import
- [ ] System tray, transport strip, global shortcuts

## Credits

Original idea by [Semih Kışlar](https://x.com/semihdev).
macOS Textream by [Fatih Kadir Akın](https://fka.dev).
Windows version by [CRTkafa](https://github.com/CRTkafa).

## License

MIT
