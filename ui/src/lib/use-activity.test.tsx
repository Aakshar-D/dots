import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useActivity } from "@/lib/use-activity";
import { backend } from "@/test/backend";

// Final review I-3: reading with the scroll wheel or moving the pointer is use, not idle.
describe("the window's activity", () => {
  it("counts scrolling, pointer movement and focus as input", () => {
    vi.useFakeTimers();
    try {
      renderHook(() => useActivity());
      for (const event of [
        new WheelEvent("wheel"),
        new PointerEvent("pointermove"),
        new FocusEvent("focus"),
      ]) {
        vi.advanceTimersByTime(31_000);
        const before = backend.callsTo("report_activity").length;
        window.dispatchEvent(event);
        expect(backend.callsTo("report_activity").length).toBe(before + 1);
      }
    } finally {
      vi.useRealTimers();
    }
  });
});
