import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, DotOverview, Run } from "@/api/types";
import { localDateTime, localTime } from "@/lib/format";
import { nextRun, retrying } from "@/screens/DotsScreen";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// The contract's sample, whose retry waits and which needs attention.
const troubled: DotOverview = sample.overview;
/// A scheduled dot with nothing waiting.
const scheduled: DotOverview = {
  ...troubled,
  schedule_state: {
    ...troubled.schedule_state!,
    retry_at: null,
    retry_attempt: null,
    attention: null,
  },
};
const busy: DotOverview = {
  dot: { ...sample.dot, id: "dot-busy", name: "triage", schedule: null },
  active_run: { ...sample.run, id: "run-active", dot_id: "dot-busy", status: "running" },
  last_run: null,
  schedule_state: null,
};
/// `scheduled` with its next fire time at `next_fire`.
const firing = (next_fire: string | null): DotOverview => ({
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, next_fire },
});
/// `scheduled` with the state `status` after `fired_count` slots fired.
const ended = (status: "completed" | "expired", fired_count: number): DotOverview => ({
  ...scheduled,
  schedule_state: { ...scheduled.schedule_state!, status, fired_count, next_fire: null },
});
const row = (name: string) => screen.getByRole("link", { name }).closest("tr")!;

describe("the Dots screen", () => {
  it("lists each dot with its state, next run and last result", async () => {
    backend.on("dot_overview", () => [scheduled, busy]);
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });

    const first = within(row(scheduled.dot.name));
    expect(first.getByText(`${scheduled.dot.engine} · ${scheduled.dot.model}`)).toBeVisible();
    expect(first.getByText("Idle")).toBeVisible();
    expect(first.getByText(localDateTime(scheduled.schedule_state!.next_fire))).toBeVisible();
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
    // Nothing waits for either.
    expect(screen.queryByText("Needs attention")).toBeNull();
    expect(screen.queryByText(/^Retrying at/)).toBeNull();
  });

  it("says why a scheduled dot will not run", () => {
    const disabled = { ...scheduled, dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(scheduled, false)).toBe(localDateTime(scheduled.schedule_state!.next_fire));
    expect(nextRun(scheduled, true)).toBe("Paused");
    expect(nextRun(disabled, true)).toBe("Disabled");
    expect(nextRun(busy, false)).toBe("No schedule");
    expect(nextRun(firing(null), false)).toBe("Not scheduled");
    expect(nextRun({ ...scheduled, schedule_state: null }, false)).toBe("Not scheduled");
  });

  it("says that a schedule has ended, and how", () => {
    expect(nextRun(ended("completed", 3), false)).toBe("Completed");
    // A Once whose slot was missed and skipped completed without a run.
    expect(nextRun(ended("completed", 0), false)).toBe("Completed (missed)");
    // A stored cron expression that no longer parses.
    expect(nextRun(ended("expired", 2), false)).toBe("Expired");
    // An ended schedule stays ended while schedules are paused.
    expect(nextRun(ended("completed", 1), true)).toBe("Completed");
    const off = { ...ended("completed", 1), dot: { ...scheduled.dot, enabled: false } };
    expect(nextRun(off, false)).toBe("Disabled");
  });

  it("says when a retry waits, unless nothing can run", () => {
    const at = troubled.schedule_state!.retry_at;
    expect(retrying(troubled, false)).toBe(`Retrying at ${localTime(at)} (attempt 2 of 6)`);
    expect(retrying(scheduled, false)).toBeNull();
    expect(retrying(troubled, true)).toBeNull();
    expect(retrying({ ...troubled, dot: { ...troubled.dot, enabled: false } }, false)).toBeNull();
    expect(retrying({ ...troubled, schedule_state: null }, false)).toBeNull();
  });

  it("flags a dot whose schedule needs attention, and shows its waiting retry", async () => {
    backend.on("dot_overview", () => [troubled, busy]);
    renderApp("/dots");
    await screen.findByRole("link", { name: troubled.dot.name });
    const first = within(row(troubled.dot.name));
    expect(first.getByText("Needs attention")).toBeVisible();
    expect(first.getByText(localDateTime(troubled.schedule_state!.next_fire))).toBeVisible();
    expect(
      first.getByText(
        `Retrying at ${localTime(troubled.schedule_state!.retry_at)} (attempt 2 of 6)`,
      ),
    ).toBeVisible();
    expect(within(row("triage")).queryByText("Needs attention")).toBeNull();
  });

  it("shows how an ended schedule ended in place of the next run", async () => {
    backend.on("dot_overview", () => [ended("completed", 0)]);
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });
    expect(within(row(scheduled.dot.name)).getByText("Completed (missed)")).toBeVisible();
  });

  it("shows Paused in place of the next run while schedules are paused", async () => {
    backend.on("dot_overview", () => [scheduled]);
    backend.on("status", () => ({ ...quietStatus, schedules_paused: true }));
    renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });
    expect(await within(row(scheduled.dot.name)).findByText("Paused")).toBeVisible();
  });

  it("shows the new next run when the tray resumes the schedules", async () => {
    const paused = { ...quietStatus, schedules_paused: true };
    // While paused the core holds the time the dot would have run, which is past by now.
    const next = "2026-10-20T01:00:00.000Z";
    let overview = [scheduled];
    backend.on("dot_overview", () => overview);
    backend.on("status", () => paused);
    const { listening } = renderApp("/dots");
    await screen.findByRole("link", { name: scheduled.dot.name });
    expect(await within(row(scheduled.dot.name)).findByText("Paused")).toBeVisible();
    await listening();

    overview = [firing(next)];
    backend.on("status", () => quietStatus);
    act(() => backend.emit("status", quietStatus));
    expect(await within(row(scheduled.dot.name)).findByText(localDateTime(next))).toBeVisible();
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

  it("holds only the switch of the dot that is being switched", async () => {
    backend.on("dot_overview", () => [scheduled, busy]);
    const answers = { [scheduled.dot.id]: later<Dot>(), [busy.dot.id]: later<Dot>() };
    backend.on("set_dot_enabled", (args) => answers[args.id as string].answer);
    const { user } = renderApp("/dots");
    const first = await screen.findByRole("switch", { name: `${scheduled.dot.name} enabled` });
    const second = screen.getByRole("switch", { name: "triage enabled" });

    await user.click(first);
    await waitFor(() => expect(first).toBeDisabled());
    expect(second).toBeEnabled();
    await user.click(second);
    await waitFor(() => expect(second).toBeDisabled());
    expect(backend.callsTo("set_dot_enabled")).toEqual([
      { id: scheduled.dot.id, enabled: false },
      { id: busy.dot.id, enabled: false },
    ]);

    // Each switch is free again when its own command has answered.
    await act(async () => answers[busy.dot.id].give({ ...busy.dot, enabled: false }));
    await waitFor(() => expect(second).toBeEnabled());
    expect(first).toBeDisabled();
    await act(async () => answers[scheduled.dot.id].give({ ...scheduled.dot, enabled: false }));
    await waitFor(() => expect(first).toBeEnabled());
  });

  it("holds only the Run button of the dot that is being started", async () => {
    backend.on("dot_overview", () => [scheduled, busy]);
    const answers = { [scheduled.dot.id]: later<Run>(), [busy.dot.id]: later<Run>() };
    backend.on("run_now", (args) => answers[args.dotId as string].answer);
    const { user, router } = renderApp("/dots");
    const first = await screen.findByRole("button", { name: `Run ${scheduled.dot.name} now` });
    const second = screen.getByRole("button", { name: "Run triage now" });

    await user.click(first);
    await waitFor(() => expect(first).toBeDisabled());
    expect(second).toBeEnabled();
    await user.click(second);
    await waitFor(() => expect(second).toBeDisabled());
    expect(backend.callsTo("run_now")).toEqual([
      { dotId: scheduled.dot.id },
      { dotId: busy.dot.id },
    ]);

    // Each started run is opened when the core has answered: the last answer is what stays.
    await act(async () => answers[busy.dot.id].give({ ...sample.run, id: "run-triage" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-triage"));
    await act(async () => answers[scheduled.dot.id].give({ ...sample.run, id: "run-nightly" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-nightly"));
  });

  /// A backend holding one dot, for a walk between the list and the dot's editor.
  function oneDot() {
    const held = { dot: scheduled.dot };
    backend.on("dot_overview", () => [{ ...scheduled, dot: held.dot }]);
    backend.on("get_dot", () => held.dot);
    backend.on("update_dot", (args) => {
      held.dot = { ...held.dot, ...(args.spec as object) };
      return held.dot;
    });
    return held;
  }
  const name = scheduled.dot.name;

  it("hands the dot it switched off to the editor, which then saves it switched off", async () => {
    const held = oneDot();
    backend.on("set_dot_enabled", (args) => {
      held.dot = { ...held.dot, enabled: args.enabled as boolean };
      return held.dot;
    });
    // The editor loads the dot while it is enabled.
    const { user } = renderApp(`/dots/${scheduled.dot.id}`);
    expect(await screen.findByLabelText("Enabled")).toBeChecked();

    await user.click(screen.getByRole("link", { name: "Dots" }));
    await user.click(await screen.findByRole("switch", { name: `${name} enabled` }));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: `${name} enabled` })).not.toBeChecked(),
    );

    await user.click(screen.getByRole("link", { name }));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      id: scheduled.dot.id,
      spec: { enabled: false },
    });
  });

  it("makes the editor load the dot again after a switch whose answer was lost", async () => {
    const held = oneDot();
    // The dot is switched off, but the answer is not one the UI can read.
    backend.on("set_dot_enabled", (args) => {
      held.dot = { ...held.dot, enabled: args.enabled as boolean };
      return { id: held.dot.id };
    });
    const { user } = renderApp(`/dots/${scheduled.dot.id}`);
    expect(await screen.findByLabelText("Enabled")).toBeChecked();

    await user.click(screen.getByRole("link", { name: "Dots" }));
    await user.click(await screen.findByRole("switch", { name: `${name} enabled` }));
    expect(await screen.findByText(/^set_dot_enabled: unexpected/)).toBeVisible();

    await user.click(screen.getByRole("link", { name }));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
  });

  it("fills an editor opened during a switch from the switch's answer", async () => {
    const held = oneDot();
    const answer = later<Dot>();
    backend.on("set_dot_enabled", () => answer.answer);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("switch", { name: `${name} enabled` }));
    await waitFor(() => expect(backend.callsTo("set_dot_enabled")).toHaveLength(1));

    // The command is on its way to the core, which still holds the enabled dot: a copy loaded
    // now would be the old one, so the editor waits for the command.
    await user.click(screen.getByRole("link", { name }));
    await screen.findByRole("heading", { name: "Dot" });
    await act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));
    expect(backend.callsTo("get_dot")).toEqual([]);
    expect(screen.queryByLabelText("Enabled")).toBeNull();

    held.dot = { ...held.dot, enabled: false };
    await act(async () => answer.give(held.dot));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
  });

  it("loads the dot for an editor opened during a switch that then fails", async () => {
    oneDot();
    const answer = later<Dot>();
    backend.on("set_dot_enabled", () => answer.answer);
    const { user } = renderApp("/dots");
    await user.click(await screen.findByRole("switch", { name: `${name} enabled` }));
    await waitFor(() => expect(backend.callsTo("set_dot_enabled")).toHaveLength(1));
    await user.click(screen.getByRole("link", { name }));
    await screen.findByRole("heading", { name: "Dot" });

    await act(async () => answer.fail({ kind: "internal", message: "database is locked" }));
    expect(await screen.findByLabelText("Enabled")).toBeChecked();
    expect(screen.getByText("database is locked")).toBeVisible();
  });

  it("shows a failed load in place of the list", async () => {
    backend.on("dot_overview", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp("/dots");
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
  });
});
