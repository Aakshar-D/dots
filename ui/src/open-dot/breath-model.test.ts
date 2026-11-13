import { describe, expect, it } from "vitest";

import {
  breathStateOf,
  createBreath,
  LOGO,
  MORPH_MS,
  type BreathInput,
  type BreathState,
  type Dot,
  type Frame,
} from "./breath-model";

const FRAME_MS = 1000 / 60;
const BASE: BreathInput = { state: "resting", mic: 0, voice: 0, options: 3, still: false };
const STATES: BreathState[] = [
  "resting",
  "listening",
  "thinking",
  "speaking",
  "asking",
  "done",
  "trouble",
];

/// A Breath driven by a clock the test holds: one step per 60 Hz frame.
function driver() {
  const breath = createBreath();
  let now = 0;
  return {
    /// Steps for `ms` with `input`, returning every frame.
    run(input: Partial<BreathInput>, ms: number): Frame[] {
      const frames: Frame[] = [];
      const end = now + ms;
      while (now < end - 1e-6) {
        now += FRAME_MS;
        frames.push(breath.step({ ...BASE, ...input }, now));
      }
      return frames;
    },
    /// One step `ms` after the last.
    after(input: Partial<BreathInput>, ms: number): Frame {
      now += ms;
      return breath.step({ ...BASE, ...input }, now);
    },
  };
}

const mean = (xs: number[]) => xs.reduce((a, b) => a + b, 0) / xs.length;
const spread = (xs: number[]) => Math.max(...xs) - Math.min(...xs);
const dist = (a: Dot, b: Dot) => Math.hypot(a.x - b.x, a.y - b.y);
const fromCentre = (d: Dot) => Math.hypot(d.x - 100, d.y - 100);
/// Every body drawn: the three dots and any lobe that has a size.
const bodies = (f: Frame) => [...f.dots, ...f.lobes].filter((d) => d.r > 1);
/// The values that move, for one frame.
const values = (f: Frame) =>
  [...f.dots, ...(f.lobes.length ? f.lobes : [{ x: NaN, y: NaN, r: 0, opacity: 0 }])].flatMap(
    (d) => [d.x, d.y, d.r],
  );

/// Times, in seconds, of a series' local peaks.
function peaks(series: number[]): number[] {
  const out: number[] = [];
  for (let i = 1; i < series.length - 1; i++) {
    if (series[i]! > series[i - 1]! && series[i]! >= series[i + 1]!) {
      out.push((i * FRAME_MS) / 1000);
    }
  }
  return out;
}

