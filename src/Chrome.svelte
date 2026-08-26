<script lang="ts">
  /**
   * Window chrome for a frameless window.
   *
   * Turning decorations off takes the title bar, the buttons and the resize
   * border with it, so all three are rebuilt here. Resizing goes through
   * `startResizeDragging` rather than relying on the window manager's own
   * hit-testing, which an undecorated window does not reliably get.
   */
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  /**
   * Resolved per call rather than once at module scope.
   *
   * `getCurrentWindow` reads globals the Tauri host injects, so calling it
   * while the module loads throws outside the app — which would take the whole
   * UI down instead of just the chrome, and makes the interface impossible to
   * open in a plain browser while working on it.
   */
  const host = () => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  };

  let maximized = $state(false);

  const EDGES = [
    { dir: "North", cls: "n" },
    { dir: "South", cls: "s" },
    { dir: "West", cls: "w" },
    { dir: "East", cls: "e" },
    { dir: "NorthWest", cls: "nw" },
    { dir: "NorthEast", cls: "ne" },
    { dir: "SouthWest", cls: "sw" },
    { dir: "SouthEast", cls: "se" },
  ] as const;

  onMount(() => {
    const window = host();
    if (!window) return;
    const sync = () => void window.isMaximized().then((value) => (maximized = value));
    sync();
    const unlisten = window.onResized(sync);
    return () => {
      void unlisten.then((off) => off());
    };
  });

  /** Only a primary-button press starts a resize; right-click must not. */
  function beginResize(
    event: PointerEvent,
    direction: (typeof EDGES)[number]["dir"],
  ) {
    if (event.button !== 0) return;
    void host()?.startResizeDragging(direction);
  }

  /**
   * Starts a window drag, unless this is the second click of a double.
   *
   * `startDragging` hands the pointer to the window manager, which swallows
   * the events a `dblclick` would be assembled from — so the double-click to
   * maximise has to be detected here, before the drag begins.
   */
  function beginDrag(event: PointerEvent) {
    if (event.button !== 0) return;
    if (event.detail >= 2) {
      void host()?.toggleMaximize();
      return;
    }
    void host()?.startDragging();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="bar" onpointerdown={beginDrag}>
  <div class="mark">
    <span class="glyph" aria-hidden="true"></span>
    <span class="name">Textream</span>
  </div>

  <div class="controls">
    <button
      class="chip"
      title="Minimise"
      aria-label="Minimise"
      onpointerdown={(event) => event.stopPropagation()}
      onclick={() => host()?.minimize()}
    >
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1 5h8" /></svg>
    </button>
    <button
      class="chip"
      title={maximized ? "Restore" : "Maximise"}
      aria-label={maximized ? "Restore" : "Maximise"}
      onpointerdown={(event) => event.stopPropagation()}
      onclick={() => host()?.toggleMaximize()}
    >
      {#if maximized}
        <svg viewBox="0 0 10 10" aria-hidden="true"
          ><path d="M3 1h6v6M1 3h6v6H1z" /></svg
        >
      {:else}
        <svg viewBox="0 0 10 10" aria-hidden="true"
          ><path d="M1 1h8v8H1z" /></svg
        >
      {/if}
    </button>
    <button
      class="chip close"
      title="Close"
      aria-label="Close"
      onpointerdown={(event) => event.stopPropagation()}
      onclick={() => host()?.close()}
    >
      <svg viewBox="0 0 10 10" aria-hidden="true"
        ><path d="M1 1l8 8M9 1l-8 8" /></svg
      >
    </button>
  </div>
</header>

{#if !maximized}
  {#each EDGES as edge (edge.dir)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="grip {edge.cls}"
      onpointerdown={(event) => beginResize(event, edge.dir)}
    ></div>
  {/each}
{/if}

<style>
  .bar {
    position: relative;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 38px;
    padding-left: 16px;
    flex: none;
    -webkit-user-select: none;
    user-select: none;
  }

  .mark {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .glyph {
    width: 9px;
    height: 9px;
    border-radius: 50% 50% 50% 12%;
    background: var(--accent);
    box-shadow: 0 0 10px color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .name {
    font-family: var(--display);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--ink-dim);
  }

  .controls {
    display: flex;
    height: 100%;
  }
  .chip {
    width: 44px;
    height: 100%;
    display: grid;
    place-items: center;
    border: 0;
    background: transparent;
    color: var(--ink-dim);
    cursor: pointer;
    transition:
      background 120ms ease,
      color 120ms ease;
  }
  .chip svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.2;
  }
  .chip:hover {
    background: var(--surface-lift);
    color: var(--ink);
  }
  .chip.close:hover {
    background: #e81123;
    color: #fff;
  }
  .chip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -3px;
  }

  /* Resize grips sit above everything and carry no visuals of their own. */
  .grip {
    position: fixed;
    z-index: 50;
  }
  .n,
  .s {
    left: 6px;
    right: 6px;
    height: 5px;
    cursor: ns-resize;
  }
  .n {
    top: 0;
  }
  .s {
    bottom: 0;
  }
  .w,
  .e {
    top: 6px;
    bottom: 6px;
    width: 5px;
    cursor: ew-resize;
  }
  .w {
    left: 0;
  }
  .e {
    right: 0;
  }
  .nw,
  .ne,
  .sw,
  .se {
    width: 10px;
    height: 10px;
  }
  .nw {
    top: 0;
    left: 0;
    cursor: nwse-resize;
  }
  .ne {
    top: 0;
    right: 0;
    cursor: nesw-resize;
  }
  .sw {
    bottom: 0;
    left: 0;
    cursor: nesw-resize;
  }
  .se {
    bottom: 0;
    right: 0;
    cursor: nwse-resize;
  }
</style>
