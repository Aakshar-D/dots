import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { localDateTime, localTime } from "@/lib/format";
import { sample } from "@/test/fixtures";

import { SkipLine } from "./SkipLine";

const skip = sample.scheduleSkip;

/// Only the clock is faked, so nothing waits.
const at = (iso: string) => vi.useFakeTimers({ toFake: ["Date"], now: Date.parse(iso) });

describe("a skipped slot in the conversation", () => {
  afterEach(() => vi.useRealTimers());

  it("says the time of a skip from today", () => {
    at("2026-10-09T11:01:00Z");
    render(<SkipLine skip={skip} />);
    expect(
      screen.getByText(
        `Schedule skipped at ${localTime(skip.slot_at)}: still working on the previous run`,
      ),
    ).toBeVisible();
  });

  it("puts the date before the time of a skip from another day", () => {
    at("2026-10-12T11:01:00Z");
    render(<SkipLine skip={skip} />);
    expect(
      screen.getByText(
        `Schedule skipped at ${localDateTime(skip.slot_at)}: still working on the previous run`,
      ),
    ).toBeVisible();
  });

  it("is muted, like a note", () => {
    at("2026-10-09T11:01:00Z");
    render(<SkipLine skip={skip} />);
    expect(screen.getByText(/Schedule skipped at/)).toHaveClass("text-muted-foreground");
  });
});
