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

const dots = (c: HTMLElement) => [...c.querySelectorAll<SVGCircleElement>("circle[data-dot]")];
/// The numbers written on the three dots.
const written = (c: HTMLElement) =>
  dots(c).map((el) => ["cx", "cy", "r"].map((a) => el.getAttribute(a)).join(" "));
const logoWritten = () =>
  LOGO.map((d) => [d.x, d.y, d.r].map((v) => v.toFixed(2)).join(" "));

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
  it("renders three circles in the logo colours", () => {
    const { container } = render(<Breath size={132} state="resting" />);
    expect(dots(container).map((el) => el.getAttribute("fill"))).toEqual([
      "#f2c94c",
      "#56ccf2",
      "#6fcf97",
    ]);
    // In the logo arrangement before the first frame.
    expect(written(container)).toEqual(logoWritten());
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
      voiceStore.set({ level: { mic: (i % 10) / 10 + 0.1, voice: 0 } });
      if (i % 3 === 0) frame();
    }
    expect(onRender).toHaveBeenCalledTimes(1);
    // The frames moved the dots: they lean apart and swell with the mic.
    expect(written(container)).not.toEqual(before);
    expect(Number(dots(container)[0]!.getAttribute("r"))).toBeGreaterThan(LOGO[0].r);
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
    // It breathes for as long as the microphone is open.
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
    rerender(<Breath size={132} state="done" options={4} />);
    expect(callbacks.size).toBe(1);
  });

  it("the rail form fills its box and the stage form may glow past it", () => {
    const { container } = render(
      <>
        <Breath size={24} state="resting" quiet />
        <Breath size={132} state="resting" />
      </>,
    );
    const [rail, stage] = [...container.querySelectorAll("svg")];
    expect(rail!.querySelector("[data-fit]")?.getAttribute("transform")).toMatch(/scale\(1\.45\)/);
    expect(stage!.querySelector("[data-fit]")).toBeNull();
    expect(stage!.getAttribute("overflow")).toBe("visible");
    expect(rail!.getAttribute("overflow")).toBeNull();
  });

  it("the stage form breathes while resting", () => {
    const { container } = render(<Breath size={132} state="resting" />);
    const seen = new Set<string>();
    for (let i = 0; i < 120; i++) {
      frame();
      seen.add(written(container).join("|"));
    }
    expect(seen.size).toBeGreaterThan(10);
    expect(container.querySelector("[data-glow]")).not.toBeNull();
  });

  it("reduced motion honoured", () => {
    const media = reducedMotion(true);
    const { container, rerender } = render(<Breath size={132} state="listening" />);
    voiceStore.set({ level: { mic: 1, voice: 1 } });
    for (const state of ["listening", "thinking", "speaking", "asking"] as BreathState[]) {
      rerender(<Breath size={132} state={state} options={4} />);
      for (let i = 0; i < 30; i++) {
        frame();
        expect(written(container)).toEqual(logoWritten());
      }
    }
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
  });

  it("each mark has its own filter, and the smallest draws without melting", () => {
    const { container } = render(
      <>
        <Breath size={132} state="speaking" />
        <Breath size={132} state="speaking" />
        <Breath size={24} state="speaking" quiet />
        <Breath size={39} state="speaking" />
      </>,
    );
    const ids = [...container.querySelectorAll("filter")].map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
    const groups = [...container.querySelectorAll("[data-bodies]")];
    expect(groups).toHaveLength(4);
    const used = groups.map((g) => g.getAttribute("filter"));
    expect(used[0]).not.toEqual(used[1]);
    for (const u of used.slice(0, 2)) {
      const id = /^url\(#(.+)\)$/.exec(u ?? "")?.[1];
      expect(id).toBeTruthy();
      const filter = container.querySelector(`filter[id="${id}"]`);
      expect(filter).not.toBeNull();
      // A soft ramp with the same cut point as 24 -10: alpha 5/12.
      expect(filter!.querySelector("feColorMatrix")?.getAttribute("values")).toMatch(/0 0 0 12 -5$/);
    }
    // Below 40 px no melt.
    expect(used[2]).toBeNull();
    expect(used[3]).toBeNull();
  });
});
