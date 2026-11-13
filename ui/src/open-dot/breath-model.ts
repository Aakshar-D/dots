import type { VoiceState } from "@/api/types";

/// The numbers behind The Breath: where the three bars of the Sage mark are, how wide, how tall
/// and how bright, frame by frame. The mark is three stacked pill bars that step like an S; each
/// state gives every bar a target made of slow sines and the smoothed sound level, and the bars
/// ease toward it. The model never reads a clock: `step` is given the time, so the same inputs
/// always give the same frames.

export type BreathState =
  | "resting"
  | "listening"
  | "thinking"
  | "speaking"
  | "asking"
  | "done"
  | "trouble";
/// One pill bar, in a 200 × 200 box. `x`, `y` are its centre.
export type Bar = { x: number; y: number; w: number; h: number; opacity: number };
/// `extra`: the fourth bar shown while asking with four options, empty otherwise.
/// `mic`, `voice`: the smoothed levels, exactly 0 once too small to see. `settled`: every value
/// is at its target, so with the same input the next frame is this one.
export type Frame = {
  bars: [Bar, Bar, Bar];
  extra: Bar[];
  glow: number;
  mic: number;
  voice: number;
  settled: boolean;
};
export type BreathInput = {
  state: BreathState;
  mic: number;
  voice: number;
  options: number;
  still: boolean;
};

/// How long a change of state takes: the values ease from wherever they are toward the new
/// targets, and are within 5% of them after this long.
export const MORPH_MS = 320;
/// How long `done` shows after a reply ends, before the mark rests.
export const DONE_MS = 600;

const MIDDLE = 100;
/// Every bar is this tall at rest, and a pill: its ends are half circles.
const THICK = 25;
/// How far apart the bars' centres are, top to bottom.
const PITCH = 35;

type Body = { x: number; y: number; w: number; h: number };

/// The logo arrangement: the top bar sits right of the middle one and the bottom bar left, so
/// the three step like an S. The fourth body is the extra bar, which has no width until asking
/// with four options; it waits where it will appear, below the others.
const HOME: readonly Body[] = [
  { x: MIDDLE + 10, y: MIDDLE - PITCH, w: 80, h: THICK },
  { x: MIDDLE, y: MIDDLE, w: 100, h: THICK },
  { x: MIDDLE - 10, y: MIDDLE + PITCH, w: 80, h: THICK },
  { x: MIDDLE, y: MIDDLE + 1.5 * PITCH, w: 0, h: THICK },
];

/// The logo arrangement at full brightness.
export const LOGO: readonly [Bar, Bar, Bar] = [
  { ...HOME[0]!, opacity: 1 },
  { ...HOME[1]!, opacity: 1 },
  { ...HOME[2]!, opacity: 1 },
];

/// Easing rate per second: 1 - e^(-3) = 95% of the way after MORPH_MS.
const MORPH_RATE = 3000 / MORPH_MS;
/// Brightness eases a little slower.
const FADE_RATE = 6;
/// The level rises fast and falls back more slowly, so syllables read as a swell rather than a
/// flicker.
const ATTACK_RATE = 14;
const RELEASE_RATE = 7;
/// The longest step one frame may take, in seconds. A frame after the tab was hidden eases like
/// a slow frame instead of cutting to the target.
const MAX_DT = 0.05;
/// Below this a value is at its target.
const SNAP = 1e-3;
/// Below this a smoothed level is silence, so a mark at rest can tell nothing moves.
const LEVEL_SNAP = 1e-4;
/// Below this width the extra bar is not there.
const SEEN = 0.05;
/// The glow behind the middle bar, and how much more of it the sound adds.
const GLOW = 0.28;
const GLOW_GAIN = 0.32;
/// How dim the bars are when something is wrong.
const TROUBLE_DIM = 0.32;
/// With reduced motion brightness carries the sound: from this, up to full with the level.
const STILL_BASE = 0.85;

const TURN = 2 * Math.PI;

/// A level as given by the voice, read as 0 to 1 whatever it is.
function unit(v: number): number {
  if (Number.isNaN(v)) return 0;
  return Math.min(1, Math.max(0, v));
}

function optionCount(n: number): number {
  if (Number.isNaN(n)) return 1;
  return Math.min(4, Math.max(1, Math.round(n)));
}

