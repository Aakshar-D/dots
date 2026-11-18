import { fireEvent, render, screen } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { voiceStore } from "@/lib/use-voice";

import { Breath } from "./Breath";
import { LOGO, type BreathState } from "./breath-model";

/// A `requestAnimationFrame` the test drives by hand.
const callbacks = new Map<number, FrameRequestCallback>();
let nextHandle = 1;
let now = 0;
const original = {
  request: window.requestAnimationFrame,
  cancel: window.cancelAnimationFrame,
  matchMedia: window.matchMedia,
};

/// Runs the frames waiting, 1/60 s after the last.
function frame(): void {
  now += 1000 / 60;
  const waiting = [...callbacks.values()];
  callbacks.clear();
  for (const cb of waiting) cb(now);
}
const frames = (n: number) => {
  for (let i = 0; i < n; i++) frame();
};

/// A `prefers-reduced-motion` query whose answer the test sets and changes.
function reducedMotion(initially: boolean) {
  const listeners = new Set<() => void>();
  const query = {
    matches: initially,
    media: "(prefers-reduced-motion: reduce)",
    addEventListener: (_: string, l: () => void) => listeners.add(l),
    removeEventListener: (_: string, l: () => void) => listeners.delete(l),
  };
  window.matchMedia = (() => query) as unknown as typeof window.matchMedia;
  return {
    change(reduce: boolean) {
      query.matches = reduce;
      for (const l of [...listeners]) l();
    },
  };
}

function setHidden(hidden: boolean) {
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event("visibilitychange"));
}

const bars = (c: ParentNode) => [...c.querySelectorAll<SVGRectElement>("rect[data-bar]")];
const SHAPE = ["x", "y", "width", "height", "rx"];
/// The numbers written on the three bars.
const written = (c: ParentNode) =>
  bars(c).map((el) => SHAPE.map((a) => el.getAttribute(a)).join(" "));
/// The logo as a `rect` holds it: the corner, the size, and ends rounded into a pill.
const logoWritten = () =>
  LOGO.map((b) =>
    [b.x - b.w / 2, b.y - b.h / 2, b.w, b.h, b.h / 2].map((v) => v.toFixed(2)).join(" "),
  );
const number = (el: Element, name: string) => Number(el.getAttribute(name));
/// Where a bar's centre is, across.
const centre = (el: Element) => number(el, "x") + number(el, "width") / 2;

beforeEach(() => {
  callbacks.clear();
  now = 0;
  window.requestAnimationFrame = (cb) => {
    const handle = nextHandle++;
    callbacks.set(handle, cb);
    return handle;
  };
  window.cancelAnimationFrame = (handle) => void callbacks.delete(handle);
  reducedMotion(false);
  voiceStore.reset();
});
afterEach(() => {
  window.requestAnimationFrame = original.request;
  window.cancelAnimationFrame = original.cancel;
  window.matchMedia = original.matchMedia;
  // `document.hidden` is jsdom's own again.
  delete (document as { hidden?: boolean }).hidden;
  voiceStore.reset();
});

