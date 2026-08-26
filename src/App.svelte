<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import * as api from "./lib/api";
  import type {
    ColorPreset,
    DownloadProgress,
    FontFamily,
    FontSize,
    Geometry,
    ModelStatus,
    Mode,
    Settings,
  } from "./lib/api";

  const SAMPLE = `Welcome back to the show. [smile at camera]

Today we are shipping something I have wanted for a long time.

[pause] Let me show you how it works.`;

  /** Mirrors `Settings::default()` in Rust, for the frames before load. */
  const FALLBACK: Settings = {
    mode: "classic",
    wordsPerSecond: 2.0,
    placement: "topCenter",
    target: "followCursor",
    width: 420,
    height: 160,
    hideFromCapture: true,
    clickThrough: true,
    appearance: {
      fontFamily: "sans",
      fontSize: "lg",
      highlight: "yellow",
      cue: "orange",
      opacity: 0.92,
    },
    modelId: null,
    script: "",
  };

  const FONTS: { id: FontFamily; label: string }[] = [
    { id: "sans", label: "Sans" },
    { id: "serif", label: "Serif" },
    { id: "mono", label: "Mono" },
    { id: "dyslexic", label: "Dyslexic" },
  ];
  const SIZES: FontSize[] = ["xs", "sm", "lg", "xl"];
  const COLORS: { id: ColorPreset; css: string }[] = [
    { id: "white", css: "#ffffff" },
    { id: "yellow", css: "#ffd60a" },
    { id: "green", css: "#33d64a" },
    { id: "blue", css: "#4f8cff" },
    { id: "pink", css: "#ff6191" },
    { id: "orange", css: "#ff9e0a" },
  ];

  let settings = $state<Settings>({ ...FALLBACK, script: SAMPLE });
  let loaded = $state(false);

  let running = $state(false);
  let paused = $state(false);
  let muted = $state(false);
  let status = $state("");
  let statusKind = $state<"info" | "warn">("info");
  let level = $state(0);
  let voiceActive = $state(false);
  let wordProgress = $state(0);

  let models = $state<ModelStatus[]>([]);
  let downloading = $state(false);
  let downloadPercent = $state(0);

  let frame = 0;
  let lastFrameTime = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  const selectedModel = $derived(
    models.find((candidate) => candidate.id === settings.modelId) ??
      models[0] ??
      null,
  );
  const speechReady = $derived(selectedModel?.installed === true);
  const needsMicrophone = $derived(settings.mode !== "classic");
  const wordCount = $derived(
    settings.script.trim() === ""
      ? 0
      : settings.script.trim().split(/\s+/).length,
  );

  const geometry = (): Geometry => ({
    placement: settings.placement,
    target: settings.target,
    width: settings.width,
    height: settings.height,
  });

  function say(message: string, kind: "info" | "warn" = "info") {
    status = message;
    statusKind = kind;
  }

  const megabytes = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

  /**
   * Writes settings after a short pause.
   *
   * Every slider drag fires an input event, and each one would otherwise be a
   * disk write. Debouncing also means the value that lands is the one the user
   * settled on, not the last frame of the drag.
   */
  function persist() {
    if (!loaded) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      void api.saveSettings($state.snapshot(settings));
    }, 400);
  }

  onMount(() => {
    void restore();
    const unlisten = listen<DownloadProgress>(api.EVENT_DOWNLOAD, (event) => {
      const { received, total } = event.payload;
      downloadPercent = total > 0 ? (received / total) * 100 : 0;
    });
    return () => {
      void unlisten.then((off) => off());
    };
  });

  async function restore() {
    try {
      const stored = await api.loadSettings();
      // A first run has no script; the sample is more use than a blank page.
      settings = { ...stored, script: stored.script || SAMPLE };
      models = await api.speechModels();
      settings.modelId ??= models[0]?.id ?? null;
    } catch (error) {
      say(`Could not read your settings: ${error}`, "warn");
    } finally {
      loaded = true;
    }
  }

  async function downloadModel() {
    if (!selectedModel || downloading) return;
    downloading = true;
    downloadPercent = 0;
    say(`Downloading ${selectedModel.label}…`);
    try {
      const updated = await api.downloadSpeechModel(selectedModel.id);
      models = models.map((m) => (m.id === updated.id ? updated : m));
      say(`${updated.label} is ready. Word Tracking is available.`);
    } catch (error) {
      say(`Download failed: ${error}`, "warn");
    } finally {
      downloading = false;
    }
  }

  async function start() {
    if (settings.script.trim() === "") {
      say("Nothing to read — paste a script first.", "warn");
      return;
    }

    await api.loadScript(settings.script);
    await api.setMode(settings.mode);
    await api.setSpeed(settings.wordsPerSecond);

    // Arm the session before showing anything: it opens the microphone and
    // loads the model, either of which can fail, and an overlay that appears
    // and then never moves is worse than one that never appeared.
    try {
      await api.startSession(
        settings.mode === "wordTracking" ? settings.modelId : null,
      );
    } catch (error) {
      say(String(error), "warn");
      return;
    }

    await api.showOverlay(geometry());
    await api.setClickThrough(settings.clickThrough);
    const accepted = await api.setHideFromCapture(settings.hideFromCapture);
    if (settings.hideFromCapture && !accepted) {
      say(
        "This build of Windows cannot hide the overlay from screen capture (needs 10 2004 or newer).",
        "warn",
      );
    } else {
      say("");
    }

    running = true;
    lastFrameTime = performance.now();
    frame = requestAnimationFrame(loop);
  }

  async function stop() {
    running = false;
    paused = false;
    muted = false;
    cancelAnimationFrame(frame);
    level = 0;
    voiceActive = false;
    await api.stopSession();
    await api.hideOverlay();
  }

  async function togglePause() {
    paused = (await api.setPaused(!paused)).paused;
    say(paused ? "Paused. The microphone is still open." : "");
  }

  async function toggleMute() {
    muted = !muted;
    await api.setMicrophoneMuted(muted);
  }

  async function loop(now: number) {
    const delta = (now - lastFrameTime) / 1000;
    lastFrameTime = now;

    const progress = await api.tick(delta);
    level = progress.level;
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
    persist();
    if (running) await api.setOverlayGeometry(geometry());
  }

  async function pushSpeed() {
    persist();
    await api.setSpeed(settings.wordsPerSecond);
  }

  async function pushMode(next: Mode) {
    settings.mode = next;
    persist();
    if (running) await api.setMode(next);
  }

  async function pushClickThrough() {
    persist();
    if (running) await api.setClickThrough(settings.clickThrough);
  }

  async function pushHideFromCapture() {
    persist();
    if (running) await api.setHideFromCapture(settings.hideFromCapture);
  }

  onDestroy(() => {
    cancelAnimationFrame(frame);
    clearTimeout(saveTimer);
  });
