import { describe, expect, it } from "vitest";

import {
  breathStateOf,
  createBreath,
  LOGO,
  MORPH_MS,
  type Bar,
  type BreathInput,
  type BreathState,
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
/// Every bar drawn: the three, and the fourth while it has a width.
const drawn = (f: Frame) => [...f.bars, ...f.extra].filter((b) => b.w > 1);
/// The values that move, for one frame. A fourth bar that is away has no place to compare.
const values = (f: Frame) =>
  [...f.bars, ...(f.extra.length ? f.extra : [{ x: NaN, y: NaN, w: 0, h: NaN }])].flatMap((b) => [
    b.x,
    b.y,
    b.w,
    b.h,
  ]);
const sameShape = (a: Bar, b: { x: number; y: number; w: number; h: number }) => {
  expect(a.x).toBe(b.x);
  expect(a.y).toBe(b.y);
  expect(a.w).toBe(b.w);
  expect(a.h).toBe(b.h);
};
/// The largest change of any value from one frame to the next, starting after `prev`.
function biggestStep(frames: Frame[], prev: Frame): number {
  let worst = 0;
  for (const f of frames) {
    const a = values(prev);
    const b = values(f);
    for (let i = 0; i < a.length; i++) {
      if (Number.isNaN(a[i]!) || Number.isNaN(b[i]!)) continue;
      worst = Math.max(worst, Math.abs(b[i]! - a[i]!));
    }
    prev = f;
  }
  return worst;
}

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
  it("the_logo_steps_like_an_s", () => {
    const [top, middle, bottom] = LOGO;
    // Stacked top to bottom, the top bar right of the middle one and the bottom bar left.
    expect(top.y).toBeLessThan(middle.y);
    expect(middle.y).toBeLessThan(bottom.y);
    expect(top.x).toBeGreaterThan(middle.x);
    expect(bottom.x).toBeLessThan(middle.x);
    expect(middle.x - bottom.x).toBe(top.x - middle.x);
    expect(middle.y - top.y).toBe(bottom.y - middle.y);
    // The middle bar is the longest and sits in the middle of the box.
    expect([middle.x, middle.y]).toEqual([100, 100]);
    expect(middle.w).toBeGreaterThan(top.w);
    expect(top.w).toBe(bottom.w);
    // Pills that do not touch: wider than tall, with room between them.
    for (const bar of LOGO) expect(bar.w).toBeGreaterThan(bar.h * 2);
    expect(middle.y - top.y).toBeGreaterThan((middle.h + top.h) / 2);
    // A new Breath starts there.
    const first = createBreath().step({ ...BASE, state: "done" }, 0);
    first.bars.forEach((bar, i) => sameShape(bar, LOGO[i]!));
    expect(first.extra).toEqual([]);
  });

  it("resting_drifts_about_every_4_5_seconds", () => {
    const d = driver();
    d.run({}, 2000);
    const frames = d.run({}, 20000);
    const series = [
      frames.map((f) => f.bars[0].x),
      frames.map((f) => f.bars[1].w),
      frames.map((f) => -f.bars[2].x),
    ];
    const times = series.map(peaks);
    for (const t of times) {
      expect(t.length).toBeGreaterThanOrEqual(4);
      const gaps = t.slice(1).map((x, i) => x - t[i]!);
      expect(mean(gaps)).toBeGreaterThan(4.2);
      expect(mean(gaps)).toBeLessThan(4.8);
    }
    // Slightly out of step: each bar is at its farthest at another moment.
    const offset = (a: number, b: number) => {
      const diff = Math.abs(times[a]![0]! - times[b]![0]!) % 4.5;
      return Math.min(diff, 4.5 - diff);
    };
    expect(offset(0, 1)).toBeGreaterThan(0.2);
    expect(offset(0, 2)).toBeGreaterThan(0.2);
    expect(offset(1, 2)).toBeGreaterThan(0.2);
    for (const f of frames) {
      // The top bar goes right of its place and back, the bottom bar left and back: about 3.
      expect(f.bars[0].x).toBeGreaterThanOrEqual(LOGO[0].x);
      expect(f.bars[0].x).toBeLessThan(LOGO[0].x + 3.2);
      expect(f.bars[2].x).toBeLessThanOrEqual(LOGO[2].x);
      expect(f.bars[2].x).toBeGreaterThan(LOGO[2].x - 3.2);
      // Nothing else moves: the rows, the heights, the outer widths, the middle's place.
      f.bars.forEach((bar, i) => {
        expect(bar.y).toBe(LOGO[i]!.y);
        expect(bar.h).toBe(LOGO[i]!.h);
      });
      expect(f.bars[0].w).toBe(LOGO[0].w);
      expect(f.bars[2].w).toBe(LOGO[2].w);
      expect(f.bars[1].x).toBe(LOGO[1].x);
    }
    expect(spread(series[0]!)).toBeGreaterThan(2.5);
    expect(spread(series[2]!)).toBeGreaterThan(2.5);
    // The middle bar breathes by about 4% either way.
    expect(spread(series[1]!)).toBeGreaterThan(LOGO[1].w * 0.06);
    expect(spread(series[1]!)).toBeLessThan(LOGO[1].w * 0.09);
  });

  it("listening_is_a_level_meter_and_the_middle_bar_is_widest", () => {
    const d = driver();
    d.run({ state: "listening" }, 2000);
    const quiet = d.run({ state: "listening" }, 6000);
    d.run({ state: "listening", mic: 0.8 }, 1000);
    const loud = d.run({ state: "listening", mic: 0.8 }, 6000);
    const growth = ([0, 1, 2] as const).map(
      (i) => mean(loud.map((f) => f.bars[i].w)) - mean(quiet.map((f) => f.bars[i].w)),
    );
    for (const g of growth) expect(g).toBeGreaterThan(30);
    expect(growth[1]).toBeGreaterThan(growth[0]!);
    expect(growth[1]).toBeGreaterThan(growth[2]!);
    for (const f of [...quiet, ...loud]) {
      // Centred: the S straightens into a column.
      for (const bar of f.bars) expect(bar.x).toBeCloseTo(100, 6);
      expect(f.bars[1].w).toBeGreaterThan(f.bars[0].w);
      expect(f.bars[1].w).toBeGreaterThan(f.bars[2].w);
      // No two alike.
      expect(f.bars[0].w).not.toBe(f.bars[2].w);
    }
    // The widths rise with the mic all the way: louder is always wider.
    const widths = [0, 0.25, 0.5, 0.75, 1].map((mic) => {
      const m = driver();
      m.run({ state: "listening", mic }, 2000);
      const frames = m.run({ state: "listening", mic }, 6000);
      return ([0, 1, 2] as const).map((i) => mean(frames.map((f) => f.bars[i].w)));
    });
    for (let step = 1; step < widths.length; step++) {
      for (const i of [0, 1, 2]) expect(widths[step]![i]!).toBeGreaterThan(widths[step - 1]![i]! + 8);
    }
    // From about 60 to 150 for the middle bar, 50 to 110 for the others.
    expect(widths[0]![1]!).toBeGreaterThan(58);
    expect(widths[0]![1]!).toBeLessThan(68);
    expect(widths[4]![1]!).toBeGreaterThan(142);
    expect(widths[4]![1]!).toBeLessThanOrEqual(150);
    for (const i of [0, 2]) {
      expect(widths[0]![i]!).toBeGreaterThan(48);
      expect(widths[4]![i]!).toBeLessThanOrEqual(110);
      expect(widths[4]![i]!).toBeGreaterThan(102);
    }
  });

  it("silence_while_listening_is_calm_not_dead", () => {
    const d = driver();
    d.run({ state: "listening", mic: 1 }, 2000);
    d.run({ state: "listening", mic: 0 }, 2000);
    const silent = d.run({ state: "listening", mic: 0 }, 9000);
    for (const i of [0, 1, 2] as const) {
      const ws = silent.map((f) => f.bars[i].w);
      // Still moving, and only a little.
      expect(spread(ws)).toBeGreaterThan(3);
      expect(spread(ws)).toBeLessThan(7);
    }
    expect(silent.some((f) => f.settled)).toBe(false);
  });

  it("thinking_slides_the_bars_out_of_phase_like_pistons", () => {
    const d = driver();
    d.run({ state: "thinking" }, 1500);
    const frames = d.run({ state: "thinking" }, 9600);
    const xs = ([0, 1, 2] as const).map((i) => frames.map((f) => f.bars[i].x));
    const times = xs.map(peaks);
    for (const i of [0, 1, 2]) {
      // About 22 either side of the middle, once every 1.6 s.
      expect(Math.max(...xs[i]!)).toBeGreaterThan(100 + 19);
      expect(Math.max(...xs[i]!)).toBeLessThan(100 + 23);
      expect(Math.min(...xs[i]!)).toBeLessThan(100 - 19);
      expect(Math.min(...xs[i]!)).toBeGreaterThan(100 - 23);
      const gaps = times[i]!.slice(1).map((x, n) => x - times[i]![n]!);
      expect(mean(gaps)).toBeGreaterThan(1.5);
      expect(mean(gaps)).toBeLessThan(1.7);
    }
    // A third of a cycle apart, so the three are never side by side: their offsets cancel.
    const lag = (a: number, b: number) => {
      const diff = Math.abs(times[a]![0]! - times[b]![0]!) % 1.6;
      return Math.min(diff, 1.6 - diff);
    };
    for (const [a, b] of [
      [0, 1],
      [1, 2],
      [0, 2],
    ] as const) {
      expect(lag(a, b)).toBeGreaterThan(0.45);
      expect(lag(a, b)).toBeLessThan(0.62);
    }
    for (const f of frames) {
      expect(Math.abs(f.bars[0].x + f.bars[1].x + f.bars[2].x - 300)).toBeLessThan(0.5);
      // Only the slide: widths, heights and rows hold.
      f.bars.forEach((bar, i) => {
        expect(bar.w).toBeCloseTo(80, 1);
        expect(bar.h).toBe(LOGO[i]!.h);
        expect(bar.y).toBe(LOGO[i]!.y);
      });
    }
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
    // Settled means the next frame is this one.
    expect(d.run({ state: "done" }, 500).at(-1)).toEqual(done.at(-1));
    // Trouble holds still too, once it has dimmed.
    const trouble = d.run({ state: "trouble" }, 3000);
    expect(trouble[0]!.settled).toBe(false);
    expect(trouble.at(-1)!.settled).toBe(true);
    const still = d.run({ state: "speaking", still: true }, 3000);
    expect(still.at(-1)!.settled).toBe(true);
    // Sound coming in, or fading out, is movement.
    expect(d.run({ state: "speaking", voice: 0.5, still: true }, 100).at(-1)!.settled).toBe(false);
    expect(d.run({ state: "done", mic: 0.5 }, 3000).at(-1)!.settled).toBe(false);
  });

  it("speaking_swells_the_middle_bar_with_the_voice", () => {
    const d = driver();
    d.run({ state: "speaking" }, 2000);
    const quiet = d.run({ state: "speaking" }, 5200);
    d.run({ state: "speaking", voice: 1 }, 1000);
    const loud = d.run({ state: "speaking", voice: 1 }, 5200);
    // The middle bar grows taller, up to about 1.45 times, and the others keep their height.
    for (const f of quiet) expect(f.bars[1].h).toBeCloseTo(LOGO[1].h, 1);
    for (const f of loud) {
      expect(f.bars[1].h / LOGO[1].h).toBeGreaterThan(1.4);
      expect(f.bars[1].h / LOGO[1].h).toBeLessThanOrEqual(1.45);
      expect(f.bars[0].h).toBe(LOGO[0].h);
      expect(f.bars[2].h).toBe(LOGO[2].h);
    }
    // Every bar widens, the middle one most.
    const growth = ([0, 1, 2] as const).map(
      (i) => mean(loud.map((f) => f.bars[i].w)) - mean(quiet.map((f) => f.bars[i].w)),
    );
    for (const g of growth) expect(g).toBeGreaterThan(15);
    expect(growth[1]).toBeGreaterThan(growth[0]! + 10);
    expect(growth[1]).toBeGreaterThan(growth[2]! + 10);
    // The outer bars move away from the middle one, so the taller bar never touches them.
    expect(mean(loud.map((f) => f.bars[0].y))).toBeLessThan(LOGO[0].y - 4);
    expect(mean(loud.map((f) => f.bars[2].y))).toBeGreaterThan(LOGO[2].y + 4);
    for (const f of [...quiet, ...loud]) {
      expect(f.bars[1].y - f.bars[0].y).toBeGreaterThan((f.bars[1].h + f.bars[0].h) / 2 + 3);
      expect(f.bars[2].y - f.bars[1].y).toBeGreaterThan((f.bars[1].h + f.bars[2].h) / 2 + 3);
      // It keeps the S.
      expect(f.bars[0].x).toBeGreaterThan(f.bars[1].x);
      expect(f.bars[2].x).toBeLessThan(f.bars[1].x);
    }
    // In between it is in between.
    const half = driver();
    half.run({ state: "speaking", voice: 0.5 }, 2000);
    const mid = half.run({ state: "speaking", voice: 0.5 }, 100).at(-1)!;
    expect(mid.bars[1].h / LOGO[1].h).toBeGreaterThan(1.15);
    expect(mid.bars[1].h / LOGO[1].h).toBeLessThan(1.3);
  });

  it("a_burst_of_voice_lifts_it_and_it_falls_back_smoothly", () => {
    const d = driver();
    const before = d.run({ state: "speaking", voice: 0 }, 2000).at(-1)!.bars[1].h;
    const burst = d.run({ state: "speaking", voice: 1 }, 200);
    const decay = d.run({ state: "speaking", voice: 0 }, 2000);
    expect(burst.at(-1)!.bars[1].h - before).toBeGreaterThan(4);
    expect(Math.abs(decay.at(-1)!.bars[1].h - before)).toBeLessThan(0.1);
    // It carries on up for a moment, turns once, and then only falls: no bounce.
    const hs = [...burst, ...decay].map((f) => f.bars[1].h);
    const top = hs.indexOf(Math.max(...hs));
    for (let i = 1; i < hs.length; i++) {
      if (i <= top) expect(hs[i]!).toBeGreaterThanOrEqual(hs[i - 1]!);
      else expect(hs[i]!).toBeLessThanOrEqual(hs[i - 1]!);
    }
    // And never below where it started.
    expect(Math.min(...hs)).toBeGreaterThanOrEqual(before);
  });

  it("asking_shows_as_many_bars_as_options", () => {
    for (const [options, count] of [
      [1, 1],
      [2, 2],
      [3, 3],
      [4, 4],
      [6, 4],
    ] as const) {
      const d = driver();
      d.run({ state: "speaking", voice: 0.4 }, 1000);
      d.run({ state: "asking", options }, 2000);
      const frames = d.run({ state: "asking", options }, 4800);
      for (const f of frames) {
        const shown = drawn(f);
        expect(shown).toHaveLength(count);
        // Alike, and evenly spaced from the top down around the middle.
        for (const bar of shown) {
          expect(bar.w).toBeCloseTo(90, 1);
          expect(bar.h).toBeCloseTo(25, 1);
        }
        const rows = shown.map((b) => b.y);
        for (let i = 1; i < rows.length; i++) expect(rows[i]! - rows[i - 1]!).toBeCloseTo(35, 1);
        expect(mean(rows)).toBeCloseTo(100, 1);
        // A bar that is not an option has shrunk to nothing.
        expect([...f.bars, ...f.extra].filter((b) => b.w <= 1).every((b) => b.w === 0)).toBe(true);
        expect(f.extra).toHaveLength(count === 4 ? 1 : 0);
      }
      // Each slides a few units sideways, one after another down the list.
      const sliding = ([0, 1, 2] as const).filter((i) => frames[0]!.bars[i].w > 1);
      const times = sliding.map((i) => {
        const xs = frames.map((f) => f.bars[i].x);
        expect(spread(xs)).toBeGreaterThan(6);
        expect(spread(xs)).toBeLessThan(9);
        return peaks(xs)[0]!;
      });
      for (let i = 1; i < times.length; i++) {
        const behind = (((times[i]! - times[i - 1]!) % 2.4) + 2.4) % 2.4;
        expect(behind).toBeGreaterThan(0.4);
        expect(behind).toBeLessThan(0.8);
      }
    }
  });

  it("a_bar_that_is_not_an_option_shrinks_where_it_stands", () => {
    const d = driver();
    const start = d.run({ state: "done" }, 1000).at(-1)!;
    const frames = d.run({ state: "asking", options: 1 }, 2000);
    for (const f of frames) {
      for (const i of [0, 2] as const) {
        expect(f.bars[i].x).toBe(start.bars[i].x);
        expect(f.bars[i].y).toBe(start.bars[i].y);
      }
    }
    expect(frames.at(-1)!.bars[0].w).toBe(0);
    expect(frames.at(-1)!.bars[2].w).toBe(0);
    expect(frames.at(-1)!.bars[1].w).toBeCloseTo(90, 1);
  });

  it("done_settles_to_the_logo", () => {
    const d = driver();
    d.run({ state: "asking", options: 4 }, 2000);
    d.run({ state: "speaking", voice: 0.6 }, 2000);
    const frames = d.run({ state: "done" }, 600);
    frames.at(-1)!.bars.forEach((bar, i) => {
      expect(Math.abs(bar.x - LOGO[i]!.x)).toBeLessThan(0.5);
      expect(Math.abs(bar.y - LOGO[i]!.y)).toBeLessThan(0.5);
      expect(Math.abs(bar.w - LOGO[i]!.w)).toBeLessThan(0.5);
      expect(Math.abs(bar.h - LOGO[i]!.h)).toBeLessThan(0.5);
    });
    // It settles once: no fourth bar left, and then it is exactly the logo.
    const settled = d.run({ state: "done" }, 1500).at(-1)!;
    expect(settled.extra).toEqual([]);
    settled.bars.forEach((bar, i) => sameShape(bar, LOGO[i]!));
    expect(settled.bars[0].opacity).toBe(1);
  });

  it("trouble_dims_and_holds_still", () => {
    const d = driver();
    d.run({ state: "listening", mic: 0.7 }, 2000);
    d.run({ state: "trouble", mic: 0.7 }, 2000);
    const frames = d.run({ state: "trouble", mic: 0.7, voice: 0.7 }, 2000);
    for (const f of frames) {
      expect(values(f)).toEqual(values(frames[0]!));
      f.bars.forEach((bar, i) => {
        sameShape(bar, LOGO[i]!);
        expect(bar.opacity).toBeCloseTo(0.32, 2);
      });
      expect(f.glow).toBeLessThan(0.15);
    }
  });

  it("no_state_moves_more_than_a_little_between_frames", () => {
    // The smoothness guard. Within a state, at 60 frames a second, no centre, width or height
    // changes by more than this from one frame to the next, even when the sound goes from
    // silence to full and back several times a second. The largest is the middle bar's width
    // while listening (about 6, so 3 at each end); the pistons move about 1.5.
    const BOUND = 7;
    for (const state of STATES) {
      for (const options of [1, 2, 3, 4]) {
        const d = driver();
        let prev = d.run({ state, options }, 1500).at(-1)!;
        let worst = 0;
        const watch = (frames: Frame[]) => {
          worst = Math.max(worst, biggestStep(frames, prev));
          prev = frames.at(-1)!;
        };
        watch(d.run({ state, options }, 5000));
        for (let burst = 0; burst < 12; burst++) {
          watch(d.run({ state, options, mic: 1, voice: 1 }, 90 + burst * 20));
          watch(d.run({ state, options, mic: 0, voice: 0 }, 60 + burst * 25));
        }
        expect(worst).toBeLessThan(BOUND);
        // And a state that is meant to move does.
        if (state !== "done" && state !== "trouble") expect(worst).toBeGreaterThan(0.05);
      }
    }
  });

  it("state_changes_are_continuous", () => {
    // No value moves more than this per 60th of a second on the way from one state to another:
    // about what the level meter moves within a state. A cut would move a width by up to 110
    // at once. The fastest is an outer bar of a loud listening shrinking away (110 to 0), a
    // little way into the change.
    const BOUND = 8;
    const input = { mic: 1, voice: 1, options: 4 };
    for (const fps of [30, 60, 144]) {
      const frameMs = 1000 / fps;
      // Distance moved per frame, scaled to a 60th of a second.
      const scale = FRAME_MS / frameMs;
      let worst = 0;
      const check = (frames: Frame[], prev: Frame) => {
        worst = Math.max(worst, biggestStep(frames, prev) * scale);
        return frames.at(-1)!;
      };
      const run = (
        breath: ReturnType<typeof createBreath>,
        clock: { now: number },
        state: BreathState,
        options: number,
        ms: number,
      ) => {
        const frames: Frame[] = [];
        const end = clock.now + ms;
        while (clock.now < end - 1e-6) {
          clock.now += frameMs;
          frames.push(breath.step({ ...BASE, ...input, state, options }, clock.now));
        }
        return frames;
      };
      for (const from of STATES) {
        for (const to of STATES) {
          if (from === to) continue;
          // Asking for one thing takes the most bars away at once.
          for (const options of [1, 4]) {
            const breath = createBreath();
            const clock = { now: 0 };
            const prev = run(breath, clock, from, 4, 1500).at(-1)!;
            check(run(breath, clock, to, options, 1000), prev);
          }
          // And a switch 100 ms into a morph, to a third state.
          for (const third of STATES) {
            if (third === to) continue;
            const b2 = createBreath();
            const c2 = { now: 0 };
            let p = run(b2, c2, from, 4, 1500).at(-1)!;
            p = check(run(b2, c2, to, 4, 100), p);
            check(run(b2, c2, third, 1, 600), p);
          }
        }
      }
      expect(worst).toBeGreaterThan(1);
      expect(worst).toBeLessThan(BOUND);
    }
  });

  it("a_change_of_state_never_overshoots", () => {
    // From a loud listening to the logo: every value goes straight to its place and stops,
    // never past it.
    const d = driver();
    const start = d.run({ state: "listening", mic: 1 }, 2000).at(-1)!;
    const frames = [start, ...d.run({ state: "done", mic: 1 }, 2000)];
    const pick = (f: Frame) => f.bars.flatMap((b) => [b.x, b.y, b.w, b.h]);
    const goal = LOGO.flatMap((b) => [b.x, b.y, b.w, b.h]);
    const from = pick(start);
    for (let n = 1; n < frames.length; n++) {
      const was = pick(frames[n - 1]!);
      const is = pick(frames[n]!);
      is.forEach((v, i) => {
        const side = Math.sign(from[i]! - goal[i]!);
        // Still on the side it started from. It may carry on the way it was going for a moment,
        // by less than a unit; once it has turned toward its place it does not turn back.
        expect((v - goal[i]!) * side).toBeGreaterThanOrEqual(0);
        expect(Math.abs(v - goal[i]!)).toBeLessThanOrEqual(Math.abs(from[i]! - goal[i]!) + 1);
        if (n > 6) {
          expect(Math.abs(v - goal[i]!)).toBeLessThanOrEqual(Math.abs(was[i]! - goal[i]!));
        }
      });
    }
    expect(pick(frames.at(-1)!)).toEqual(goal);
  });

  it("a_frame_after_a_long_pause_does_not_cut", () => {
    // The tab was hidden for ten seconds: the next frame still eases.
    const d = driver();
    const start = d.run({ state: "done" }, 1000).at(-1)!;
    const next = d.after({ state: "asking", options: 4 }, 10_000);
    const goal = d.run({ state: "asking", options: 4 }, 3000).at(-1)!;
    const moved = Math.abs(next.bars[0].y - start.bars[0].y);
    const whole = Math.abs(goal.bars[0].y - start.bars[0].y);
    expect(whole).toBeGreaterThan(15);
    expect(moved).toBeGreaterThan(0);
    expect(moved).toBeLessThan(whole / 2);
  });

  it("morphs_over_about_morph_ms", () => {
    expect(MORPH_MS).toBeGreaterThanOrEqual(320);
    expect(MORPH_MS).toBeLessThanOrEqual(420);
    const d = driver();
    const start = d.run({ state: "done" }, 1000).at(-1)!;
    const frames = d.run({ state: "asking", options: 2 }, 2000);
    const goal = frames.at(-1)!;
    const at = (ms: number) => frames[Math.round(ms / FRAME_MS) - 1]!;
    // The top bar's row does not sway while asking, so it measures the morph alone.
    const left = (f: Frame) =>
      Math.abs(goal.bars[0].y - f.bars[0].y) / Math.abs(goal.bars[0].y - start.bars[0].y);
    // Within 5% after MORPH_MS, and not before a quarter of it.
    expect(left(at(MORPH_MS))).toBeLessThan(0.06);
    expect(left(at(MORPH_MS / 4))).toBeGreaterThan(0.3);
  });

  it("a_change_of_state_starts_gently", () => {
    // No kick: a bar sets off slowly, is fastest part of the way there, and slows to a stop.
    // Its speed never changes by much from one frame to the next.
    const d = driver();
    const start = d.run({ state: "done" }, 1000).at(-1)!;
    const frames = [start, ...d.run({ state: "asking", options: 1 }, 2000)];
    const steps = frames.slice(1).map((f, i) => frames[i]!.bars[0].w - f.bars[0].w);
    const fastest = Math.max(...steps);
    expect(steps[0]!).toBeLessThan(fastest / 4);
    expect(steps.indexOf(fastest)).toBeGreaterThan(2);
    for (let i = 1; i < steps.length; i++) {
      expect(Math.abs(steps[i]! - steps[i - 1]!)).toBeLessThan(fastest / 2);
    }
    expect(frames.at(-1)!.bars[0].w).toBe(0);
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
          f.bars.forEach((bar, i) => sameShape(bar, LOGO[i]!));
          expect(f.extra).toEqual([]);
          opacities.push(f.bars[0].opacity);
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
    expect(loud.bars[0].opacity).toBeGreaterThan(silent.bars[0].opacity);
    // Speaking with Sage's voice.
    const s = driver();
    const hush = s.run({ state: "speaking", still: true }, 1500).at(-1)!;
    const said = s.run({ state: "speaking", voice: 1, still: true }, 1500).at(-1)!;
    expect(said.glow).toBeGreaterThan(hush.glow);
  });

  it("level_is_smoothed", () => {
    // Read from the levels themselves, which carry no sine of their own.
    const d = driver();
    d.run({ state: "listening" }, 1000);
    // One frame of a loud click barely shows, and fades.
    const spike = d.run({ state: "listening", mic: 1 }, FRAME_MS)[0]!;
    expect(spike.mic).toBeGreaterThan(0);
    expect(spike.mic).toBeLessThan(0.25);
    const after = d.run({ state: "listening" }, 500);
    expect(Math.max(...after.map((f) => f.mic))).toBeLessThan(spike.mic);
    // And the bar it drives moves less still: it eases toward the level.
    const calm = driver();
    const before = calm.run({ state: "listening" }, 3000).at(-1)!.bars[1].w;
    const click = calm.run({ state: "listening", mic: 1 }, FRAME_MS)[0]!.bars[1].w;
    expect(Math.abs(click - before)).toBeLessThan(3);
    // Fast to rise, slower to fall.
    const rise = driver();
    const up = rise.run({ state: "listening", mic: 1 }, 2000);
    const down = rise.run({ state: "listening", mic: 0 }, 2000);
    const upAt = up.findIndex((f) => f.mic > 0.5);
    const downAt = down.findIndex((f) => f.mic < 0.5);
    expect(upAt).toBeGreaterThan(0);
    expect(downAt).toBeGreaterThan(upAt);
    expect(up.at(-1)!.mic).toBeGreaterThan(0.99);
    // The voice is smoothed the same way.
    const said = driver();
    const voiced = said.run({ state: "speaking", voice: 1 }, 2000);
    expect(voiced.map((f) => f.voice)).toEqual(up.map((f) => f.mic));
  });

  it("the_same_inputs_give_the_same_frames", () => {
    // The model never reads a clock: two Breaths given the same times agree exactly, whenever
    // they are run.
    const script: [Partial<BreathInput>, number][] = [
      [{ state: "resting" }, 700],
      [{ state: "listening", mic: 0.7 }, 900],
      [{ state: "thinking" }, 800],
      [{ state: "speaking", voice: 0.9 }, 900],
      [{ state: "asking", options: 4 }, 900],
      [{ state: "done" }, 700],
    ];
    const play = () => {
      const d = driver();
      return script.flatMap(([input, ms]) => d.run(input, ms));
    };
    expect(play()).toEqual(play());
    // Time stands still when the clock does: the same time twice gives the same frame.
    const breath = createBreath();
    breath.step({ ...BASE, state: "thinking" }, 0);
    const once = breath.step({ ...BASE, state: "thinking" }, 500);
    expect(breath.step({ ...BASE, state: "thinking" }, 500)).toEqual(once);
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
          for (const v of [...values(f).filter((n) => !Number.isNaN(n)), f.glow, f.bars[0].opacity]) {
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
      for (const bar of f.bars) {
        expect(bar.w).toBeGreaterThanOrEqual(0);
        expect(bar.h).toBeGreaterThan(0);
      }
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
