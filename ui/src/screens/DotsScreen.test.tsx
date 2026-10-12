import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { nextRun } from "@/screens/DotsScreen";
import { backend } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const scheduled: DotOverview = sample.overview;
const busy: DotOverview = {
  dot: { ...sample.dot, id: "dot-busy", name: "triage", schedule: null },
  active_run: { ...sample.run, id: "run-active", dot_id: "dot-busy", status: "running" },
  last_run: null,
  next_fire: null,
};
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;

describe("the Dots screen", () => {
  it("lists each dot with its state, next run and last result", async () => {
    backend.on("dot_overview", () => [scheduled, busy]);
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });

    const first = within(row(scheduled.dot.name));
    expect(first.getByText(`${scheduled.dot.engine} · ${scheduled.dot.model}`)).toBeVisible();
    expect(first.getByText("Idle")).toBeVisible();
    expect(first.getByText(localDateTime(scheduled.next_fire))).toBeVisible();
    expect(
      first.getByRole("link", { name: `Open the last run of ${scheduled.dot.name}` }),
    ).toHaveAttribute("href", `/runs/${scheduled.last_run!.id}`);
    expect(first.getByRole("switch", { name: `${scheduled.dot.name} enabled` })).toBeChecked();

    const second = within(row("triage"));
    expect(second.getByRole("link", { name: "Open the active run of triage" })).toHaveTextContent(
      "Running",
    );
    expect(second.getByText("No schedule")).toBeVisible();
    expect(second.getByText("Never ran")).toBeVisible();
  });

  it("says why a scheduled dot will not run", () => {
    const disabled = { ...scheduled, dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(scheduled, false)).toBe(localDateTime(scheduled.next_fire));
    expect(nextRun(scheduled, true)).toBe("Paused");
    expect(nextRun(disabled, true)).toBe("Disabled");
    expect(nextRun(busy, false)).toBe("No schedule");
    expect(nextRun({ ...scheduled, next_fire: null }, false)).toBe("Not scheduled");
  });

  it("shows Paused in place of the next run while schedules are paused", async () => {
    backend.on("dot_overview", () => [scheduled]);
    backend.on("status", () => ({ ...quietStatus, schedules_paused: true }));
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });
    expect(await within(row(scheduled.dot.name)).findByText("Paused")).toBeVisible();
  });

  it("explains what a dot is when there are none, and offers to make one", async () => {
    renderApp("/dots");
    expect(await screen.findByText(/No dots yet/)).toBeVisible();
    expect(screen.getByRole("link", { name: "New dot" })).toHaveAttribute("href", "/dots/new");
  });

  it("starts a run and opens it", async () => {
    backend.on("dot_overview", () => [scheduled]);
    backend.on("run_now", () => ({ ...sample.run, id: "run-new", status: "queued" }));
    const { user, router } = renderApp("/dots");
    await user.click(
      await screen.findByRole("button", { name: `Run ${scheduled.dot.name} now` }),
    );
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-new"));
    expect(backend.callsTo("run_now")).toEqual([{ dotId: scheduled.dot.id }]);
  });

  it("shows why a run could not start", async () => {
    backend.on("dot_overview", () => [scheduled]);
    backend.on("run_now", () => {
      throw { kind: "conflict", message: "the dot is disabled" };
    });
    const { user } = renderApp("/dots");
    await user.click(
      await screen.findByRole("button", { name: `Run ${scheduled.dot.name} now` }),
    );
    expect(await screen.findByText("the dot is disabled")).toBeVisible();
  });

  it("turns a dot off and shows the stored state", async () => {
    let enabled = true;
    backend.on("dot_overview", () => [{ ...scheduled, dot: { ...scheduled.dot, enabled } }]);
    backend.on("set_dot_enabled", (args) => {
      enabled = args.enabled as boolean;
      return { ...scheduled.dot, enabled };
    });
    const { user } = renderApp("/dots");
    const toggle = await screen.findByRole("switch", { name: `${scheduled.dot.name} enabled` });
    await user.click(toggle);
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: `${scheduled.dot.name} enabled` }),
      ).not.toBeChecked(),
    );
    expect(backend.callsTo("set_dot_enabled")).toEqual([{ id: scheduled.dot.id, enabled: false }]);
    expect(within(row(scheduled.dot.name)).getByText("Disabled")).toBeVisible();
  });

  it("shows a failed load in place of the list", async () => {
    backend.on("dot_overview", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp("/dots");
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
  });
});