</script>

<main>
  <header>
    <h1>Textream</h1>
    <span class="sub">Teleprompter for Windows</span>
  </header>

  <section class="editor">
    <textarea
      bind:value={settings.script}
      oninput={persist}
      spellcheck="false"
      placeholder="Paste your script. Put stage directions in [brackets] — they are shown but never waited for."
    ></textarea>
    <div class="meta">
      <span>{wordCount} words</span>
      <span
        >~{(wordCount / settings.wordsPerSecond / 60).toFixed(1)} min at this pace</span
      >
    </div>
  </section>

  <section class="controls">
    <fieldset>
      <legend>Mode</legend>
      <div class="segmented">
        <button
          class:active={settings.mode === "wordTracking"}
          disabled={!speechReady}
          title={speechReady
            ? "Highlights each word as you say it"
            : "Download the speech model to enable this"}
          onclick={() => pushMode("wordTracking")}
        >
          Word Tracking
          {#if !speechReady}<em>needs model</em>{/if}
        </button>
        <button
          class:active={settings.mode === "classic"}
          title="Scrolls at a constant speed. No microphone."
          onclick={() => pushMode("classic")}>Classic</button
        >
        <button
          class:active={settings.mode === "voiceActivated"}
          title="Scrolls while you speak, pauses in silence."
          onclick={() => pushMode("voiceActivated")}>Voice-Activated</button
        >
      </div>
    </fieldset>

    <fieldset>
      <legend>Speech model</legend>
      {#if selectedModel}
        <div class="model">
          <div class="model-name">
            <strong>{selectedModel.label}</strong>
            <span class="tag"
              >{selectedModel.installed
                ? "installed"
                : megabytes(selectedModel.downloadBytes)}</span
            >
          </div>
          {#if !selectedModel.installed}
            <button class="ghost" disabled={downloading} onclick={downloadModel}>
              {downloading ? `${downloadPercent.toFixed(0)}%` : "Download"}
            </button>
          {/if}
        </div>
        {#if downloading}
          <div class="bar"><span style="width:{downloadPercent}%"></span></div>
        {/if}
      {:else}
        <p class="hint">No speech models are registered.</p>
      {/if}
      <p class="hint">
        Recognition runs entirely on this machine. Nothing is uploaded, and no
        account is needed.
      </p>
    </fieldset>

    <fieldset>
      <legend>Pace — {settings.wordsPerSecond.toFixed(1)} words/s</legend>
      <input
        type="range"
        min="0.5"
        max="8"
        step="0.1"
        bind:value={settings.wordsPerSecond}
        oninput={pushSpeed}
        disabled={settings.mode === "wordTracking"}
      />
      <p class="hint">
        {settings.mode === "wordTracking"
          ? "Word Tracking follows your voice, so pace is not used."
          : "How fast the script advances."}
      </p>
    </fieldset>

    <fieldset>
      <legend>Placement</legend>
      <select bind:value={settings.placement} onchange={pushGeometry}>
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
        Width — {settings.width}px
        <input
          type="range"
          min="280"
          max="500"
          bind:value={settings.width}
          oninput={pushGeometry}
        />
      </label>
      <label>
        Height — {settings.height}px
        <input
          type="range"
          min="100"
          max="400"
          bind:value={settings.height}
          oninput={pushGeometry}
        />
      </label>
    </fieldset>

    <fieldset>
      <legend>Typeface</legend>
      <div class="segmented">
        {#each FONTS as font (font.id)}
          <button
            class:active={settings.appearance.fontFamily === font.id}
            onclick={() => {
              settings.appearance.fontFamily = font.id;
              persist();
            }}>{font.label}</button
          >
        {/each}
      </div>
      <div class="segmented sizes">
        {#each SIZES as size (size)}
          <button
            class:active={settings.appearance.fontSize === size}
            onclick={() => {
              settings.appearance.fontSize = size;
              persist();
            }}>{size.toUpperCase()}</button
          >
        {/each}
      </div>
    </fieldset>

    <fieldset>
      <legend>Colour</legend>
      <div class="swatch-row">
        <span class="swatch-label">Spoken word</span>
        {#each COLORS as color (color.id)}
          <button
            class="swatch"
            class:active={settings.appearance.highlight === color.id}
            style="background:{color.css}"
            title={color.id}
            aria-label="Highlight {color.id}"
            onclick={() => {
              settings.appearance.highlight = color.id;
              persist();
            }}
          ></button>
        {/each}
      </div>
      <div class="swatch-row">
        <span class="swatch-label">Cues</span>
        {#each COLORS as color (color.id)}
          <button
            class="swatch"
            class:active={settings.appearance.cue === color.id}
            style="background:{color.css}"
            title={color.id}
            aria-label="Cue {color.id}"
            onclick={() => {
              settings.appearance.cue = color.id;
              persist();
            }}
          ></button>
        {/each}
      </div>
    </fieldset>

    <fieldset>
      <legend>
        Background — {(settings.appearance.opacity * 100).toFixed(0)}%
      </legend>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        bind:value={settings.appearance.opacity}
        oninput={persist}
      />
      <p class="hint">
        Lower is more see-through. Fully clear leaves only the text over your
        screen.
      </p>
    </fieldset>

    <fieldset>
      <legend>Overlay behaviour</legend>
      <label class="check">
        <input
          type="checkbox"
          bind:checked={settings.hideFromCapture}
          onchange={pushHideFromCapture}
        />
        Hide from screen share and recordings
      </label>
      <label class="check">
        <input
          type="checkbox"
          bind:checked={settings.clickThrough}
          onchange={pushClickThrough}
        />
        Let clicks pass through to the app behind
      </label>
      <p class="hint">
        Clicks cannot both pass through and land on the overlay. Turn this off
        to click a word and jump the prompter there.
      </p>
    </fieldset>
  </section>

  <footer>
    <div class="transport">
      <button class="primary" onclick={running ? stop : start}>
        {running ? "Stop" : "Start"}
      </button>
      {#if running}
        <button class="ghost" onclick={togglePause}>
          {paused ? "Resume" : "Pause"}
        </button>
        {#if needsMicrophone}
          <button class="ghost" class:muted onclick={toggleMute}>
            {muted ? "Unmute" : "Mute"}
          </button>
        {/if}
        <div class="live">
          {#if needsMicrophone}
            <span class="dot" class:on={voiceActive}></span>
            <div class="wave">
              <span style="width:{Math.min(100, level * 400)}%"></span>
            </div>
          {/if}
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
    min-height: 150px;
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
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
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
  .segmented.sizes {
    margin-top: 6px;
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

  .swatch-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 0;
  }
  .swatch-label {
    width: 88px;
    color: #8b9098;
    font-size: 12px;
  }
  .swatch {
    width: 20px;
    height: 20px;
    border: 2px solid transparent;
    border-radius: 50%;
    padding: 0;
    cursor: pointer;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.35);
  }
  .swatch.active {
    border-color: #e8eaed;
  }

  .model {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid #24282e;
    border-radius: 8px;
    background: #14171b;
  }
  .model-name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .tag {
    color: #6e747c;
    font-size: 11px;
  }
  .ghost {
    padding: 6px 14px;
    border: 1px solid #2f3540;
    border-radius: 6px;
    background: transparent;
    color: #e8eaed;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    min-width: 84px;
  }
  .ghost:hover:not(:disabled) {
    background: #1d2127;
  }
  .ghost:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .ghost.muted {
    border-color: #7a4a12;
    color: #ff9e0a;
  }
  .bar {
    height: 3px;
    margin-top: 8px;
    border-radius: 2px;
    background: #1d2127;
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: #4f8cff;
    transition: width 200ms linear;
  }

  input[type="range"] {
    width: 100%;
    accent-color: #4f8cff;
  }
  input[type="range"]:disabled {
    opacity: 0.4;
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