describe("Breath", () => {
  it("renders three pill bars stepping like an S, the middle one in the brand colours", () => {
    const { container } = render(<Breath size={132} state="resting" />);
    const [top, middle, bottom] = bars(container);
    expect(bars(container)).toHaveLength(3);
    // In the logo arrangement before the first frame.
    expect(written(container)).toEqual(logoWritten());
    expect(centre(top!)).toBeGreaterThan(centre(middle!));
    expect(centre(bottom!)).toBeLessThan(centre(middle!));
    expect(number(top!, "y")).toBeLessThan(number(middle!, "y"));
    expect(number(middle!, "y")).toBeLessThan(number(bottom!, "y"));
    // The outer bars take the text colour, so they follow the theme.
    expect(container.querySelector("svg")).toHaveClass("text-foreground");
    expect(top!.getAttribute("fill")).toBe("currentColor");
    expect(bottom!.getAttribute("fill")).toBe("currentColor");
    // The middle bar is filled with a gradient laid across the box, so its colours do not
    // slide as the bar changes width.
    const id = /^url\(#(.+)\)$/.exec(middle!.getAttribute("fill") ?? "")?.[1];
    expect(id).toBeTruthy();
    const sweep = container.querySelector(`linearGradient[id="${id}"]`);
    expect(sweep?.getAttribute("gradientUnits")).toBe("userSpaceOnUse");
    expect(["x1", "x2", "y1", "y2"].map((a) => sweep?.getAttribute(a))).toEqual([
      "0",
      "200",
      "0",
      "0",
    ]);
    expect([...sweep!.querySelectorAll("stop")].map((s) => s.getAttribute("stop-color"))).toEqual([
      "#c9f27a",
      "#5fe0a6",
      "#4bb8e8",
    ]);
    // The fourth bar is there for a fourth option, and has no width until then.
    const extra = container.querySelector("rect[data-extra]");
    expect(extra?.getAttribute("width")).toBe("0.00");
    expect(extra?.getAttribute("fill")).toBe("currentColor");
    expect(container.querySelector("[data-bars]")?.querySelectorAll("rect")).toHaveLength(4);
    expect(container.querySelector("circle")).toBeNull();
  });

  it("a frame writes attributes without re-rendering", () => {
    const onRender = vi.fn();
    const { container } = render(
      <Profiler id="breath" onRender={onRender}>
        <Breath size={132} state="listening" />
      </Profiler>,
    );
    expect(onRender).toHaveBeenCalledTimes(1);
    const before = written(container);
    for (let i = 0; i < 30; i++) {
      voiceStore.set({ level: { mic: 1, voice: 0 } });
      frame();
    }
    expect(onRender).toHaveBeenCalledTimes(1);
    // The frames moved the bars: they centre up and widen with the mic, the middle one most.
    expect(written(container)).not.toEqual(before);
    const [top, middle, bottom] = bars(container);
    expect(centre(top!)).toBeLessThan(LOGO[0].x - 5);
    expect(number(middle!, "width")).toBeGreaterThan(LOGO[1].w + 20);
    expect(number(middle!, "width")).toBeGreaterThan(number(top!, "width"));
    expect(number(middle!, "width")).toBeGreaterThan(number(bottom!, "width"));
  });

  it("speaking swells the middle bar with the voice and keeps it a pill", () => {
    const { container } = render(<Breath size={132} state="speaking" />);
    for (let i = 0; i < 40; i++) {
      voiceStore.set({ level: { mic: 0, voice: 1 } });
      frame();
    }
    const middle = bars(container)[1]!;
    expect(number(middle, "height")).toBeGreaterThan(LOGO[1].h * 1.3);
    expect(number(middle, "rx")).toBeCloseTo(number(middle, "height") / 2, 1);
    // The glow is a copy of the middle bar, and moves with it.
    const halo = container.querySelector("[data-glow] rect")!;
    for (const a of SHAPE) expect(halo.getAttribute(a)).toBe(middle.getAttribute(a));
    expect(halo.getAttribute("fill")).toBe(middle.getAttribute("fill"));
  });

  it("asking shows as many bars as options", () => {
    const { container, rerender } = render(<Breath size={132} state="asking" options={4} />);
    frames(120);
    const all = () => [...container.querySelectorAll("[data-bars] rect")];
    const shown = () => all().filter((el) => number(el, "width") > 1);
    expect(shown()).toHaveLength(4);
    expect(number(container.querySelector("rect[data-extra]")!, "width")).toBeGreaterThan(80);
    rerender(<Breath size={132} state="asking" options={2} />);
    frames(120);
    expect(shown()).toHaveLength(2);
    expect(container.querySelector("rect[data-extra]")!.getAttribute("width")).toBe("0.00");
    // A bar on its way out is never drawn taller than it is wide.
    for (const el of all()) expect(number(el, "height")).toBeLessThanOrEqual(number(el, "width"));
  });

  it("stops the loop on unmount", () => {
    const { unmount } = render(<Breath size={132} state="thinking" />);
    frames(3);
    expect(callbacks.size).toBe(1);
    unmount();
    expect(callbacks.size).toBe(0);
    frames(3);
    expect(callbacks.size).toBe(0);
  });

  it("stops the loop while the document is hidden and resumes when visible", () => {
    const { container } = render(<Breath size={132} state="thinking" />);
    frames(3);
    setHidden(true);
    expect(callbacks.size).toBe(0);
    const held = written(container);
    frames(5);
    expect(written(container)).toEqual(held);
    setHidden(false);
    expect(callbacks.size).toBe(1);
    frames(5);
    expect(written(container)).not.toEqual(held);
  });

  it("starts hidden without a loop", () => {
    setHidden(true);
    render(<Breath size={132} state="thinking" />);
    expect(callbacks.size).toBe(0);
  });

  it("the_quiet_mark_stops_asking_for_frames_when_resting", () => {
    const { container } = render(<Breath size={24} state="resting" quiet />);
    frames(5);
    // Still and in the logo: no frame is asked for any more.
    expect(callbacks.size).toBe(0);
    expect(written(container)).toEqual(logoWritten());
    // The rail form has no glow.
    expect(container.querySelector("[data-glow]")).toBeNull();
    expect(container.querySelector("filter")).toBeNull();
    // A store change that moves nothing wakes it for a frame, and it sleeps again.
    voiceStore.set({ hearing: "x" });
    frames(3);
    expect(callbacks.size).toBe(0);
  });

  it("it_wakes_when_the_microphone_opens", () => {
    const { container } = render(<Breath size={24} state="resting" quiet />);
    frames(5);
    expect(callbacks.size).toBe(0);
    voiceStore.set({ micOpen: true });
    expect(callbacks.size).toBe(1);
    const seen = new Set<string>();
    for (let i = 0; i < 120; i++) {
      frame();
      seen.add(written(container).join("|"));
    }
    // It drifts for as long as the microphone is open.
    expect(seen.size).toBeGreaterThan(10);
    expect(callbacks.size).toBe(1);
    // Closed again: it settles back into the logo and sleeps.
    voiceStore.set({ micOpen: false });
    frames(120);
    expect(callbacks.size).toBe(0);
    expect(written(container)).toEqual(logoWritten());
  });

  it("it_wakes_on_a_state_change", () => {
    const { container, rerender } = render(<Breath size={132} state="done" />);
    frames(5);
    expect(callbacks.size).toBe(0);
    rerender(<Breath size={132} state="thinking" />);
    expect(callbacks.size).toBe(1);
    frames(10);
    expect(written(container)).not.toEqual(logoWritten());
    // Done again settles and sleeps; the options changing wakes it too.
    rerender(<Breath size={132} state="done" />);
    frames(200);
    expect(callbacks.size).toBe(0);
    expect(written(container)).toEqual(logoWritten());
    rerender(<Breath size={132} state="done" options={4} />);
    expect(callbacks.size).toBe(1);
  });

  it("it_wakes_when_sound_comes_in_and_sleeps_when_it_has_faded", () => {
    render(<Breath size={132} state="done" />);
    frames(5);
    expect(callbacks.size).toBe(0);
    voiceStore.set({ level: { mic: 0, voice: 0.8 } });
    expect(callbacks.size).toBe(1);
    frames(30);
    expect(callbacks.size).toBe(1);
    voiceStore.set({ level: { mic: 0, voice: 0 } });
    frames(300);
    expect(callbacks.size).toBe(0);
  });

  it("the rail form fills its box and the stage form may glow past it", () => {
    const { container } = render(
      <>
        <Breath size={24} state="resting" quiet />
        <Breath size={132} state="resting" />
      </>,
    );
    const [rail, stage] = [...container.querySelectorAll("svg")];
    const scale = Number(
      /scale\(([\d.]+)\)/.exec(rail!.querySelector("[data-fit]")?.getAttribute("transform") ?? "")?.[1],
    );
    // The logo is as wide as its middle bar: scaled, about 80% of the 200 box.
    expect((LOGO[1].w * scale) / 200).toBeGreaterThan(0.75);
    expect((LOGO[1].w * scale) / 200).toBeLessThan(0.85);
    expect(rail!.querySelector("[data-fit]")?.getAttribute("transform")).toMatch(
      /^translate\(100 100\) scale\([\d.]+\) translate\(-100 -100\)$/,
    );
    expect(bars(rail!.querySelector("[data-fit]")!)).toHaveLength(3);
    expect(stage!.querySelector("[data-fit]")).toBeNull();
    expect(stage!.getAttribute("overflow")).toBe("visible");
    expect(rail!.getAttribute("overflow")).toBeNull();
  });

  it("the stage form drifts while resting", () => {
    const { container } = render(<Breath size={132} state="resting" />);
    const seen = new Set<string>();
    for (let i = 0; i < 120; i++) {
      frame();
      seen.add(written(container).join("|"));
    }
    expect(seen.size).toBeGreaterThan(10);
    expect(container.querySelector("[data-glow]")).not.toBeNull();
  });

  it("trouble dims the bars and the glow", () => {
    const { container } = render(<Breath size={132} state="trouble" />);
    frames(200);
    expect(Number(container.querySelector("[data-bars]")!.getAttribute("opacity"))).toBeCloseTo(0.32, 2);
    expect(Number(container.querySelector("[data-glow]")!.getAttribute("opacity"))).toBeLessThan(0.15);
    expect(written(container)).toEqual(logoWritten());
    expect(callbacks.size).toBe(0);
  });

  it("reduced motion honoured", () => {
    const media = reducedMotion(true);
    const { container, rerender } = render(<Breath size={132} state="listening" />);
    voiceStore.set({ level: { mic: 1, voice: 1 } });
    const glows = new Set<string | null>();
    for (const state of ["listening", "thinking", "speaking", "asking"] as BreathState[]) {
      rerender(<Breath size={132} state={state} options={4} />);
      for (let i = 0; i < 30; i++) {
        frame();
        expect(written(container)).toEqual(logoWritten());
        expect(container.querySelector("rect[data-extra]")!.getAttribute("width")).toBe("0.00");
        glows.add(container.querySelector("[data-glow]")!.getAttribute("opacity"));
      }
    }
    // Brightness carries the sound instead.
    expect(glows.size).toBeGreaterThan(5);
    // It follows the setting as it changes.
    media.change(false);
    frames(30);
    expect(written(container)).not.toEqual(logoWritten());
  });

  it("is a button with a label when it can be pressed", () => {
    const onClick = vi.fn();
    const { rerender } = render(<Breath size={132} state="speaking" onClick={onClick} />);
    fireEvent.click(screen.getByRole("button", { name: "Stop and listen" }));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("img")).toBeNull();
    rerender(<Breath size={132} state="resting" onClick={onClick} label="Talk to Sage" />);
    expect(screen.getByRole("button", { name: "Talk to Sage" })).toBeInTheDocument();
  });

  it("is an image named for its state otherwise", () => {
    const { rerender } = render(<Breath size={132} state="resting" />);
    for (const [state, name] of [
      ["listening", "Sage is listening"],
      ["thinking", "Sage is thinking"],
      ["speaking", "Sage is speaking"],
      ["asking", "Sage is asking"],
      ["resting", "Sage is resting"],
      ["done", "Sage is resting"],
      ["trouble", "Sage has a problem"],
    ] as const) {
      rerender(<Breath size={132} state={state} />);
      expect(screen.getByRole("img", { name })).toBeInTheDocument();
      expect(screen.queryByRole("button")).toBeNull();
    }
    rerender(<Breath size={132} state="resting" label="Sage" />);
    expect(screen.getByRole("img", { name: "Sage" })).toBeInTheDocument();
  });

  it("each mark has its own gradient and glow, and nothing melts", () => {
    const { container } = render(
      <>
        <Breath size={132} state="speaking" />
        <Breath size={132} state="speaking" />
        <Breath size={24} state="speaking" quiet />
      </>,
    );
    // Two marks on one page must not share an id, or one would be painted with the other's.
    const ids = [...container.querySelectorAll("linearGradient, filter")].map((el) => el.id);
    expect(ids).toHaveLength(5);
    expect(new Set(ids).size).toBe(ids.length);
    for (const svg of container.querySelectorAll("svg")) {
      const fill = /^url\(#(.+)\)$/.exec(bars(svg)[1]!.getAttribute("fill") ?? "")?.[1];
      expect(svg.querySelector(`linearGradient[id="${fill}"]`)).not.toBeNull();
      const glow = svg.querySelector("[data-glow]");
      if (glow) {
        const id = /^url\(#(.+)\)$/.exec(glow.getAttribute("filter") ?? "")?.[1];
        expect(svg.querySelector(`filter[id="${id}"] feGaussianBlur`)).not.toBeNull();
      }
      // The bars are drawn as they are: only the glow is blurred.
      expect(svg.querySelector("[data-bars]")!.getAttribute("filter")).toBeNull();
      expect(svg.querySelector("feColorMatrix")).toBeNull();
    }
  });
});
