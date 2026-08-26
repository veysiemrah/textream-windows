/**
 * A tiny critically-under-damped spring integrator.
 *
 * CSS easing curves cannot follow a value that keeps changing — the microphone
 * level updates many times a second, and restarting a transition on every
 * update produces stutter rather than motion. A spring absorbs a moving target
 * and settles on its own.
 */
export interface Spring {
  /** Where the spring is being pulled. */
  target: number;
  /** Current position. */
  readonly value: number;
  /** Advances one frame and returns the new position. */
  step(): number;
  /** Jumps to a value with no motion. */
  snap(to: number): void;
}

export interface SpringOptions {
  /** How hard the spring pulls. Higher is snappier. */
  stiffness?: number;
  /** How much motion survives each frame. Lower settles sooner. */
  damping?: number;
}

export function createSpring(
  initial: number,
  { stiffness = 0.14, damping = 0.72 }: SpringOptions = {},
): Spring {
  let value = initial;
  let velocity = 0;

  return {
    target: initial,
    get value() {
      return value;
    },
    step() {
      velocity = (velocity + (this.target - value) * stiffness) * damping;
      value += velocity;
      // Park exactly on target once the motion is below display resolution,
      // so an idle spring stops costing a repaint every frame.
      if (Math.abs(velocity) < 0.0001 && Math.abs(this.target - value) < 0.0001) {
        value = this.target;
        velocity = 0;
      }
      return value;
    },
    snap(to: number) {
      value = to;
      velocity = 0;
      this.target = to;
    },
  };
}

/** True when the viewer asked for less motion. */
export const prefersReducedMotion = () =>
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;
