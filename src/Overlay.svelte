<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    EVENT_APPEARANCE,
    EVENT_PROGRESS,
    EVENT_SCRIPT,
    jumpToWord,
    type AppearanceView,
    type ProgressView,
    type ScriptView,
  } from "./lib/api";

  let script = $state<ScriptView | null>(null);
  let progress = $state<ProgressView | null>(null);
  let appearance = $state<AppearanceView | null>(null);
  let viewport: HTMLDivElement | undefined = $state();

  const activeWord = $derived(progress?.activeWord ?? -1);

  /** Words moved per wheel notch when scrolling to catch up. */
  const WORDS_PER_NOTCH = 3;

  /**
   * Scrolls the reading position rather than the viewport.
   *
   * Moving only the scrollbar would look right for a moment and then snap back
   * the instant the next progress event arrives, because the effect below
   * follows the active word. Jumping the tracker is what the presenter means
   * by scrolling: get ahead, or go back and pick up a line again.
   */
  async function catchUp(event: WheelEvent) {
    if (!script || script.words.length === 0) return;
    event.preventDefault();

    const direction = Math.sign(event.deltaY);
    if (direction === 0) return;
    const from = activeWord < 0 ? 0 : activeWord;
    const target = Math.min(
      script.words.length - 1,
      Math.max(0, from + direction * WORDS_PER_NOTCH),
    );
    progress = await jumpToWord(target);
  }

  const style = $derived(
    appearance
      ? [
          `--font-stack:${appearance.fontStack}`,
          `--font-size:${appearance.fontSizePx}px`,
          `--highlight:${appearance.highlight}`,
          `--cue:${appearance.cue}`,
          `--bg-opacity:${appearance.opacity}`,
        ].join(";")
      : "",
  );

  onMount(() => {
    const unlisten = Promise.all([
      listen<ScriptView>(EVENT_SCRIPT, (event) => {
        script = event.payload;
        progress = null;
      }),
      listen<ProgressView>(EVENT_PROGRESS, (event) => {
        progress = event.payload;
      }),
      listen<AppearanceView>(EVENT_APPEARANCE, (event) => {
        appearance = event.payload;
      }),
    ]);
    return () => {
      void unlisten.then((offs) => offs.forEach((off) => off()));
    };
  });

  /**
   * Keeps the active word parked one third down the viewport rather than
   * centred. The presenter needs to see what is coming more than what is gone,
   * and a centred anchor wastes half the overlay on already-read text.
   */
  $effect(() => {
    if (!viewport || activeWord < 0) return;
    const node = viewport.querySelector<HTMLElement>(
      `[data-word="${activeWord}"]`,
    );
    if (!node) return;
    const target =
      node.offsetTop - viewport.clientHeight / 3 + node.offsetHeight / 2;
    viewport.scrollTo({ top: Math.max(0, target), behavior: "smooth" });
  });
</script>

<div class="pill" dir={script?.direction ?? "ltr"} {style}>
  {#if script && script.words.length > 0}
    <div class="viewport" bind:this={viewport} onwheel={catchUp}>
      <p class="words">
        {#each script.words as word (word.id)}<button
            type="button"
            data-word={word.id}
            class="word"
            class:read={word.id < activeWord}
            class:active={word.id === activeWord}
            class:cue={word.isAnnotation}
            title="Jump here"
            onclick={() => jumpToWord(word.id)}>{word.text}</button
          >{" "}{/each}
      </p>
    </div>
    <div class="rail">
      <span
        style="width:{script.words.length
          ? ((progress?.wordProgress ?? 0) / script.words.length) * 100
          : 0}%"
      ></span>
    </div>
  {:else}
    <p class="empty">Waiting for a script…</p>
  {/if}
</div>

<style>
  /* Bundled so the dyslexia-friendly face works on a machine that has never
     installed it — the whole point of offering it. */
  @font-face {
    font-family: "OpenDyslexic Three";
    src: url("./assets/fonts/OpenDyslexic3-Regular.ttf") format("truetype");
    font-weight: 400;
    font-display: swap;
  }

  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    /* The window itself is transparent; only the pill paints. */
    background: transparent;
    overflow: hidden;
    color-scheme: dark;
  }

  .pill {
    /* Defaults for the frame or two before the first appearance event. */
    --font-stack: "Segoe UI Variable Display", "Segoe UI", system-ui, sans-serif;
    --font-size: 20px;
    --highlight: #ffd60a;
    --cue: #ff9e0a;
    --bg-opacity: 0.92;

    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    height: 100vh;
    padding: 14px 20px 10px;
    border-radius: 18px;
    background: rgba(10, 11, 13, var(--bg-opacity));
    /* Mica and Acrylic are unavailable to a transparent WebView2 surface, so
       the frosted look is done in CSS against whatever shows through. */
    backdrop-filter: blur(20px) saturate(140%);
    box-shadow:
      0 8px 32px rgba(0, 0, 0, 0.45),
      inset 0 0 0 1px rgba(255, 255, 255, 0.06);
    font:
      600 var(--font-size) / 1.65 var(--font-stack);
  }

  .viewport {
    flex: 1;
    overflow: hidden;
    /* Fades the edges so text arrives and leaves instead of being clipped. */
    mask-image: linear-gradient(
      to bottom,
      transparent,
      #000 14%,
      #000 82%,
      transparent
    );
  }

  .words {
    margin: 0;
    padding: 0;
    color: rgba(255, 255, 255, 0.32);
    overflow-wrap: break-word;
  }

  .word {
    /* A button for the keyboard and for screen readers, styled back down to
       text — tapping a word to jump has to hit something focusable. */
    display: inline;
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    transition:
      color 140ms ease,
      opacity 140ms ease;
  }
  .word.read {
    color: rgba(255, 255, 255, 0.62);
  }
  .word.active {
    color: var(--highlight);
  }
  .word.cue {
    /* Cues are shown — the presenter still needs the direction — but never
       highlighted, so they read as instruction rather than as script. */
    color: var(--cue);
    opacity: 0.55;
    font-style: italic;
    font-weight: 500;
  }
  .word.cue.active {
    opacity: 0.75;
  }

  .rail {
    height: 3px;
    margin-top: 8px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
    flex: none;
  }
  .rail span {
    display: block;
    height: 100%;
    background: var(--highlight);
    transition: width 120ms linear;
  }

  .empty {
    margin: auto;
    color: rgba(255, 255, 255, 0.35);
    font-size: 14px;
    font-weight: 500;
  }
</style>
