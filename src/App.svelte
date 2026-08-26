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
  import Chrome from "./Chrome.svelte";
  import LiquidStart from "./LiquidStart.svelte";

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

  const MODES: { id: Mode; label: string; blurb: string }[] = [
    {
      id: "wordTracking",
      label: "Follow",
      blurb: "Highlights each word as you say it.",
    },
    { id: "classic", label: "Auto", blurb: "Scrolls at a constant speed." },
    {
      id: "voiceActivated",
      label: "Voice",
      blurb: "Scrolls while you speak, holds in silence.",
    },
  ];

  const FONTS: { id: FontFamily; label: string }[] = [
    { id: "sans", label: "Sans" },
    { id: "serif", label: "Serif" },
    { id: "mono", label: "Mono" },
    { id: "dyslexic", label: "Dyslexic" },
  ];
  const SIZES: FontSize[] = ["xs", "sm", "lg", "xl"];
  const COLORS: Record<ColorPreset, string> = {
    white: "#ffffff",
    yellow: "#ffd60a",
    green: "#33d64a",
    blue: "#4f8cff",
    pink: "#ff6191",
    orange: "#ff9e0a",
  };
  const COLOR_IDS = Object.keys(COLORS) as ColorPreset[];

  let settings = $state<Settings>({ ...FALLBACK, script: SAMPLE });
  let loaded = $state(false);
  let firstRun = $state(false);
  let backdrop = $state<"mica" | "blur" | "none">("none");
  let panelOpen = $state(false);

  let running = $state(false);
  let transitioning = $state(false);
  let paused = $state(false);
  let muted = $state(false);
  let status = $state("");
  let statusKind = $state<"info" | "warn">("info");
  let level = $state(0);
  let voiceActive = $state(false);
  let wordProgress = $state(0);

  let models = $state<ModelStatus[]>([]);
  let bindings = $state<[string, string][]>([]);
  let diagnostics = $state<api.SpeechDiagnostics | null>(null);
  let downloading = $state(false);
  let downloadPercent = $state(0);

  let frame = 0;
  let lastFrameTime = 0;
  let lastDiagnostics = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  const selectedModel = $derived(
    models.find((candidate) => candidate.id === settings.modelId) ??
      models[0] ??
      null,
  );
  const speechReady = $derived(selectedModel?.installed === true);
  const needsMicrophone = $derived(settings.mode !== "classic");
  const modeIndex = $derived(
    Math.max(
      0,
      MODES.findIndex((mode) => mode.id === settings.mode),
    ),
  );
  const wordCount = $derived(
    settings.script.trim() === ""
      ? 0
      : settings.script.trim().split(/\s+/).length,
  );
  const minutes = $derived(wordCount / settings.wordsPerSecond / 60);
  const accent = $derived(COLORS[settings.appearance.highlight]);
  const toggleKeys = $derived(
    bindings.find(([name]) => name === "Start or stop")?.[1] ?? null,
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
    const unlisten = Promise.all([
      listen<DownloadProgress>(api.EVENT_DOWNLOAD, (event) => {
        const { received, total } = event.payload;
        downloadPercent = total > 0 ? (received / total) * 100 : 0;
      }),
      // Shortcuts route through the same handlers the buttons use, so a
      // hands-free start behaves exactly like a clicked one.
      listen<api.ShortcutAction>(api.EVENT_SHORTCUT, (event) => {
        void onShortcut(event.payload);
      }),
    ]);
    return () => {
      void unlisten.then((offs) => offs.forEach((off) => off()));
    };
  });

  async function onShortcut(action: api.ShortcutAction) {
    if (action === "toggle") return toggleRun();
    // Hold and mute mean nothing when nothing is running.
    if (!running) return;
    if (action === "hold") return togglePause();
    if (action === "mute" && needsMicrophone) return toggleMute();
  }

  async function restore() {
    try {
      backdrop = await api.windowBackdrop();
      // Checked before loadSettings writes anything, and loadSettings never
      // does — so this stays accurate for as long as it takes to read it.
      firstRun = await api.isFirstRun();
      const stored = await api.loadSettings();
      // A first run has no script; the sample is more use than a blank page.
      settings = { ...stored, script: stored.script || SAMPLE };
      models = await api.speechModels();
      bindings = await api.shortcutBindings();
      settings.modelId ??= models[0]?.id ?? null;
    } catch (error) {
      say(`Could not read your settings: ${error}`, "warn");
    } finally {
      loaded = true;
    }
  }

  /**
   * Dismisses the welcome banner and forces an immediate settings save.
   *
   * Without the forced save, a user who dismisses the banner and then starts
   * a take straight away — never touching a slider or the script — would see
   * it again on the next launch, because nothing would have written
   * settings.json yet.
   */
  async function dismissFirstRun() {
    if (!firstRun) return;
    firstRun = false;
    clearTimeout(saveTimer);
    try {
      await api.saveSettings($state.snapshot(settings));
    } catch {
      // Losing this one write only means the banner might reappear once;
      // not worth surfacing as an error.
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
      say(`${updated.label} is ready. Follow mode is available.`);
    } catch (error) {
      say(`Download failed: ${error}`, "warn");
    } finally {
      downloading = false;
    }
  }

  async function removeModel() {
    if (!selectedModel || downloading || running) return;
    try {
      const updated = await api.removeSpeechModel(selectedModel.id);
      models = models.map((m) => (m.id === updated.id ? updated : m));
      // Follow mode needs a model; without one it would start and never move.
      if (settings.mode === "wordTracking") await pushMode("classic");
      say(`${updated.label} removed.`);
    } catch (error) {
      say(`Could not remove the model: ${error}`, "warn");
    }
  }

  async function toggleRun() {
    // start() and stop() both open or release a microphone and a model, which
    // take real time to settle. Without this guard, a second click before the
    // first finishes would race two attempts against the same device — the
    // button is also disabled while this is true, but a guard here covers
    // activation via the keyboard's repeat-on-hold too.
    if (transitioning) return;
    transitioning = true;
    try {
      if (running) await stop();
      else await start();
    } finally {
      transitioning = false;
    }
  }

  async function start() {
    if (settings.script.trim() === "") {
      say("Nothing to read — paste a script first.", "warn");
      return;
    }

    void dismissFirstRun();
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
        "This build of Windows cannot hide the overlay from capture. It needs Windows 10 2004 or newer.",
        "warn",
      );
    } else {
      say("");
    }

    panelOpen = false;
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
    diagnostics = null;
    await api.stopSession();
    await api.hideOverlay();
  }

  async function togglePause() {
    paused = (await api.setPaused(!paused)).paused;
    say(paused ? "Held. The microphone is still open." : "");
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

    // Follow mode is the only one that transcribes, and a second-by-second
    // poll is plenty for numbers a person reads.
    if (settings.mode === "wordTracking" && now - lastDiagnostics > 700) {
      lastDiagnostics = now;
      diagnostics = await api.speechDiagnostics();
    }
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

<div
  class="shell"
  class:opaque={backdrop === "none"}
  style="--accent:{accent}"
>
  <Chrome />

  <main class="stage">
    {#if firstRun}
      <div class="welcome">
        <div class="welcome-text">
          <strong>Welcome to Textream.</strong> Paste your script below, pick Follow,
          Auto or Voice at the bottom, then press Start
          {#if toggleKeys}
            — or <kbd>{toggleKeys}</kbd> from anywhere.
          {:else}
            .
          {/if}
          Everything runs on this machine; nothing is uploaded.
        </div>
        <button
          class="welcome-dismiss"
          aria-label="Dismiss"
          onclick={dismissFirstRun}>×</button
        >
      </div>
    {/if}

    <textarea
      bind:value={settings.script}
      oninput={persist}
      spellcheck="false"
      placeholder="Paste your script. Put stage directions in [brackets] — they show on the prompter but it never waits for them."
    ></textarea>

    <div class="gauge">
      <span><b>{wordCount}</b> words</span>
      <span class="sep"></span>
      <span
        ><b>{minutes < 1 ? minutes.toFixed(1) : Math.round(minutes)}</b> min at this
        pace</span
      >
      {#if status}
        <span class="sep"></span>
        <span class="status" class:warn={statusKind === "warn"}>{status}</span>
      {/if}
    </div>

    {#if diagnostics}
      <!-- Follow mode only. Seeing what the recogniser actually heard is the
           difference between "it does not work" and a fixable problem. -->
      <div class="heard" class:starved={diagnostics.droppedChunks > 0}>
        <span class="heard-label" title={diagnostics.inputFormat}>Heard</span>
        <span class="heard-text">{diagnostics.heard || "…"}</span>
        {#if diagnostics.droppedChunks > 0}
          <span class="heard-warn"
            >{diagnostics.droppedChunks} audio chunks dropped</span
          >
        {/if}
      </div>
    {/if}
  </main>

  <aside class="panel" class:open={panelOpen} aria-hidden={!panelOpen}>
    <div class="panel-scroll">
      <section>
        <h2>Prompter</h2>
        <label class="field">
          <span class="label">Placement</span>
          <select bind:value={settings.placement} onchange={pushGeometry}>
            <option value="topCenter">Top centre — near the webcam</option>
            <option value="floating">Floating window</option>
            <option value="fullscreen">Fullscreen on this display</option>
            <option value="transportStrip">Strip above the taskbar</option>
          </select>
        </label>
        <p class="note">
          The prompter sits at the top of the screen because that is where your
          camera is. The taskbar strip carries controls only.
        </p>

        <label class="field">
          <span class="label">Width <b>{settings.width}</b></span>
          <input
            type="range"
            min="280"
            max="500"
            bind:value={settings.width}
            oninput={pushGeometry}
          />
        </label>
        <label class="field">
          <span class="label">Height <b>{settings.height}</b></span>
          <input
            type="range"
            min="100"
            max="400"
            bind:value={settings.height}
            oninput={pushGeometry}
          />
        </label>
      </section>

      <section>
        <h2>Type</h2>
        <div class="chips">
          {#each FONTS as font (font.id)}
            <button
              class:on={settings.appearance.fontFamily === font.id}
              onclick={() => {
                settings.appearance.fontFamily = font.id;
                persist();
              }}>{font.label}</button
            >
          {/each}
        </div>
        <div class="chips">
          {#each SIZES as size (size)}
            <button
              class:on={settings.appearance.fontSize === size}
              onclick={() => {
                settings.appearance.fontSize = size;
                persist();
              }}>{size.toUpperCase()}</button
            >
          {/each}
        </div>
      </section>

      <section>
        <h2>Colour</h2>
        <div class="field">
          <span class="label">Spoken word</span>
          <div class="swatches">
            {#each COLOR_IDS as id (id)}
              <button
                class="swatch"
                class:on={settings.appearance.highlight === id}
                style="--c:{COLORS[id]}"
                aria-label="Highlight {id}"
                title={id}
                onclick={() => {
                  settings.appearance.highlight = id;
                  persist();
                }}
              ></button>
            {/each}
          </div>
        </div>
        <div class="field">
          <span class="label">Cues</span>
          <div class="swatches">
            {#each COLOR_IDS as id (id)}
              <button
                class="swatch"
                class:on={settings.appearance.cue === id}
                style="--c:{COLORS[id]}"
                aria-label="Cue {id}"
                title={id}
                onclick={() => {
                  settings.appearance.cue = id;
                  persist();
                }}
              ></button>
            {/each}
          </div>
        </div>
        <label class="field">
          <span class="label"
            >Background <b>{(settings.appearance.opacity * 100).toFixed(0)}%</b
            ></span
          >
          <input
            type="range"
            min="0"
            max="1"
            step="0.01"
            bind:value={settings.appearance.opacity}
            oninput={persist}
          />
        </label>
      </section>

      <section>
        <h2>Behaviour</h2>
        <label class="toggle">
          <input
            type="checkbox"
            bind:checked={settings.hideFromCapture}
            onchange={pushHideFromCapture}
          />
          <span>Hide from screen share and recordings</span>
        </label>
        <label class="toggle">
          <input
            type="checkbox"
            bind:checked={settings.clickThrough}
            onchange={pushClickThrough}
          />
          <span>Let clicks pass through to the app behind</span>
        </label>
        <p class="note">
          Clicks cannot both pass through and land on the prompter. Turn this
          off to click a word and jump there.
        </p>
      </section>

      <section>
        <h2>Shortcuts</h2>
        {#if bindings.length > 0}
          <dl class="keys">
            {#each bindings as [name, keys] (keys)}
              <dt>{name}</dt>
              <dd><kbd>{keys}</kbd></dd>
            {/each}
          </dl>
          <p class="note">
            These work while another app has focus, so you can start and hold a
            take without leaving the camera.
          </p>
        {:else}
          <p class="note">No shortcuts are registered.</p>
        {/if}
      </section>

      <section>
        <h2>Speech</h2>
        {#if models.length > 0}
          <label class="field">
            <span class="label">Language</span>
            <select
              bind:value={settings.modelId}
              disabled={downloading}
              onchange={persist}
            >
              {#each models as model (model.id)}
                <option value={model.id}>
                  {model.label}
                  {model.installed ? "· ready" : `· ${megabytes(model.downloadBytes)}`}
                </option>
              {/each}
            </select>
          </label>

          {#if selectedModel}
            <div class="model">
              <div>
                <strong>{selectedModel.label}</strong>
                <span class="tag"
                  >{selectedModel.installed
                    ? "installed"
                    : megabytes(selectedModel.downloadBytes)}</span
                >
              </div>
              {#if selectedModel.installed}
                <button
                  class="action"
                  disabled={downloading || running}
                  title={running
                    ? "Stop the prompter before removing a model"
                    : "Delete the downloaded files"}
                  onclick={removeModel}>Remove</button
                >
              {:else}
                <button
                  class="action"
                  disabled={downloading}
                  onclick={downloadModel}
                >
                  {downloading ? `${downloadPercent.toFixed(0)}%` : "Download"}
                </button>
              {/if}
            </div>
            {#if downloading}
              <div class="progress">
                <span style="width:{downloadPercent}%"></span>
              </div>
            {/if}
          {/if}
        {:else}
          <p class="note">No speech models are registered.</p>
        {/if}
        <p class="note">
          Recognition runs on this machine. Nothing is uploaded, and there is no
          account.
        </p>
        <p class="note">
          Turkish is not here yet — no streaming model has been published for it
          in a format this engine can load.
        </p>
      </section>
    </div>
  </aside>

  <footer class="dock">
    <LiquidStart
      {running}
      {paused}
      {level}
      listening={needsMicrophone && !muted}
      disabled={transitioning}
      onclick={toggleRun}
    />

    <div class="middle">
      <div class="segmented" style="--i:{modeIndex}">
        <span class="pill" aria-hidden="true"></span>
        {#each MODES as mode (mode.id)}
          <button
            class:on={settings.mode === mode.id}
            disabled={mode.id === "wordTracking" && !speechReady}
            title={mode.id === "wordTracking" && !speechReady
              ? "Download the speech model to enable this"
              : mode.blurb}
            onclick={() => pushMode(mode.id)}
          >
            {mode.label}
          </button>
        {/each}
      </div>

      {#if settings.mode === "classic" || settings.mode === "voiceActivated"}
        <label class="pace">
          <span class="label">Pace <b>{settings.wordsPerSecond.toFixed(1)}</b> w/s</span>
          <input
            type="range"
            min="0.5"
            max="8"
            step="0.1"
            bind:value={settings.wordsPerSecond}
            oninput={pushSpeed}
          />
        </label>
      {:else}
        <p class="pace-note">Follow mode paces itself from your voice.</p>
      {/if}
    </div>

    <div class="right">
      {#if running}
        <div class="readout">
          {#if needsMicrophone}
            <span class="tally" class:live={voiceActive} class:muted></span>
          {/if}
          <span class="count">{Math.floor(wordProgress)}<i>/{wordCount}</i></span>
        </div>
        <button class="action" onclick={togglePause}
          >{paused ? "Resume" : "Hold"}</button
        >
        {#if needsMicrophone}
          <button class="action" class:warn={muted} onclick={toggleMute}
            >{muted ? "Unmute" : "Mute"}</button
          >
        {/if}
      {/if}
      <button
        class="action"
        class:on={panelOpen}
        aria-expanded={panelOpen}
        onclick={() => (panelOpen = !panelOpen)}>Settings</button
      >
    </div>
  </footer>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    /* The window is transparent so the compositor's backdrop shows through. */
    background: transparent;
    overflow: hidden;
    color-scheme: dark;
  }

  .shell {
    /* Bahnschrift is a variable DIN that ships with Windows 10 and 11. It is
       the lettering of broadcast and studio equipment, it costs nothing to
       embed, and it is nobody's default UI face. */
    --display: Bahnschrift, "DIN Alternate", "Segoe UI", sans-serif;
    --body: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;

    --tint: rgba(11, 13, 18, 0.72);
    --surface: rgba(255, 255, 255, 0.045);
    --surface-lift: rgba(255, 255, 255, 0.08);
    --edge: rgba(255, 255, 255, 0.09);
    --ink: #edf0f5;
    --ink-dim: #8a9099;
    --ink-faint: #5c636d;
    /* --accent is set inline from the highlight colour: the app wears
       whatever colour the presenter reads in. */

    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    overflow: hidden;
    /* Square on purpose. The DWM backdrop fills the whole window rectangle, so
       rounding the content would leave the blur showing as sharp corners
       outside it. Windows 11 rounds top-level windows itself anyway. */
    background: var(--tint);
    box-shadow: inset 0 0 0 1px var(--edge);
    color: var(--ink);
    font: 14px/1.55 var(--body);
  }
  /* No compositor effect available: paint an opaque surface rather than
     letting the desktop show straight through a transparent window. */
  .shell.opaque {
    background: #0b0d12;
  }

  .stage {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 4px 22px 0;
  }

  textarea {
    flex: 1;
    min-height: 0;
    resize: none;
    border: 0;
    padding: 8px 2px 16px;
    background: transparent;
    color: var(--ink);
    font: 16px/1.85 var(--body);
    outline: none;
  }
  textarea::placeholder {
    color: var(--ink-faint);
  }

  .welcome {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin: 8px 0 4px;
    padding: 12px 14px;
    border: 1px solid var(--edge);
    border-radius: 10px;
    background: var(--surface);
    flex: none;
  }
  .welcome-text {
    flex: 1;
    color: var(--ink-dim);
    font-size: 12.5px;
    line-height: 1.6;
  }
  .welcome-text strong {
    color: var(--ink);
    font-weight: 600;
  }
  .welcome-dismiss {
    flex: none;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-faint);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }
  .welcome-dismiss:hover {
    background: var(--surface-lift);
    color: var(--ink);
  }

  .gauge {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 2px;
    border-top: 1px solid var(--edge);
    font-family: var(--display);
    font-size: 11px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }
  .gauge b {
    color: var(--ink-dim);
    font-variant-numeric: tabular-nums;
  }
  .sep {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.5;
  }
  .status {
    text-transform: none;
    letter-spacing: 0.02em;
  }
  .status.warn {
    color: #ff9e0a;
  }

  .heard {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 8px 2px 10px;
    border-top: 1px solid var(--edge);
    min-width: 0;
  }
  .heard-label {
    font-family: var(--display);
    font-size: 10.5px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--ink-faint);
    flex: none;
  }
  .heard-text {
    flex: 1;
    min-width: 0;
    color: var(--ink-dim);
    font-size: 12.5px;
    font-style: italic;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
  }
  .heard-warn {
    flex: none;
    color: #ff9e0a;
    font-family: var(--display);
    font-size: 10.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .heard.starved .heard-label {
    color: #ff9e0a;
  }

  /* ---- settings panel ---- */
  .panel {
    position: absolute;
    top: 38px;
    right: 0;
    bottom: 110px;
    width: 340px;
    z-index: 3;
    background: rgba(9, 11, 15, 0.86);
    border-left: 1px solid var(--edge);
    backdrop-filter: blur(24px) saturate(140%);
    transform: translateX(100%);
    opacity: 0;
    visibility: hidden;
    /* `visibility` animates discretely — given a duration it flips at the
       halfway point, so the panel would spend the first half of its entrance
       invisible and then appear mid-slide. Zero duration with a delay instead:
       it becomes visible immediately on open, and only hides once the closing
       slide has finished. */
    transition:
      transform 380ms cubic-bezier(0.32, 1.35, 0.5, 1),
      opacity 200ms ease,
      visibility 0s linear 380ms;
  }
  .panel.open {
    transform: translateX(0);
    opacity: 1;
    visibility: visible;
    transition:
      transform 380ms cubic-bezier(0.32, 1.35, 0.5, 1),
      opacity 200ms ease,
      visibility 0s linear 0s;
  }
  .panel-scroll {
    height: 100%;
    overflow-y: auto;
    padding: 20px 22px 28px;
    box-sizing: border-box;
  }
  .panel section + section {
    margin-top: 26px;
  }
  h2 {
    margin: 0 0 12px;
    font-family: var(--display);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--ink-faint);
  }

  .field {
    display: block;
    margin-bottom: 14px;
  }
  .label {
    display: block;
    margin-bottom: 7px;
    font-family: var(--display);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-dim);
  }
  .label b {
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }
  .note {
    margin: 8px 0 0;
    color: var(--ink-faint);
    font-size: 11.5px;
    line-height: 1.5;
  }

  select {
    width: 100%;
    padding: 9px 10px;
    border: 1px solid var(--edge);
    border-radius: 8px;
    background: var(--surface);
    color: inherit;
    font: inherit;
    font-size: 13px;
  }
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
  }

  .chips {
    display: flex;
    gap: 6px;
    margin-bottom: 8px;
  }
  .chips button {
    flex: 1;
    padding: 8px 4px;
    border: 1px solid var(--edge);
    border-radius: 8px;
    background: var(--surface);
    color: var(--ink-dim);
    font-family: var(--display);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      transform 320ms cubic-bezier(0.34, 1.56, 0.64, 1),
      background 160ms ease,
      color 160ms ease;
  }
  .chips button:hover {
    background: var(--surface-lift);
    color: var(--ink);
  }
  .chips button.on {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--ink);
    transform: translateY(-1px);
  }

  .keys {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 7px 12px;
    margin: 0;
    align-items: center;
  }
  .keys dt {
    font-size: 13px;
    color: var(--ink-dim);
  }
  .keys dd {
    margin: 0;
  }
  kbd {
    display: inline-block;
    padding: 3px 8px;
    border: 1px solid var(--edge);
    border-radius: 6px;
    background: var(--surface);
    color: var(--ink);
    font-family: var(--display);
    font-size: 11px;
    letter-spacing: 0.04em;
    white-space: nowrap;
  }

  .swatches {
    display: flex;
    gap: 8px;
  }
  .swatch {
    width: 22px;
    height: 22px;
    padding: 0;
    border: 2px solid transparent;
    border-radius: 50%;
    background: var(--c);
    cursor: pointer;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.35);
    transition: transform 320ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .swatch:hover {
    transform: scale(1.14);
  }
  .swatch.on {
    border-color: var(--ink);
    transform: scale(1.08);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 0;
    font-size: 13px;
    cursor: pointer;
  }
  .toggle input {
    accent-color: var(--accent);
  }

  .model {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 11px 13px;
    border: 1px solid var(--edge);
    border-radius: 9px;
    background: var(--surface);
  }
  .tag {
    margin-left: 8px;
    color: var(--ink-faint);
    font-family: var(--display);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .progress {
    height: 3px;
    margin-top: 9px;
    border-radius: 2px;
    background: var(--surface-lift);
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 200ms linear;
  }

  /* ---- dock ---- */
  .dock {
    position: relative;
    z-index: 4;
    display: flex;
    align-items: center;
    gap: 22px;
    height: 110px;
    padding: 0 20px;
    flex: none;
    border-top: 1px solid var(--edge);
    background: rgba(255, 255, 255, 0.02);
  }
  .middle {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .segmented {
    position: relative;
    display: flex;
    gap: 2px;
    padding: 4px;
    border: 1px solid var(--edge);
    border-radius: 11px;
    background: var(--surface);
    max-width: 330px;
  }
  /* The indicator overshoots and settles rather than sliding linearly — the
     one place the dock borrows the blob's physics. */
  .pill {
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: 4px;
    width: calc((100% - 8px) / 3);
    border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent);
    transform: translateX(calc(var(--i) * 100%));
    transition: transform 460ms cubic-bezier(0.32, 1.45, 0.46, 1);
  }
  .segmented button {
    position: relative;
    z-index: 1;
    flex: 1;
    padding: 8px 6px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--ink-dim);
    font-family: var(--display);
    font-size: 12px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    cursor: pointer;
    transition: color 200ms ease;
  }
  .segmented button:hover:not(:disabled) {
    color: var(--ink);
  }
  .segmented button.on {
    color: var(--ink);
  }
  .segmented button:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .pace {
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: 330px;
  }
  .pace .label {
    margin: 0;
    white-space: nowrap;
  }
  .pace-note {
    margin: 0;
    color: var(--ink-faint);
    font-size: 11.5px;
  }

  .action {
    padding: 8px 15px;
    border: 1px solid var(--edge);
    border-radius: 9px;
    background: var(--surface);
    color: var(--ink-dim);
    font-family: var(--display);
    font-size: 11.5px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      background 160ms ease,
      color 160ms ease,
      transform 320ms cubic-bezier(0.34, 1.56, 0.64, 1);
  }
  .action:hover:not(:disabled) {
    background: var(--surface-lift);
    color: var(--ink);
    transform: translateY(-1px);
  }
  .action:active:not(:disabled) {
    transform: translateY(0) scale(0.97);
  }
  .action:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .action.on {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--ink);
  }
  .action.warn {
    border-color: #7a4a12;
    color: #ff9e0a;
  }

  .readout {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-right: 4px;
  }
  .tally {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ink-faint);
    transition:
      background 140ms ease,
      box-shadow 140ms ease;
  }
  .tally.live {
    background: var(--accent);
    box-shadow: 0 0 12px var(--accent);
  }
  .tally.muted {
    background: #ff9e0a;
    box-shadow: none;
  }
  .count {
    font-family: var(--display);
    font-size: 15px;
    font-variant-numeric: tabular-nums;
    color: var(--ink);
  }
  .count i {
    font-style: normal;
    font-size: 12px;
    color: var(--ink-faint);
  }

  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .pill,
    .panel,
    .action,
    .chips button,
    .swatch {
      transition-duration: 1ms;
    }
  }
</style>
