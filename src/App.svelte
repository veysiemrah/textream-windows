<script lang="ts">
  import { onDestroy } from "svelte";
  import * as api from "./lib/api";
  import type { Geometry, Mode, Placement } from "./lib/api";
  import { startMeter, type Meter } from "./lib/microphone";

  const SAMPLE = `Welcome back to the show. [smile at camera]

Today we are shipping something I have wanted for a long time.

[pause] Let me show you how it works.`;

  let script = $state(SAMPLE);
  let mode = $state<Mode>("classic");
  let wordsPerSecond = $state(2.0);
  let placement = $state<Placement>("topCenter");
  let width = $state(420);
  let height = $state(160);
  let hideFromCapture = $state(true);
  let clickThrough = $state(true);

  let running = $state(false);
  let status = $state("");
  let statusKind = $state<"info" | "warn">("info");
  let level = $state(0);
  let voiceActive = $state(false);
  let wordProgress = $state(0);

  let meter: Meter | null = null;
  let frame = 0;
  let lastFrameTime = 0;

  // Word Tracking needs a local speech recogniser, which is the next milestone.
  // Offering the mode before it exists would just look broken.
  const SPEECH_READY = false;

  const geometry = (): Geometry => ({
    placement,
    target: "followCursor",
    width,
    height,
  });

  const needsMicrophone = $derived(mode !== "classic");
  const wordCount = $derived(
    script.trim() === "" ? 0 : script.trim().split(/\s+/).length,
  );

  function say(message: string, kind: "info" | "warn" = "info") {
    status = message;
    statusKind = kind;
  }

  async function start() {
    if (script.trim() === "") {
      say("Nothing to read — paste a script first.", "warn");
      return;
    }

    await api.loadScript(script);
    await api.setMode(mode);
    await api.setSpeed(wordsPerSecond);

    if (needsMicrophone) {
      try {
        meter = await startMeter();
      } catch {
        say("Microphone unavailable. Switching to Classic.", "warn");
        mode = "classic";
        await api.setMode(mode);
      }
    }

    await api.showOverlay(geometry());
    await api.setClickThrough(clickThrough);
    const accepted = await api.setHideFromCapture(hideFromCapture);
    if (hideFromCapture && !accepted) {
      say(
        "This build of Windows cannot hide the overlay from screen capture (needs 10 2004 or newer).",
        "warn",
      );
    } else {
      say("");
    }

    await api.startSession();
    running = true;
    lastFrameTime = performance.now();
    frame = requestAnimationFrame(loop);
  }

  async function stop() {
    running = false;
    cancelAnimationFrame(frame);
    meter?.stop();
    meter = null;
    level = 0;
    voiceActive = false;
    await api.stopSession();
    await api.hideOverlay();
  }

  async function loop(now: number) {
    const delta = (now - lastFrameTime) / 1000;
    lastFrameTime = now;

    level = meter ? meter.level() : 0;
    // The gate is timestamped in seconds from the same monotonic clock the
    // hangover is measured against.
    const progress = await api.tick(delta, meter ? level : null, now / 1000);
    voiceActive = progress.voiceActive;
    wordProgress = progress.wordProgress;

    if (progress.finished) {
      say("Reached the end of the script.");
      await stop();
      return;
    }
    if (running) frame = requestAnimationFrame(loop);
  }

  async function pushGeometry() {
    if (running) await api.setOverlayGeometry(geometry());
  }

  async function pushSpeed() {
    await api.setSpeed(wordsPerSecond);
  }

  async function pushMode(next: Mode) {
    mode = next;
    if (running) await api.setMode(next);
  }

  onDestroy(() => {
    cancelAnimationFrame(frame);
    meter?.stop();
  });
</script>

