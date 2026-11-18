import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Run } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { QuietCard } from "@/screens/home/QuietCard";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const MIN = 60_000;
const quiet = (minutesAgo: number, limitSecs = 600): Run => ({
  ...sample.run,
  status: "running",
  quiet_since: new Date(NOW - minutesAgo * MIN).toISOString(),
  quiet_limit_secs: limitSecs,
});

function show(run: Run) {
  const queryClient = createQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <QuietCard run={run} />
    </QueryClientProvider>,
  );
}

// Only the clock is faked: the timers stay real, so nothing here waits for time to pass.
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
});
afterEach(() => vi.useRealTimers());

describe("the quiet card", () => {
  it("says how long the run has been quiet and when it stops by itself", () => {
    show(quiet(3));
    expect(screen.getByText("Quiet for 13 min")).toBeVisible();
    expect(screen.getByText("Stops by itself in 7 min")).toBeVisible();
  });

  it("takes its minutes from the run's limit, whatever the sol says now", () => {
    // The dot's limit was edited to 60 s while the run was quiet; the run keeps its 900 s.
    backend.on("get_dot", () => ({ ...sample.dot, quiet_limit_secs: 60 }));
    show(quiet(0, 900));
    expect(screen.getByText("Quiet for 15 min")).toBeVisible();
    expect(screen.getByText("Stops by itself in 15 min")).toBeVisible();
  });

  it("counts on with the clock", () => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
    show(quiet(3));
    act(() => {
      vi.advanceTimersByTime(2 * MIN);
    });
    expect(screen.getByText("Quiet for 15 min")).toBeVisible();
    expect(screen.getByText("Stops by itself in 5 min")).toBeVisible();
  });

  it("is not there for a run that is not quiet or not running", () => {
    const { container, rerender } = show({ ...quiet(3), quiet_since: null });
    expect(container).toBeEmptyDOMElement();
    rerender(
      <QueryClientProvider client={createQueryClient()}>
        <QuietCard run={{ ...quiet(3), status: "succeeded" }} />
      </QueryClientProvider>,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("stops the run", async () => {
    backend.on("cancel_run", () => sample.run);
    show(quiet(3));
    await userEvent.setup().click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(backend.callsTo("cancel_run")).toEqual([{ runId: sample.run.id }]));
  });

  it("shows the error on the card when Stop fails", async () => {
    backend.on("cancel_run", () => {
      throw { kind: "internal", message: "could not stop the run" };
    });
    show(quiet(3));
    await userEvent.setup().click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("could not stop the run");
  });

  it("keeps waiting", async () => {
    backend.on("keep_waiting", () => null);
    show(quiet(3));
    await userEvent.setup().click(screen.getByRole("button", { name: "Keep waiting" }));
    await waitFor(() =>
      expect(backend.callsTo("keep_waiting")).toEqual([{ runId: sample.run.id }]),
    );
  });

  it("shows the error on the card when the run has just ended", async () => {
    backend.on("keep_waiting", () => {
      throw { kind: "conflict", message: "the run is not running" };
    });
    show(quiet(3));
    await userEvent.setup().click(screen.getByRole("button", { name: "Keep waiting" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("the run is not running");
  });

  it("shows the latest failure only: Stop's error goes when Keep waiting is pressed", async () => {
    backend.on("cancel_run", () => {
      throw { kind: "internal", message: "could not stop the run" };
    });
    backend.on("keep_waiting", () => {
      throw { kind: "conflict", message: "the run is not running" };
    });
    show(quiet(3));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("could not stop the run");
    await user.click(screen.getByRole("button", { name: "Keep waiting" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("the run is not running"),
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("could not stop the run");
  });

  it("drops Stop's error when Keep waiting then succeeds", async () => {
    backend.on("cancel_run", () => {
      throw { kind: "internal", message: "could not stop the run" };
    });
    backend.on("keep_waiting", () => null);
    show(quiet(3));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(await screen.findByRole("alert")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Keep waiting" }));
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});
