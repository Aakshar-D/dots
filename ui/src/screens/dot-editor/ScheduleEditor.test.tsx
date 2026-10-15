import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { Schedule } from "@/api/types";
import { localDateTimeIn } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { sample } from "@/test/fixtures";

const [once, everyAnchored, , daily, weekly, cron] = sample.schedules;
const none: SchedulePreview = { times: [], pending: false };

function setup(
  initial: Schedule | null,
  more: { stored?: Schedule | null; error?: string; preview?: SchedulePreview } = {},
) {
  let latest = initial;
  let changes = 0;
  function Host() {
    const [schedule, setSchedule] = useState(initial);
    latest = schedule;
    return (
      <ScheduleEditor
        schedule={schedule}
        stored={more.stored === undefined ? initial : more.stored}
        onChange={(s) => {
          changes += 1;
          setSchedule(s);
        }}
        error={more.error}
        preview={more.preview ?? none}
      />
    );
  }
  const { unmount } = render(<Host />);
  return { user: userEvent.setup(), schedule: () => latest, changes: () => changes, unmount };
}

const kind = () => screen.getByLabelText("Kind");

describe("the schedule editor", () => {
  it("offers every kind, and no schedule at first", async () => {
    const { user, schedule } = setup(null);
    expect(kind()).toHaveValue("none");
    expect(within(kind()).getAllByRole("option").map((o) => o.getAttribute("value"))).toEqual([
      "none",
      "once",
      "every",
      "daily",
      "weekly",
      "cron",
    ]);
    expect(screen.queryByLabelText("Catch up a missed run")).toBeNull();
    expect(screen.queryByRole("group", { name: "Stop" })).toBeNull();

    await user.selectOptions(kind(), "daily");
    expect(schedule()).toEqual({
      rule: { kind: "daily", time: "09:00" },
      catch_up: true,
      until: null,
    });
    expect(screen.getByLabelText("Time")).toHaveValue("09:00");
    expect(screen.getByLabelText("Catch up a missed run")).toBeChecked();
    expect(screen.getByRole("radio", { name: "Never" })).toBeChecked();

    await user.selectOptions(kind(), "none");
    expect(schedule()).toBeNull();
  });

  it("shows each stored kind as it is and changes nothing by showing it", () => {
    for (const s of sample.schedules) {
      const { changes, unmount } = setup(s);
      expect(kind()).toHaveValue(s.rule.kind);
      expect(changes()).toBe(0);
      unmount();
    }
  });

  it("sets the time and the days of a weekly schedule with toggles", async () => {
    const { user, schedule } = setup(weekly);
    const days = screen.getByRole("group", { name: "Days" });
    const toggles = within(days).getAllByRole("button");
    expect(toggles.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Monday",
      "Tuesday",
      "Wednesday",
      "Thursday",
      "Friday",
      "Saturday",
      "Sunday",
    ]);
    expect(toggles.filter((b) => b.getAttribute("aria-pressed") === "true")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Monday" }));
    await user.click(screen.getByRole("button", { name: "Wednesday" }));
    const wednesday = screen.getByRole("button", { name: "Wednesday" });
    expect(wednesday).toHaveAttribute("aria-pressed", "true");
    expect(schedule()?.rule).toEqual({ kind: "weekly", days: ["wed", "fri"], time: "07:30" });

    // By keyboard too.
    screen.getByRole("button", { name: "Sunday" }).focus();
    await user.keyboard(" ");
    expect(schedule()?.rule).toMatchObject({ days: ["wed", "fri", "sun"] });

    const time = screen.getByLabelText("Time");
    await user.clear(time);
    await user.type(time, "18:45");
    expect(schedule()?.rule).toMatchObject({ time: "18:45" });
    expect(schedule()?.until).toBeNull();
  });

  it("takes minutes or hours, and keeps the anchor while the minutes stay", async () => {
    const { user, schedule } = setup(everyAnchored);
    // 90 minutes are not whole hours.
    expect(screen.getByLabelText("Every")).toHaveValue(90);
    expect(screen.getByLabelText("Unit")).toHaveValue("minutes");

    const every = screen.getByLabelText("Every");
    await user.clear(every);
    await user.type(every, "2");
    await user.selectOptions(screen.getByLabelText("Unit"), "hours");
    expect(schedule()?.rule).toEqual({ kind: "every", minutes: 120, anchor: null });
    expect(screen.getByLabelText("Every")).toHaveValue(2);

    await user.clear(every);
    await user.type(every, "1.5");
    expect(schedule()).toEqual(everyAnchored);

    await user.clear(every);
    expect(schedule()?.rule).toMatchObject({ minutes: Number.NaN });
  });

  it("shows whole hours in hours", () => {
    setup({ ...everyAnchored, rule: { kind: "every", minutes: 180, anchor: null } });
    expect(screen.getByLabelText("Every")).toHaveValue(3);
    expect(screen.getByLabelText("Unit")).toHaveValue("hours");
  });

  it("asks a once schedule for its date only: no catch-up, no stop", async () => {
    const { user, schedule } = setup({ ...daily, catch_up: false });
    await user.selectOptions(kind(), "once");
    expect(schedule()).toEqual({
      rule: { kind: "once", at: localDateTimeIn(1, "09:00") },
      catch_up: true,
      until: null,
    });
    expect(screen.queryByLabelText("Catch up a missed run")).toBeNull();
    expect(screen.queryByRole("group", { name: "Stop" })).toBeNull();

    const at = screen.getByLabelText("Date and time");
    await user.clear(at);
    await user.type(at, "2030-01-02T07:05");
    expect(schedule()?.rule).toEqual({ kind: "once", at: "2030-01-02T07:05" });
  });

  it("stops never, after a number of runs or after a date", async () => {
    const { user, schedule } = setup(daily);
    expect(screen.getByRole("radio", { name: "After a date" })).toBeChecked();
    expect(screen.getByLabelText("Stop date")).toHaveValue("2026-12-31T18:00");

    await user.click(screen.getByRole("radio", { name: "After a number of runs" }));
    expect(schedule()?.until).toEqual({ count: 10 });
    const runs = screen.getByLabelText("Number of runs");
    await user.clear(runs);
    await user.type(runs, "3");
    expect(schedule()?.until).toEqual({ count: 3 });
    expect(screen.queryByLabelText("Stop date")).toBeNull();

    // The stored date comes back.
    await user.click(screen.getByRole("radio", { name: "After a date" }));
    expect(schedule()?.until).toEqual(daily.until);

    await user.click(screen.getByRole("radio", { name: "Never" }));
    expect(schedule()?.until).toBeNull();
    expect(screen.queryByLabelText("Number of runs")).toBeNull();
  });

  it("starts a new date stop a month ahead, at the end of the day", async () => {
    const { user, schedule } = setup(weekly);
    await user.click(screen.getByRole("radio", { name: "After a date" }));
    expect(schedule()?.until).toEqual({ date: localDateTimeIn(30, "23:59") });
    const date = screen.getByLabelText("Stop date");
    await user.clear(date);
    await user.type(date, "2031-05-06T12:00");
    expect(schedule()?.until).toEqual({ date: "2031-05-06T12:00" });
  });

  it("turns catch-up off and on", async () => {
    const { user, schedule } = setup(weekly);
    await user.click(screen.getByLabelText("Catch up a missed run"));
    expect(schedule()?.catch_up).toBe(false);
    expect(schedule()?.rule).toEqual(weekly.rule);
  });

  it("keeps the cron field as it was, and gives the stored rule back", async () => {
    const { user, schedule } = setup(cron);
    expect(screen.getByLabelText("Cron expression")).toHaveValue("0 0 2 * * *");
    // A cron schedule from before Scheduler v2 does not catch up.
    expect(screen.getByLabelText("Catch up a missed run")).not.toBeChecked();
    await user.selectOptions(kind(), "weekly");
    await user.selectOptions(kind(), "cron");
    expect(schedule()).toEqual(cron);
    const field = screen.getByLabelText("Cron expression");
    await user.clear(field);
    await user.type(field, "0 30 9 * * Mon-Fri");
    expect(schedule()?.rule).toEqual({ kind: "cron", expr: "0 30 9 * * Mon-Fri" });
  });

  it("shows the core's objection under the fields, and the next runs", () => {
    const times = ["2026-10-20T09:00:00+02:00"];
    setup(once, { error: "this schedule would never run", preview: { times, pending: false } });
    expect(screen.getByText("this schedule would never run")).toBeVisible();
    const next = screen.getByRole("list", { name: "Next runs" });
    expect(within(next).getByRole("listitem")).toHaveTextContent(localDateTime(times[0]));
  });
});
