<script lang="ts">
  /**
   * A designed dropdown for choosing an input device.
   *
   * Not a bare `<select>` — Windows renders those with the OS chrome, which
   * clashes with everything else in this panel. This is a plain
   * button-plus-listbox combobox: it owns its own open state, keyboard
   * handling and focus, and reports the chosen device id (or `null` for the
   * system default) through `onchange`.
   */
  import { tick as nextTick } from "svelte";
  import type { InputDevice } from "./lib/api";

  interface Props {
    /** Device id, or `null` for the system default. */
    value: string | null;
    devices: InputDevice[];
    /** Live microphone level, 0..1. Only meaningful while `active`. */
    level?: number;
    /** Whether the meter should animate — a session is running and listening. */
    active?: boolean;
    disabled?: boolean;
    /** Called every time the list opens, so it can be refreshed for hot-plug. */
    onrefresh?: () => void;
    onchange?: (value: string | null) => void;
  }

  let {
    value,
    devices,
    level = 0,
    active = false,
    disabled = false,
    onrefresh,
    onchange,
  }: Props = $props();

  let open = $state(false);
  let highlighted = $state(0);
  let rootEl: HTMLDivElement | undefined = $state();
  let listEl: HTMLUListElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();

  // "System default" is always the first row; the real devices follow in
  // whatever order cpal enumerated them.
  const optionIds = $derived<(string | null)[]>([
    null,
    ...devices.map((d) => d.id),
  ]);

  const defaultDevice = $derived(devices.find((d) => d.isDefault));

  function labelFor(id: string | null): string {
    if (id === null) {
      return defaultDevice
        ? `System default (${defaultDevice.name})`
        : "System default";
    }
    return devices.find((d) => d.id === id)?.name ?? id;
  }

  const currentLabel = $derived(labelFor(value));
  // A chosen device that vanished (unplugged, renamed) still shows its old
  // name rather than silently blanking — matching what Rust does, which
  // falls back to the default device without touching the saved id.
  const currentMissing = $derived(
    value !== null && !devices.some((d) => d.id === value),
  );

  function meterPercent(): number {
    // A quiet room still reads above zero on a linear meter; matching the
    // liquid button's floor keeps this from looking "on" at silence.
    return Math.round(Math.min(1, Math.max(0, level - 0.01) * 6) * 100);
  }

  async function openList() {
    if (disabled) return;
    onrefresh?.();
    open = true;
    highlighted = Math.max(0, optionIds.indexOf(value));
    await nextTick();
    listEl?.focus();
  }

  function closeList(refocusButton = true) {
    open = false;
    if (refocusButton) buttonEl?.focus();
  }

  function choose(id: string | null) {
    if (id !== value) onchange?.(id);
    closeList();
  }

  function onButtonKeydown(event: KeyboardEvent) {
    if (
      event.key === "ArrowDown" ||
      event.key === "ArrowUp" ||
      event.key === "Enter" ||
      event.key === " "
    ) {
      event.preventDefault();
      void openList();
    }
  }

  function onListKeydown(event: KeyboardEvent) {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        highlighted = Math.min(optionIds.length - 1, highlighted + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        highlighted = Math.max(0, highlighted - 1);
        break;
      case "Home":
        event.preventDefault();
        highlighted = 0;
        break;
      case "End":
        event.preventDefault();
        highlighted = optionIds.length - 1;
        break;
      case "Enter":
      case " ":
        event.preventDefault();
        choose(optionIds[highlighted] ?? null);
        break;
      case "Escape":
        event.preventDefault();
        closeList();
        break;
      case "Tab":
        closeList(false);
        break;
    }
  }

  function onWindowPointerDown(event: PointerEvent) {
    if (!open) return;
    if (rootEl && !rootEl.contains(event.target as Node)) closeList(false);
  }

  $effect(() => {
    if (!open) return;
    window.addEventListener("pointerdown", onWindowPointerDown, true);
    return () =>
      window.removeEventListener("pointerdown", onWindowPointerDown, true);
  });
</script>

<div class="msel" bind:this={rootEl}>
  <button
    type="button"
    bind:this={buttonEl}
    class="msel-button"
    class:open
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    title={currentLabel}
    onclick={() => (open ? closeList() : openList())}
    onkeydown={onButtonKeydown}
  >
    <span class="msel-dot" class:missing={currentMissing} aria-hidden="true"
    ></span>
    <span class="msel-name">{currentLabel}</span>
    <span
      class="msel-meter"
      class:active
      aria-hidden="true"
      title={active
        ? "Microphone level"
        : "The level meter shows while the prompter is running"}
    >
      <span class="msel-meter-fill" style="width:{active ? meterPercent() : 0}%"
      ></span>
    </span>
    <svg class="msel-chevron" viewBox="0 0 10 6" aria-hidden="true">
      <path d="M1 1l4 4 4-4" />
    </svg>
  </button>

  {#if open}
    <ul
      bind:this={listEl}
      class="msel-list"
      role="listbox"
      tabindex="-1"
      aria-activedescendant="msel-opt-{highlighted}"
      onkeydown={onListKeydown}
      onblur={() => closeList(false)}
    >
      {#each optionIds as id, index (id ?? "__default__")}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- Keyboard selection is handled by the listbox's own keydown
             handler above; this click is the pointer path only. -->
        <li
          id="msel-opt-{index}"
          role="option"
          aria-selected={value === id}
          class:on={value === id}
          class:highlighted={highlighted === index}
          title={labelFor(id)}
          onpointerenter={() => (highlighted = index)}
          onclick={() => choose(id)}
        >
          {labelFor(id)}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .msel {
    position: relative;
  }

  .msel-button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 9px 10px;
    border: 1px solid var(--edge);
    border-radius: 8px;
    background: var(--surface);
    color: inherit;
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
    transition:
      background 160ms ease,
      border-color 160ms ease;
  }
  .msel-button:hover:not(:disabled) {
    background: var(--surface-lift);
  }
  .msel-button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .msel-button.open {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .msel-button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  .msel-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .msel-dot.missing {
    background: #ff9e0a;
  }

  .msel-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .msel-meter {
    flex: none;
    width: 36px;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-lift);
    overflow: hidden;
    box-shadow: inset 0 0 0 1px var(--edge);
  }
  .msel-meter-fill {
    display: block;
    height: 100%;
    background: var(--ink-faint);
    transition: width 90ms linear;
  }
  .msel-meter.active .msel-meter-fill {
    background: var(--accent);
  }

  .msel-chevron {
    flex: none;
    width: 10px;
    height: 6px;
    fill: none;
    stroke: var(--ink-faint);
    stroke-width: 1.3;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 160ms ease;
  }
  .msel-button.open .msel-chevron {
    transform: rotate(180deg);
  }

  .msel-list {
    position: absolute;
    z-index: 5;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    max-height: 220px;
    margin: 0;
    padding: 6px;
    list-style: none;
    overflow-y: auto;
    border: 1px solid var(--edge);
    border-radius: 10px;
    background: rgba(14, 16, 21, 0.96);
    backdrop-filter: blur(20px) saturate(140%);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.45);
    outline: none;
  }
  .msel-list li {
    padding: 8px 10px;
    border-radius: 7px;
    color: var(--ink-dim);
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  .msel-list li.highlighted {
    background: var(--surface-lift);
    color: var(--ink);
  }
  .msel-list li.on {
    color: var(--ink);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent);
  }

  @media (prefers-reduced-motion: reduce) {
    .msel-chevron,
    .msel-meter-fill {
      transition-duration: 1ms;
    }
  }
</style>