/// Where each bar wants to be at time `t` (seconds). A width of 0 means the bar is not part of
/// this shape; `step` lets it shrink where it stands.
function targets(state: BreathState, t: number, level: number, n: number): Body[] {
  /// A sine between -1 and 1 that takes `period` seconds, `lag` of a cycle behind.
  const wave = (period: number, lag = 0) => Math.sin((t / period - lag) * TURN);
  return HOME.map((home, i): Body => {
    if (i === 3 && !(state === "asking" && n === 4)) return { ...home, w: 0 };
    const middle = i === 1;
    switch (state) {
      case "resting": {
        // The outer bars drift out along the S and back every 4.5 s, and the middle bar
        // breathes in width, each a little after the one above.
        const out = 1.5 * (1 + wave(4.5, 0.25 + i * 0.06));
        if (middle) return { ...home, w: home.w * (1 + 0.04 * wave(4.5, 0.06)) };
        return { ...home, x: home.x + (i === 0 ? out : -out) };
      }
      case "done":
      case "trouble":
        return { ...home };
      case "listening":
        // A level meter: centred, each bar as wide as the mic is loud, the middle one most. A
        // slow sine of its own keeps each bar alive in silence and no two alike.
        return {
          x: MIDDLE,
          y: home.y,
          w: middle
            ? 63 + 84 * level + 3 * wave(3, 0)
            : 53 + 54 * level + 3 * wave(i === 0 ? 3.7 : 4.3, i * 0.2),
          h: THICK,
        };
      case "thinking":
        // Pistons: each bar slides from side to side, a third of a cycle after the one above.
        return { x: MIDDLE + 22 * wave(1.6, i / 3), y: home.y, w: 80, h: THICK };
      case "speaking": {
        // The logo, pulsing with Sage's voice: every bar widens, the middle one most, and it
        // swells in height while the outer bars move away to keep their distance from it.
        const away = 6 * level * (i === 0 ? -1 : i === 2 ? 1 : 0);
        return {
          x: home.x,
          y: home.y + away,
          w: home.w + (middle ? 40 : 20) * level + 2 * wave(2.6, i * 0.3),
          h: middle ? THICK * (1 + 0.45 * level) : THICK,
        };
      }
      case "asking": {
        // One bar per option, alike and evenly spaced from the top down. Each nudges sideways a
        // quarter of a cycle after the one above, so the eye runs down the choices.
        // A single option is the middle bar, the one in colour; otherwise from the top.
        const row = n === 1 ? (middle ? 0 : -1) : i < n ? i : -1;
        if (row < 0) return { ...home, w: 0 };
        return {
          x: MIDDLE + 4 * wave(2.4, row * 0.25),
          y: MIDDLE + (row - (n - 1) / 2) * PITCH,
          w: 90,
          h: THICK,
        };
      }
    }
  });
}

function ease(value: number, target: number, k: number): number {
  const next = value + (target - value) * k;
  return Math.abs(target - next) < SNAP ? target : next;
}

/// One Breath's smoothed values. `step` is called once per animation frame with the time.
export function createBreath(): { step(input: BreathInput, nowMs: number): Frame } {
  const bodies: Body[] = HOME.map((h) => ({ ...h }));
  let last: number | null = null;
  let t = 0;
  let mic = 0;
  let voice = 0;
  let dim = 1;

  return {
    step(input, nowMs) {
      let dt = 0;
      if (Number.isFinite(nowMs)) {
        if (last !== null) dt = Math.min(MAX_DT, Math.max(0, (nowMs - last) / 1000));
        last = nowMs;
      }
      // The breath's own clock, so a long pause does not jump its phase.
      t += dt;

      const follow = (value: number, want: number) => {
        const next =
          value + (want - value) * (1 - Math.exp(-dt * (want > value ? ATTACK_RATE : RELEASE_RATE)));
        return next < LEVEL_SNAP ? 0 : next;
      };
      mic = follow(mic, unit(input.mic));
      voice = follow(voice, unit(input.voice));
      const { state } = input;
      const level = state === "listening" ? mic : state === "speaking" ? voice : 0;

      let settled = true;
      if (input.still) {
        // Reduced motion: the logo, always; only brightness moves.
        HOME.forEach((h, i) => Object.assign(bodies[i]!, h));
      } else {
        const k = 1 - Math.exp(-dt * MORPH_RATE);
        targets(state, t, level, optionCount(input.options)).forEach((goal, i) => {
          const b = bodies[i]!;
          // A bar that is leaving shrinks where it stands instead of sliding across the others
          // as it goes.
          const g = goal.w <= 0 ? { ...b, w: 0 } : goal;
          b.x = ease(b.x, g.x, k);
          b.y = ease(b.y, g.y, k);
          b.w = ease(b.w, g.w, k);
          b.h = ease(b.h, g.h, k);
          if (b.x !== g.x || b.y !== g.y || b.w !== g.w || b.h !== g.h) settled = false;
        });
      }

      const wantDim = state === "trouble" ? TROUBLE_DIM : 1;
      dim = ease(dim, wantDim, 1 - Math.exp(-dt * FADE_RATE));
      if (dim !== wantDim || mic !== 0 || voice !== 0) settled = false;
      const opacity = input.still ? dim * (STILL_BASE + (1 - STILL_BASE) * level) : dim;
      const glow = dim * (GLOW + GLOW_GAIN * level);

      const bar = (b: Body): Bar => ({ x: b.x, y: b.y, w: b.w, h: b.h, opacity });
      const fourth = bodies[3]!;
      return {
        bars: [bar(bodies[0]!), bar(bodies[1]!), bar(bodies[2]!)],
        extra: fourth.w > SEEN ? [bar(fourth)] : [],
        glow,
        mic,
        voice,
        settled,
      };
    },
  };
}

/// Which of The Breath's states shows what Open Dot is doing. `justFinished`: a reply ended a
/// moment ago (the caller holds it for `DONE_MS`).
export function breathStateOf(
  live: { state: VoiceState; trouble: string | null; ask: unknown | null },
  justFinished: boolean,
): BreathState {
  if (live.trouble !== null && live.trouble !== "") return "trouble";
  switch (live.state) {
    case "listening":
    case "thinking":
    case "speaking":
    case "asking":
      return live.state;
    case "off":
    case "idle":
      if (live.ask !== null && live.ask !== undefined) return "asking";
      return justFinished ? "done" : "resting";
  }
}
