<script lang="ts">
  /**
   * The transport control, and the one place this app spends its boldness.
   *
   * A teleprompter's whole job is the boundary between speech and text, so the
   * control that starts one is the thing that listens: three offset blobs
   * driven by a spring off the microphone level, so the button swells as you
   * speak and settles as you stop. Everything else in the window stays flat and
   * quiet on purpose.
   */
  import { onDestroy, onMount } from "svelte";
  import { createSpring, prefersReducedMotion } from "./lib/spring";

  interface Props {
    running: boolean;
    paused: boolean;
    /** Microphone level, 0..1. Ignored when the mode has no microphone. */
    level: number;
    listening: boolean;
    /**
     * True while a start or stop is already in flight.
     *
     * Both open a microphone and a model that take real time to settle, so a
     * second click before the first finishes would race two attempts against
     * the same device — this makes the button inert until one completes.
     */
    disabled?: boolean;
    onclick: () => void;
  }

  let { running, paused, level, listening, disabled = false, onclick }: Props =
    $props();

  let swell = $state(0);
  let frame = 0;
  const spring = createSpring(0, { stiffness: 0.2, damping: 0.7 });
  const calm = prefersReducedMotion();

  onMount(() => {
    if (calm) return;
    const step = () => {
      // A quiet room still has a noise floor; ignoring the bottom of the range
      // stops the button breathing at nothing.
      const excited = listening ? Math.max(0, level - 0.01) * 6 : 0;
      spring.target = Math.min(1, excited);
      swell = spring.step();
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  });

  onDestroy(() => cancelAnimationFrame(frame));

  const label = $derived(running ? (paused ? "Held" : "Live") : "Start");
</script>

<button
  class="liquid"
  class:running
  class:paused
  class:calm
  style="--swell:{swell}"
  {disabled}
  {onclick}
  aria-label={running ? "Stop the prompter" : "Start the prompter"}
>
  <span class="blob a" aria-hidden="true"></span>
  <span class="blob b" aria-hidden="true"></span>
  <span class="blob c" aria-hidden="true"></span>
  <span class="face">{label}</span>
</button>

<style>
  .liquid {
    position: relative;
    display: grid;
    place-items: center;
    width: 92px;
    height: 92px;
    flex: none;
    border: 0;
    padding: 0;
    background: none;
    color: var(--ink);
    cursor: pointer;
    isolation: isolate;
  }
  .liquid:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .liquid:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 4px;
    border-radius: 50%;
  }

  .blob {
    position: absolute;
    inset: 10px;
    background: var(--accent);
    opacity: 0.22;
    /* Asymmetric radii are what make it read as a droplet rather than a
       circle; animating between two sets is the whole morph. */
    border-radius: 46% 54% 61% 39% / 43% 47% 53% 57%;
    transform: scale(calc(0.94 + var(--swell) * 0.22));
    transition: opacity 260ms ease;
    will-change: transform, border-radius;
  }
  .blob.a {
    animation: churn 7s ease-in-out infinite;
  }
  .blob.b {
    animation: churn 9s ease-in-out infinite reverse;
    opacity: 0.16;
  }
  .blob.c {
    animation: churn 11s ease-in-out infinite;
    opacity: 0.12;
  }

  .liquid.running .blob {
    opacity: 0.42;
  }
  .liquid.running .blob.b {
    opacity: 0.3;
  }
  .liquid.running .blob.c {
    opacity: 0.22;
  }
  .liquid.paused .blob {
    opacity: 0.14;
    animation-play-state: paused;
  }

  .liquid:hover .blob {
    opacity: 0.34;
  }

  .face {
    position: relative;
    z-index: 1;
    font-family: var(--display);
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    /* The ring is the button's actual edge; the blobs are atmosphere. */
    display: grid;
    place-items: center;
    width: 68px;
    height: 68px;
    border-radius: 50%;
    background: var(--surface-lift);
    box-shadow: inset 0 0 0 1px var(--edge);
    backdrop-filter: blur(6px);
    transition:
      transform 420ms cubic-bezier(0.34, 1.56, 0.64, 1),
      color 200ms ease;
  }
  .liquid:hover .face {
    transform: scale(1.06);
  }
  .liquid:active .face {
    transform: scale(0.94);
  }
  .liquid.running .face {
    color: var(--accent);
  }

  @keyframes churn {
    0%,
    100% {
      border-radius: 46% 54% 61% 39% / 43% 47% 53% 57%;
      rotate: 0deg;
    }
    33% {
      border-radius: 62% 38% 41% 59% / 56% 62% 38% 44%;
      rotate: 6deg;
    }
    66% {
      border-radius: 38% 62% 56% 44% / 61% 39% 61% 39%;
      rotate: -5deg;
    }
  }

  /* Reduced motion keeps the control and its states, and drops the life. */
  .liquid.calm .blob {
    animation: none;
    transform: none;
    border-radius: 50%;
  }
  .liquid.calm .face {
    transition: color 200ms ease;
  }

  @media (prefers-reduced-motion: reduce) {
    .blob {
      animation: none !important;
      transform: none !important;
    }
    .face {
      transition: color 200ms ease;
    }
  }
</style>