describe("The Breath's model", () => {
  it("resting_breathes_about_every_4_5_seconds", () => {
    const d = driver();
    d.run({}, 2000);
    const frames = d.run({}, 20000);
    const times = [0, 1, 2].map((i) => peaks(frames.map((f) => f.dots[i]!.r)));
    for (const t of times) {
      expect(t.length).toBeGreaterThanOrEqual(4);
      const gaps = t.slice(1).map((x, i) => x - t[i]!);
      expect(mean(gaps)).toBeGreaterThan(4.2);
      expect(mean(gaps)).toBeLessThan(4.8);
    }
    // Slightly out of step: each dot peaks at another moment of the breath.
    const offset = (a: number, b: number) => {
      const diff = Math.abs(times[a]![0]! - times[b]![0]!) % 4.5;
      return Math.min(diff, 4.5 - diff);
    };
    expect(offset(0, 1)).toBeGreaterThan(0.4);
    expect(offset(1, 2)).toBeGreaterThan(0.4);
    expect(offset(0, 2)).toBeGreaterThan(0.4);
    // A swell and settle, not a jump: a few percent of the radius.
    expect(spread(frames.map((f) => f.dots[0]!.r))).toBeGreaterThan(LOGO[0]!.r * 0.08);
    expect(spread(frames.map((f) => f.dots[0]!.r))).toBeLessThan(LOGO[0]!.r * 0.16);
    // They sit in the logo's arrangement.
    for (const f of frames) {
      f.dots.forEach((dot, i) => {
        expect(dot.x).toBeCloseTo(LOGO[i]!.x, 6);
        expect(dot.y).toBeCloseTo(LOGO[i]!.y, 6);
      });
    }
  });

  it("listening_swells_with_the_mic_and_yellow_most", () => {
    const d = driver();
    d.run({ state: "listening" }, 2000);
    // A whole number of listening breaths (3 s), so the mean is the size without the breath.
    const quiet = d.run({ state: "listening" }, 3000);
    d.run({ state: "listening", mic: 0.8 }, 1000);
    const loud = d.run({ state: "listening", mic: 0.8 }, 3000);
    const growth = [0, 1, 2].map(
      (i) => mean(loud.map((f) => f.dots[i]!.r)) - mean(quiet.map((f) => f.dots[i]!.r)),
    );
    for (const g of growth) expect(g).toBeGreaterThan(3);
    expect(growth[0]).toBeGreaterThan(growth[1]!);
    expect(growth[0]).toBeGreaterThan(growth[2]!);
    // They lean apart from the logo.
    quiet.at(-1)!.dots.forEach((dot, i) => {
      expect(fromCentre(dot)).toBeGreaterThan(fromCentre(LOGO[i]!) * 1.2);
    });
  });

  it("silence_brings_listening_back_to_a_shallow_breath", () => {
    const d = driver();
    d.run({ state: "listening", mic: 1 }, 2000);
    d.run({ state: "listening", mic: 0 }, 2000);
    const silent = d.run({ state: "listening", mic: 0 }, 3000);
    d.run({ state: "resting" }, 2000);
    const resting = d.run({ state: "resting" }, 4500);
    [0, 1, 2].forEach((i) => {
      const rs = silent.map((f) => f.dots[i]!.r);
      expect(mean(rs)).toBeCloseTo(LOGO[i]!.r, 0);
      // Still breathing, and shallower than at rest.
      expect(spread(rs)).toBeGreaterThan(0.2);
      expect(spread(rs)).toBeLessThan(LOGO[i]!.r * 0.07);
      expect(spread(rs)).toBeLessThan(spread(resting.map((f) => f.dots[i]!.r)));
    });
  });

  it("thinking_circles", () => {
    const d = driver();
    d.run({ state: "thinking" }, 1500);
    const frames = d.run({ state: "thinking" }, 10000);
    // The yellow dot goes all the way round the middle.
    let turned = 0;
    let last = Math.atan2(frames[0]!.dots[0]!.y - 100, frames[0]!.dots[0]!.x - 100);
    for (const f of frames) {
      const a = Math.atan2(f.dots[0]!.y - 100, f.dots[0]!.x - 100);
      let step = a - last;
      if (step > Math.PI) step -= 2 * Math.PI;
      if (step < -Math.PI) step += 2 * Math.PI;
      turned += step;
      last = a;
    }
    expect(Math.abs(turned)).toBeGreaterThan(2 * Math.PI);
    // Every pair touches and parts.
    for (const [a, b] of [
      [0, 1],
      [1, 2],
      [0, 2],
    ] as const) {
      const gaps = frames.map((f) => dist(f.dots[a], f.dots[b]) - f.dots[a].r - f.dots[b].r);
      expect(Math.min(...gaps)).toBeLessThan(0);
      expect(Math.max(...gaps)).toBeGreaterThan(0);
    }
    // They touch for a visible stretch: some pair overlaps for at least 15% of the loop.
    const touching = frames.filter((f) =>
      (
        [
          [0, 1],
          [1, 2],
          [0, 2],
        ] as const
      ).some(([a, b]) => dist(f.dots[a], f.dots[b]) < f.dots[a].r + f.dots[b].r),
    );
    expect(touching.length / frames.length).toBeGreaterThanOrEqual(0.15);
  });

  it("levels_too_small_to_see_are_zero", () => {
    // A level that fades out reaches exactly nothing, so a mark at rest can tell it is still.
    const d = driver();
    const loud = d.run({ state: "listening", mic: 1, voice: 1 }, 500).at(-1)!;
    expect(loud.mic).toBeGreaterThan(0.5);
    expect(loud.voice).toBeGreaterThan(0.5);
    const faded = d.run({ state: "resting" }, 5000).at(-1)!;
    expect(faded.mic).toBe(0);
    expect(faded.voice).toBe(0);
  });

  it("says_when_it_has_settled", () => {
    const d = driver();
    expect(d.run({ state: "thinking" }, 2000).some((f) => f.settled)).toBe(false);
    expect(d.run({ state: "resting" }, 2000).some((f) => f.settled)).toBe(false);
    const done = d.run({ state: "done" }, 3000);
    expect(done[0]!.settled).toBe(false);
    expect(done.at(-1)!.settled).toBe(true);
    const still = d.run({ state: "speaking", still: true }, 3000);
    expect(still.at(-1)!.settled).toBe(true);
    expect(d.run({ state: "speaking", voice: 0.5, still: true }, 100).at(-1)!.settled).toBe(false);
  });

  it("speaking_melts_and_pulses_with_voice", () => {
    const d = driver();
    /// One shape, three colours: every pair overlaps, and no centre lies inside another body's
    /// inner 60%, so each colour owns part of the outline.
    const melted = (f: Frame) => {
      for (const [a, b] of [
        [0, 1],
        [1, 2],
        [0, 2],
      ] as const) {
        const gap = dist(f.dots[a], f.dots[b]);
        expect(gap).toBeLessThan(f.dots[a].r + f.dots[b].r);
        expect(gap).toBeGreaterThan(0.6 * f.dots[a].r);
        expect(gap).toBeGreaterThan(0.6 * f.dots[b].r);
      }
    };
    d.run({ state: "speaking" }, 2000);
    const quiet = d.run({ state: "speaking" }, 3000);
    quiet.forEach(melted);
    d.run({ state: "speaking", voice: 0.9 }, 1000);
    const loud = d.run({ state: "speaking", voice: 0.9 }, 3000);
    loud.forEach(melted);
    for (const v of [0.3, 0.6, 1]) d.run({ state: "speaking", voice: v }, 1500).forEach(melted);
    [0, 1, 2].forEach((i) => {
      expect(mean(loud.map((f) => f.dots[i]!.r)) - mean(quiet.map((f) => f.dots[i]!.r))).toBeGreaterThan(8);
    });
    // It follows the sound syllable by syllable: a short burst lifts it, and it falls back.
    const before = d.run({ state: "speaking", voice: 0 }, 1500).at(-1)!.dots[0].r;
    const burst = d.run({ state: "speaking", voice: 1 }, 200).at(-1)!.dots[0].r;
    const after = d.run({ state: "speaking", voice: 0 }, 1000).at(-1)!.dots[0].r;
    expect(burst - before).toBeGreaterThan(4);
    expect(Math.abs(after - before)).toBeLessThan(3);
  });

  it("asking_parts_into_as_many_lobes_as_options", () => {
    for (const [options, lobes] of [
      [2, 2],
      [3, 3],
      [4, 4],
      [6, 4],
    ] as const) {
      const d = driver();
      d.run({ state: "speaking", voice: 0.4 }, 1000);
      d.run({ state: "asking", options }, 2000);
      const frames = d.run({ state: "asking", options }, 2000);
      const last = frames.at(-1)!;
      const drawn = bodies(last);
      expect(drawn.length).toBeLessThanOrEqual(4);
      const columns: number[] = [];
      for (const b of drawn) {
        if (!columns.some((x) => Math.abs(x - b.x) < 3)) columns.push(b.x);
      }
      expect(columns.length).toBe(lobes);
      // Lobes stand apart from each other.
      columns.sort((a, b) => a - b);
      for (let i = 1; i < columns.length; i++) {
        expect(columns[i]! - columns[i - 1]!).toBeGreaterThan(40);
      }
      // And hold: across two seconds no lobe moves sideways.
      for (let i = 0; i < 3; i++) {
        expect(spread(frames.map((f) => f.dots[i]!.x))).toBeLessThan(0.5);
      }
    }
  });

  it("done_settles_to_the_logo", () => {
    const d = driver();
    d.run({ state: "speaking", voice: 0.6 }, 2000);
    const frames = d.run({ state: "done" }, 600);
    frames.at(-1)!.dots.forEach((dot, i) => {
      expect(Math.abs(dot.x - LOGO[i]!.x)).toBeLessThan(0.5);
      expect(Math.abs(dot.y - LOGO[i]!.y)).toBeLessThan(0.5);
      expect(Math.abs(dot.r - LOGO[i]!.r)).toBeLessThan(0.5);
    });
    // It settles once: no lobe left, and then it is exactly the logo.
    const settled = d.run({ state: "done" }, 1500).at(-1)!;
    expect(settled.lobes).toEqual([]);
    settled.dots.forEach((dot, i) => {
      expect(dot.x).toBe(LOGO[i]!.x);
      expect(dot.y).toBe(LOGO[i]!.y);
      expect(dot.r).toBe(LOGO[i]!.r);
    });
  });

  it("trouble_dims_and_holds_still", () => {
    const d = driver();
    d.run({ state: "listening", mic: 0.7 }, 2000);
    d.run({ state: "trouble", mic: 0.7 }, 2000);
    const frames = d.run({ state: "trouble", mic: 0.7, voice: 0.7 }, 2000);
    for (const f of frames) {
      expect(values(f)).toEqual(values(frames[0]!));
      f.dots.forEach((dot, i) => {
        expect(dot.x).toBe(LOGO[i]!.x);
        expect(dot.y).toBe(LOGO[i]!.y);
        expect(dot.r).toBe(LOGO[i]!.r);
        expect(dot.opacity).toBeLessThan(0.4);
      });
      expect(f.glow).toBeLessThan(0.15);
    }
  });

  it("state_changes_are_continuous", () => {
    // No value moves more than this per 60th of a second: under a tenth of the box, where a
    // cut would move up to the whole distance between two shapes (over 100). The largest is
    // the extra lobe leaving the yellow dot for the far right (about 14).
    const BOUND = 16;
    const input = { mic: 0.6, voice: 0.6, options: 4 };
    for (const fps of [30, 60, 144]) {
      const frameMs = 1000 / fps;
      // Distance moved per frame, scaled to a 60th of a second.
      const scale = FRAME_MS / frameMs;
      let worst = 0;
      const check = (frames: Frame[], prev: Frame) => {
        for (const f of frames) {
          const a = values(prev);
          const b = values(f);
          for (let i = 0; i < a.length; i++) {
            if (Number.isNaN(a[i]!) || Number.isNaN(b[i]!)) continue;
            worst = Math.max(worst, Math.abs(b[i]! - a[i]!) * scale);
          }
          prev = f;
        }
        return prev;
      };
      const run = (breath: ReturnType<typeof createBreath>, clock: { now: number }, state: BreathState, ms: number) => {
        const frames: Frame[] = [];
        const end = clock.now + ms;
        while (clock.now < end - 1e-6) {
          clock.now += frameMs;
          frames.push(breath.step({ ...BASE, ...input, state }, clock.now));
        }
        return frames;
      };
      for (const from of STATES) {
        for (const to of STATES) {
          if (from === to) continue;
          const breath = createBreath();
          const clock = { now: 0 };
          const prev = run(breath, clock, from, 1500).at(-1)!;
          check(run(breath, clock, to, 1000), prev);
          // And a switch 100 ms into a morph, to a third state.
          for (const third of STATES) {
            if (third === to) continue;
            const b2 = createBreath();
            const c2 = { now: 0 };
            let p = run(b2, c2, from, 1500).at(-1)!;
            p = check(run(b2, c2, to, 100), p);
            check(run(b2, c2, third, 600), p);
          }
        }
      }
      expect(worst).toBeGreaterThan(1);
      expect(worst).toBeLessThan(BOUND);
    }
  });

  it("a_frame_after_a_long_pause_does_not_cut", () => {
    // The tab was hidden for ten seconds: the next frame still eases.
    const d = driver();
    const start = d.run({ state: "resting" }, 1000).at(-1)!;
    const next = d.after({ state: "asking", options: 4 }, 10_000);
    const goal = d.run({ state: "asking", options: 4 }, 3000).at(-1)!;
    const moved = dist(next.dots[0], start.dots[0]);
    const whole = dist(goal.dots[0], start.dots[0]);
    expect(whole).toBeGreaterThan(20);
    expect(moved).toBeLessThan(whole / 2);
  });

  it("morphs_over_about_morph_ms", () => {
    expect(MORPH_MS).toBeGreaterThanOrEqual(250);
    expect(MORPH_MS).toBeLessThanOrEqual(400);
    const d = driver();
    const start = d.run({ state: "resting" }, 1000).at(-1)!;
    const frames = d.run({ state: "asking", options: 2 }, 2000);
    const goal = frames.at(-1)!;
    const at = (ms: number) => frames[Math.round(ms / FRAME_MS) - 1]!;
    const left = (f: Frame) => Math.abs(goal.dots[1].x - f.dots[1].x) / Math.abs(goal.dots[1].x - start.dots[1].x);
    // Most of the way after MORPH_MS, and not before half of it.
    expect(left(at(MORPH_MS))).toBeLessThan(0.1);
    expect(left(at(MORPH_MS / 4))).toBeGreaterThan(0.3);
  });

  it("reduced_motion_only_changes_brightness", () => {
    const d = driver();
    const opacities: number[] = [];
    const glows: number[] = [];
    for (const state of [...STATES, ...STATES]) {
      for (const [mic, voice] of [
        [0, 0],
        [1, 0.3],
        [0.2, 1],
      ] as const) {
        for (const f of d.run({ state, mic, voice, options: 4, still: true }, 400)) {
          f.dots.forEach((dot, i) => {
            expect(dot.x).toBe(LOGO[i]!.x);
            expect(dot.y).toBe(LOGO[i]!.y);
            expect(dot.r).toBe(LOGO[i]!.r);
          });
          expect(f.lobes).toEqual([]);
          opacities.push(f.dots[0].opacity);
          glows.push(f.glow);
        }
      }
    }
    // Brightness is what carries the state.
    expect(spread(opacities)).toBeGreaterThan(0.3);
    expect(spread(glows)).toBeGreaterThan(0.1);
    // Listening brightens with the mic.
    const l = driver();
    const silent = l.run({ state: "listening", still: true }, 1500).at(-1)!;
    const loud = l.run({ state: "listening", mic: 1, still: true }, 1500).at(-1)!;
    expect(loud.glow).toBeGreaterThan(silent.glow);
    expect(loud.dots[0].opacity).toBeGreaterThan(silent.dots[0].opacity);
    // Speaking with Dot's voice.
    const s = driver();
    const hush = s.run({ state: "speaking", still: true }, 1500).at(-1)!;
    const said = s.run({ state: "speaking", voice: 1, still: true }, 1500).at(-1)!;
    expect(said.glow).toBeGreaterThan(hush.glow);
  });

  it("level_is_smoothed", () => {
    const d = driver();
    const before = d.run({ state: "listening" }, 3000).at(-1)!.dots[0].r;
    // One frame of a loud click.
    const spike = d.run({ state: "listening", mic: 1 }, FRAME_MS)[0]!.dots[0].r;
    const after = d.run({ state: "listening" }, 500);
    const full = 24 * 0.7; // the yellow dot's whole swell
    expect(Math.abs(spike - before)).toBeLessThan(2);
    expect(Math.max(...after.map((f) => f.dots[0].r)) - before).toBeLessThan(full / 3);
    // Fast to rise, slower to fall.
    const rise = driver();
    rise.run({ state: "listening" }, 3000);
    const base = rise.run({ state: "listening" }, FRAME_MS)[0]!.dots[0].r;
    const up = rise.run({ state: "listening", mic: 1 }, 2000);
    const top = up.at(-1)!.dots[0].r;
    const down = rise.run({ state: "listening", mic: 0 }, 2000);
    const half = (base + top) / 2;
    const upAt = up.findIndex((f) => f.dots[0].r > half);
    const downAt = down.findIndex((f) => f.dots[0].r < half);
    expect(upAt).toBeGreaterThan(0);
    expect(downAt).toBeGreaterThan(upAt);
  });

  it("survives_bad_input", () => {
    const same = (a: Partial<BreathInput>, b: Partial<BreathInput>) => {
      const x = driver();
      const y = driver();
      for (const state of STATES) {
        const fx = x.run({ ...a, state }, 300);
        const fy = y.run({ ...b, state }, 300);
        expect(fx).toEqual(fy);
        for (const f of fx) {
          for (const v of [...values(f).filter((n) => !Number.isNaN(n)), f.glow, f.dots[0].opacity]) {
            expect(Number.isFinite(v)).toBe(true);
          }
        }
      }
    };
    same({ mic: NaN, voice: NaN }, { mic: 0, voice: 0 });
    same({ mic: -5, voice: -0.1 }, { mic: 0, voice: 0 });
    same({ mic: 7, voice: Infinity }, { mic: 1, voice: 1 });
    same({ mic: -Infinity, voice: 1.5 }, { mic: 0, voice: 1 });
    same({ options: NaN }, { options: 1 });
    same({ options: 99 }, { options: 4 });
    // A clock that is missing or runs backwards does not break it.
    const breath = createBreath();
    for (const now of [100, NaN, 50, Infinity, 200, -10]) {
      const f = breath.step({ ...BASE, state: "thinking" }, now);
      for (const v of values(f).filter((n) => !Number.isNaN(n))) expect(Number.isFinite(v)).toBe(true);
    }
  });
});

describe("breathStateOf", () => {
  const live = (state: Parameters<typeof breathStateOf>[0]["state"], extra = {}) => ({
    state,
    trouble: null,
    ask: null,
    ...extra,
  });

  it("maps_what_open_dot_is_doing", () => {
    expect(breathStateOf(live("off"), false)).toBe("resting");
    expect(breathStateOf(live("idle"), false)).toBe("resting");
    expect(breathStateOf(live("idle"), true)).toBe("done");
    expect(breathStateOf(live("off"), true)).toBe("done");
    expect(breathStateOf(live("listening"), true)).toBe("listening");
    expect(breathStateOf(live("thinking"), false)).toBe("thinking");
    expect(breathStateOf(live("speaking"), false)).toBe("speaking");
    expect(breathStateOf(live("asking"), false)).toBe("asking");
    expect(breathStateOf(live("idle", { ask: { question: "?", options: [], seq: 1 } }), true)).toBe("asking");
    for (const state of ["off", "idle", "listening", "speaking"] as const) {
      expect(breathStateOf(live(state, { trouble: "No microphone" }), true)).toBe("trouble");
    }
  });
});
