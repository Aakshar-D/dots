import type { VoiceState } from "@/api/types";

/// The numbers behind The Breath: where the three dots of the Dots mark are, how big and how
/// bright, frame by frame. Ported from the accepted prototype (the-breath.html): the logo
/// arrangement, each state's targets, the level smoothing and the easing. The model never reads
/// a clock: `step` is given the time, so the same inputs always give the same frames.

export type BreathState =
  | "resting"
  | "listening"
  | "thinking"
  | "speaking"
  | "asking"
  | "done"
  | "trouble";
/// One body, in a 200 × 200 box.
export type Dot = { x: number; y: number; r: number; opacity: number };
/// `lobes`: the extra body shown while asking with four or more options, empty otherwise.
/// `mic`, `voice`: the smoothed levels, exactly 0 once too small to see. `settled`: every value
/// is at its target, so with the same input the next frame is this one.
export type Frame = {
  dots: [Dot, Dot, Dot];
  lobes: Dot[];
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

/// The prototype drew in a 400 × 280 box around its middle; its widest shape (four lobes) spans
/// 274 units, so 0.7 fits every shape into the 200 box with room to spare.
const SCALE = 0.7;
const MIDDLE = 100;

/// The logo arrangement in the prototype's units: one large dot left, two smaller right. The
/// fourth body is the extra lobe, seen only when asking with four options.
const HOME = [
  { x: -50, y: 0, r: 35 },
  { x: 50, y: -50, r: 25 },
  { x: 50, y: 50, r: 25 },
  { x: -50, y: 0, r: 0 },
] as const;

const toBox = (h: { x: number; y: number; r: number }) => ({
  x: MIDDLE + h.x * SCALE,
  y: MIDDLE + h.y * SCALE,
  r: h.r * SCALE,
});

/// The logo arrangement in the 200 box, at full brightness.
export const LOGO: readonly [Dot, Dot, Dot] = [
  { ...toBox(HOME[0]), opacity: 1 },
  { ...toBox(HOME[1]), opacity: 1 },
  { ...toBox(HOME[2]), opacity: 1 },
];

/// Easing rate per second: 1 - e^(-3) = 95% of the way after MORPH_MS. The prototype used 9.
const MORPH_RATE = 3000 / MORPH_MS;
/// Brightness eases a little slower (prototype `dim`).
const FADE_RATE = 6;
/// The level rises fast (prototype: 14) and falls back more slowly, so syllables read as a
/// swell rather than a flicker.
const ATTACK_RATE = 14;
const RELEASE_RATE = 7;
/// The longest step one frame may take, in seconds (prototype: 0.05). A frame after the tab was
/// hidden eases like a slow frame instead of cutting to the target.
const MAX_DT = 0.05;
/// Below this a value is at its target.
const SNAP = 1e-3;
/// Below this a smoothed level is silence, so a mark at rest can tell nothing moves.
const LEVEL_SNAP = 1e-4;
/// The glow behind the dots (prototype halo opacity).
const GLOW = 0.28;
/// How dim the dots are when something is wrong (prototype).
const TROUBLE_DIM = 0.32;
/// With reduced motion brightness carries the sound: from this, up to full with the level.
const STILL_BASE = 0.85;
const STILL_GLOW_GAIN = 0.32;

/// A level as given by the voice, read as 0 to 1 whatever it is.
function unit(v: number): number {
  if (Number.isNaN(v)) return 0;
  return Math.min(1, Math.max(0, v));
}

function optionCount(n: number): number {
  if (Number.isNaN(n)) return 1;
  return Math.min(4, Math.max(1, Math.round(n)));
}

type Body = { x: number; y: number; r: number };

/// Where each body wants to be at time `t` (seconds), in the prototype's units
/// (the-breath.html `targets(t)`).
function targets(state: BreathState, t: number, level: number, n: number): Body[] {
  const breath = (i: number, depth: number, period: number) =>
    1 + depth * Math.sin((t / period) * 2 * Math.PI + i * 1.3);
  return HOME.map((h, i) => {
    switch (state) {
      case "resting":
        // A slow swell and settle every 4.5 s, each dot a little out of step.
        return { x: h.x, y: h.y, r: h.r * breath(i, 0.07, 4.5) };
      case "done":
      case "trouble":
        return { x: h.x, y: h.y, r: h.r };
      case "listening":
        // Leaning apart; each swells with the mic, the yellow one most.
        return {
          x: h.x * 1.28,
          y: h.y * 1.28,
          r: i === 3 ? 0 : h.r * breath(i, 0.03, 3) + level * (i === 0 ? 24 : 12 + i * 2),
        };
      case "thinking": {
        // Circling one another, drifting in and out so they touch and part.
        const a = (i * 2 * Math.PI) / 3 + t * 1.1;
        const d = 36 + 14 * Math.sin(t * 1.9 + i * 2.1);
        return {
          x: Math.cos(a) * d,
          y: Math.sin(a) * d * 0.8,
          r: i === 3 ? 0 : 24 + 5 * Math.sin(t * 2.3 + i),
        };
      }
      case "speaking": {
        // Melted into one shape, turning slowly, pulsing with Dot's voice. Far enough apart and
        // close enough in size that each colour keeps part of the outline.
        const a = (i * 2 * Math.PI) / 3 + t * 0.7;
        const d = 18 + level * 9;
        return {
          x: Math.cos(a) * d,
          y: Math.sin(a) * d,
          r: i === 3 ? 0 : h.r * (i === 0 ? 1 : 1.2) + level * 20 + 2 * Math.sin(t * 3 + i),
        };
      }
      case "asking": {
        // One lobe per option, side by side on a gentle arc, bobbing a little.
        const span = 74;
        const slot = n === 1 ? 0 : n === 2 ? [0, 1, 1, 0][i]! : n === 3 ? [0, 1, 2, 0][i]! : i;
        const from = slot - (n - 1) / 2;
        return {
          x: from * span,
          y: -8 * Math.cos((from / n) * Math.PI) + 4 * Math.sin(t * 1.4 + slot),
          r: i === 3 && n < 4 ? 0 : 26,
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
  const bodies: Body[] = HOME.map(toBox);
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
        HOME.forEach((h, i) => Object.assign(bodies[i]!, toBox(h)));
      } else {
        const k = 1 - Math.exp(-dt * MORPH_RATE);
        targets(state, t, level, optionCount(input.options)).forEach((goal, i) => {
          const b = bodies[i]!;
          const g = toBox(goal);
          // The extra lobe shrinks where it stands instead of flying across the others as it
          // goes (the prototype sent it back under the yellow dot).
          if (i === 3 && g.r <= 0) {
            g.x = b.x;
            g.y = b.y;
          }
          b.x = ease(b.x, g.x, k);
          b.y = ease(b.y, g.y, k);
          b.r = ease(b.r, Math.max(0, g.r), k);
          if (b.x !== g.x || b.y !== g.y || b.r !== Math.max(0, g.r)) settled = false;
        });
      }

      const wantDim = state === "trouble" ? TROUBLE_DIM : 1;
      dim = ease(dim, wantDim, 1 - Math.exp(-dt * FADE_RATE));
      if (dim !== wantDim || mic !== 0 || voice !== 0) settled = false;
      const opacity = input.still ? dim * (STILL_BASE + (1 - STILL_BASE) * level) : dim;
      const glow = input.still ? dim * (GLOW + STILL_GLOW_GAIN * level) : dim * GLOW;

      const dot = (b: Body): Dot => ({ x: b.x, y: b.y, r: b.r, opacity });
      const lobe = bodies[3]!;
      return {
        dots: [dot(bodies[0]!), dot(bodies[1]!), dot(bodies[2]!)],
        lobes: lobe.r > 0.05 ? [dot(lobe)] : [],
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