<main>
  <header>
    <h1>Textream</h1>
    <span class="sub">Teleprompter for Windows</span>
  </header>

  <section class="editor">
    <textarea
      bind:value={script}
      spellcheck="false"
      placeholder="Paste your script. Put stage directions in [brackets] — they are shown but never waited for."
    ></textarea>
    <div class="meta">
      <span>{wordCount} words</span>
      <span>~{(wordCount / wordsPerSecond / 60).toFixed(1)} min at this pace</span
      >
    </div>
  </section>

  <section class="controls">
    <fieldset>
      <legend>Mode</legend>
      <div class="segmented">
        <button
          class:active={mode === "wordTracking"}
          disabled={!SPEECH_READY}
          title={SPEECH_READY
            ? "Highlights each word as you say it"
            : "Needs the on-device speech engine — next milestone"}
          onclick={() => pushMode("wordTracking")}
        >
          Word Tracking
          {#if !SPEECH_READY}<em>soon</em>{/if}
        </button>
        <button
          class:active={mode === "classic"}
          title="Scrolls at a constant speed. No microphone."
          onclick={() => pushMode("classic")}>Classic</button
        >
        <button
          class:active={mode === "voiceActivated"}
          title="Scrolls while you speak, pauses in silence."
          onclick={() => pushMode("voiceActivated")}>Voice-Activated</button
        >
      </div>
    </fieldset>

    <fieldset>
      <legend>Pace — {wordsPerSecond.toFixed(1)} words/s</legend>
      <input
        type="range"
        min="0.5"
        max="8"
        step="0.1"
        bind:value={wordsPerSecond}
        oninput={pushSpeed}
      />
    </fieldset>

    <fieldset>
      <legend>Placement</legend>
      <select bind:value={placement} onchange={pushGeometry}>
        <option value="topCenter">Top centre — near the webcam</option>
        <option value="floating">Floating window</option>
        <option value="fullscreen">Fullscreen on this display</option>
        <option value="transportStrip">Transport strip above taskbar</option>
      </select>
      <p class="hint">
        The prompter sits at the top of the screen because that is where your
        camera is. The taskbar strip carries controls only.
      </p>
    </fieldset>

    <fieldset class="split">
      <label>
        Width — {width}px
        <input
          type="range"
          min="280"
          max="500"
          bind:value={width}
          oninput={pushGeometry}
        />
      </label>
      <label>
        Height — {height}px
        <input
          type="range"
          min="100"
          max="400"
          bind:value={height}
          oninput={pushGeometry}
        />
      </label>
    </fieldset>

    <fieldset>
      <legend>Overlay behaviour</legend>
      <label class="check">
        <input type="checkbox" bind:checked={hideFromCapture} />
        Hide from screen share and recordings
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={clickThrough} />
        Let clicks pass through to the app behind
      </label>
    </fieldset>
  </section>

  <footer>
    <div class="transport">
      <button class="primary" onclick={running ? stop : start}>
        {running ? "Stop" : "Start"}
      </button>
      {#if running}
        <div class="live">
          <span class="dot" class:on={voiceActive}></span>
          <div class="wave"><span style="width:{Math.min(100, level * 400)}%"
            ></span></div>
          <span class="counter">word {Math.floor(wordProgress)}</span>
        </div>
      {/if}
    </div>
    {#if status}
      <p class="status" class:warn={statusKind === "warn"}>{status}</p>
    {/if}
  </footer>
</main>

<style>
  :global(:root) {
    color-scheme: dark;
  }
  :global(body) {
    margin: 0;
    background: #0d0f12;
    color: #e8eaed;
    font:
      14px/1.5 "Segoe UI Variable Text",
      "Segoe UI",
      system-ui,
      sans-serif;
  }

  main {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 24px 28px 20px;
    min-height: 100vh;
    box-sizing: border-box;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .sub {
    color: #8b9098;
    font-size: 13px;
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
    min-height: 180px;
  }
  textarea {
    flex: 1;
    resize: none;
    padding: 16px;
    border: 1px solid #24282e;
    border-radius: 10px;
    background: #14171b;
    color: inherit;
    font: inherit;
    line-height: 1.7;
  }
  textarea:focus {
    outline: 2px solid #4f8cff;
    outline-offset: -1px;
  }
  .meta {
    display: flex;
    gap: 16px;
    color: #8b9098;
    font-size: 12px;
  }

  .controls {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 16px 24px;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    min-width: 0;
  }
  legend {
    padding: 0 0 8px;
    color: #8b9098;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .split {
    display: flex;
    gap: 16px;
  }
  .split label {
    flex: 1;
    font-size: 12px;
    color: #8b9098;
  }

  .segmented {
    display: flex;
    gap: 4px;
    background: #14171b;
    border: 1px solid #24282e;
    border-radius: 8px;
    padding: 4px;
  }
  .segmented button {
    flex: 1;
    padding: 7px 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: #b6bbc2;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .segmented button:hover:not(:disabled) {
    background: #1d2127;
  }
  .segmented button.active {
    background: #2b64d9;
    color: #fff;
  }
  .segmented button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .segmented em {
    display: block;
    font-size: 9px;
    font-style: normal;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    opacity: 0.7;
  }

  input[type="range"] {
    width: 100%;
    accent-color: #4f8cff;
  }
  select {
    width: 100%;
    padding: 8px;
    border: 1px solid #24282e;
    border-radius: 8px;
    background: #14171b;
    color: inherit;
    font: inherit;
  }
  .hint {
    margin: 8px 0 0;
    color: #6e747c;
    font-size: 11px;
    line-height: 1.5;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
    font-size: 13px;
    cursor: pointer;
  }
  .check input {
    accent-color: #4f8cff;
  }

  footer {
    border-top: 1px solid #1d2127;
    padding-top: 16px;
  }
  .transport {
    display: flex;
    align-items: center;
    gap: 20px;
  }
  .primary {
    padding: 10px 28px;
    border: 0;
    border-radius: 8px;
    background: #2b64d9;
    color: #fff;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .primary:hover {
    background: #3a72e6;
  }

  .live {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #3a3f47;
    transition: background 120ms;
  }
  .dot.on {
    background: #33d64a;
  }
  .wave {
    width: 120px;
    height: 4px;
    border-radius: 2px;
    background: #1d2127;
    overflow: hidden;
  }
  .wave span {
    display: block;
    height: 100%;
    background: #4f8cff;
    transition: width 60ms linear;
  }
  .counter {
    color: #6e747c;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .status {
    margin: 12px 0 0;
    font-size: 12px;
    color: #8b9098;
  }
  .status.warn {
    color: #ff9e0a;
  }
</style>
