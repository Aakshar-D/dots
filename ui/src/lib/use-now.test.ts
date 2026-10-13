import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useNow } from "@/lib/use-now";

describe("useNow", () => {
  afterEach(() => vi.useRealTimers());

  it("moves with the clock only while ticking", () => {
    vi.useFakeTimers({ now: 1_000_000 });
    const { result, rerender } = renderHook(({ ticking }) => useNow(ticking), {
      initialProps: { ticking: true },
    });
    expect(result.current).toBe(1_000_000);
    act(() => vi.advanceTimersByTime(3000));
    expect(result.current).toBe(1_003_000);

    rerender({ ticking: false });
    act(() => vi.advanceTimersByTime(5000));
    expect(result.current).toBe(1_003_000);
  });
});
