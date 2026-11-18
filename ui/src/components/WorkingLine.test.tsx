import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  DOT_FRAMES,
  PHRASE_MS,
  WORKING_PHRASES,
  WorkingLine,
  elapsed,
  firstPhrase,
} from "@/components/WorkingLine";

const shown = () => screen.getByRole("status");
const phraseNow = () =>
  WORKING_PHRASES.find((p) => shown().textContent?.includes(`${p}…`)) ?? null;

describe("the working line", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-11-05T10:00:30Z"));
  });
  afterEach(() => vi.useRealTimers());

  it("tells a screen reader once that the sol is working, and starts on the run's phrase", () => {
    render(<WorkingLine seed="run-1" since="2026-11-05T10:00:00Z" dotName="Scout" />);
    expect(screen.getByText("Scout is working")).toHaveClass("sr-only");
    expect(phraseNow()).toBe(WORKING_PHRASES[firstPhrase("run-1")]);
  });

  it("moves on to the next phrase every few seconds", () => {
    render(<WorkingLine seed="run-1" since={null} />);
    const first = firstPhrase("run-1");
    act(() => vi.advanceTimersByTime(PHRASE_MS));
    expect(phraseNow()).toBe(WORKING_PHRASES[(first + 1) % WORKING_PHRASES.length]);
    act(() => vi.advanceTimersByTime(PHRASE_MS));
    expect(phraseNow()).toBe(WORKING_PHRASES[(first + 2) % WORKING_PHRASES.length]);
  });

  it("counts the time since the run started", () => {
    render(<WorkingLine seed="run-1" since="2026-11-05T10:00:00Z" />);
    expect(shown()).toHaveTextContent("30s");
    act(() => vi.advanceTimersByTime(45_000));
    expect(shown()).toHaveTextContent("1m 15s");
  });

  it("shows no time when the run has no start yet", () => {
    render(<WorkingLine seed="run-1" since={null} />);
    expect(shown().textContent).not.toMatch(/\d+s/);
  });

  it("pulses its sol, and holds it still with reduced motion", () => {
    const { unmount } = render(<WorkingLine seed="run-1" since={null} />);
    act(() => vi.advanceTimersByTime(140));
    expect(shown()).toHaveTextContent(DOT_FRAMES[1]);
    unmount();

    const media = vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
    } as MediaQueryList);
    render(<WorkingLine seed="run-1" since={null} />);
    act(() => vi.advanceTimersByTime(1000));
    expect(shown()).toHaveTextContent(DOT_FRAMES[0]);
    media.mockRestore();
  });
});

describe("the working line's pieces", () => {
  it("writes the time short", () => {
    expect(elapsed(0)).toBe("0s");
    expect(elapsed(59.9)).toBe("59s");
    expect(elapsed(65)).toBe("1m 05s");
    expect(elapsed(3725)).toBe("1h 02m");
    expect(elapsed(-3)).toBe("0s");
  });

  it("starts different runs at a stable place in the list", () => {
    expect(firstPhrase("run-1")).toBe(firstPhrase("run-1"));
    const starts = new Set(["a", "b", "c", "d", "e", "f"].map(firstPhrase));
    expect(starts.size).toBeGreaterThan(1);
  });

  it("has its own words, not the ones Claude Code shows", () => {
    for (const word of ["Thinking", "Pondering", "Percolating", "Noodling", "Finagling", "Doodling"]) {
      expect(WORKING_PHRASES.some((p) => p.startsWith(word))).toBe(false);
    }
    expect(new Set(WORKING_PHRASES).size).toBe(WORKING_PHRASES.length);
  });
});
