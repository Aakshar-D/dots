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

  it("quiet and resting writes the same numbers every frame", () => {
    const { container } = render(<Breath size={24} state="resting" quiet />);
    const seen = new Set<string>();
    for (let i = 0; i < 120; i++) {
      frame();
      seen.add(written(container).join("|"));
    }
    expect([...seen]).toEqual([logoWritten().join("|")]);
    // The rail form has no glow.
    expect(container.querySelector("[data-glow]")).toBeNull();
    // With the microphone open it breathes.
    voiceStore.set({ micOpen: true });
    for (let i = 0; i < 120; i++) {
      frame();
      seen.add(written(container).join("|"));
    }
    expect(seen.size).toBeGreaterThan(10);
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
    rerender(<Breath size={132} state="resting" onClick={onClick} label="Talk to Dot" />);
    expect(screen.getByRole("button", { name: "Talk to Dot" })).toBeInTheDocument();
  });

  it("is an image named for its state otherwise", () => {
    const { rerender } = render(<Breath size={132} state="resting" />);
    for (const [state, name] of [
      ["listening", "Dot is listening"],
      ["thinking", "Dot is thinking"],
      ["speaking", "Dot is speaking"],
      ["asking", "Dot is asking"],
      ["resting", "Dot is resting"],
      ["done", "Dot is resting"],
      ["trouble", "Dot has a problem"],
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
      </>,
    );
    const ids = [...container.querySelectorAll("filter")].map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
    const groups = [...container.querySelectorAll("[data-bodies]")];
    expect(groups).toHaveLength(3);
    const used = groups.map((g) => g.getAttribute("filter"));
    expect(used[0]).not.toEqual(used[1]);
    for (const u of used.slice(0, 2)) {
      const id = /^url\(#(.+)\)$/.exec(u ?? "")?.[1];
      expect(id).toBeTruthy();
      expect(container.querySelector(`filter[id="${id}"]`)).not.toBeNull();
    }
    expect(used[2]).toBeNull();
  });
});
