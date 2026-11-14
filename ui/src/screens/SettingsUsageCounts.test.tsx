import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { UsageCountsState } from "@/api/types";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";

const OFF: UsageCountsState = {
  enabled: false,
  url: null,
  install_id: null,
  first_hour: null,
  last_sent_hour: null,
  last_sent_at: null,
  last_error: null,
};

const URL = "https://counts.example.com/v1/hourly";

const REPORT = {
  schema: 1,
  install_id: "3f2b8c1e-5d4a-4b6f-9e7d-0a1b2c3d4e5f",
  app_version: "0.3.0",
  hour_start: "2026-11-05T10:00:00Z",
  runs_started: 4,
  runs_succeeded: 3,
  runs_failed: 1,
  messages_sent: 2,
  approvals_asked: 1,
  dots: 5,
};

/// Settings, with the core keeping the usage counts' state as it is set.
function settings(start: UsageCountsState = OFF) {
  let current = start;
  backend.on("usage_counts_state", () => current);
  backend.on("set_usage_counts", (args) => {
    const url = (args.url as string | null) ?? current.url;
    current = {
      ...current,
      enabled: args.enabled as boolean,
      url,
      install_id: current.install_id ?? (args.enabled ? REPORT.install_id : null),
      last_error: args.enabled ? current.last_error : null,
    };
    return current;
  });
  backend.on("reset_usage_counts_id", () => {
    current = { ...current, install_id: "0a0b0c0d-1111-4222-8333-444455556666" };
    return current;
  });
  backend.on("usage_counts_preview", () => REPORT);
  return renderApp("/settings/help");
}

const section = () => screen.findByRole("group", { name: "Share usage counts" });

describe("Share usage counts", () => {
  it("is off by default and says exactly what is and isn't sent", async () => {
    settings();
    const s = await section();
    const toggle = within(s).getByRole("switch", { name: "Send usage counts every hour" });
    expect(toggle).not.toBeChecked();
    expect(s).toHaveTextContent(
      "Only counts: how many runs started, finished or failed, messages sent and approvals asked each hour, and how many sols you have. Never names, files, messages or anything a sol wrote.",
    );
    expect(within(s).getByText("Not sent yet")).toBeVisible();
  });

  it("can't be turned on without a valid https address", async () => {
    const { user } = settings();
    const s = await section();
    const toggle = within(s).getByRole("switch", { name: "Send usage counts every hour" });
    expect(toggle).toBeDisabled();
    const field = within(s).getByLabelText("Send to");
    await user.type(field, "http://counts.example.com/");
    expect(toggle).toBeDisabled();
    expect(
      within(s).getByText(
        "The address must start with https:// (http:// is allowed only for localhost or 127.0.0.1).",
      ),
    ).toBeVisible();
    await user.clear(field);
    await user.type(field, URL);
    expect(toggle).toBeEnabled();
    await user.click(toggle);
    await waitFor(() => expect(toggle).toBeChecked());
    expect(backend.callsTo("set_usage_counts")).toEqual([{ enabled: true, url: URL }]);
    expect(await within(s).findByText(REPORT.install_id)).toBeVisible();
  });

  it("allows http only for this PC", async () => {
    const { user } = settings();
    const s = await section();
    await user.type(within(s).getByLabelText("Send to"), "http://localhost:8080/counts");
    expect(within(s).getByRole("switch", { name: "Send usage counts every hour" })).toBeEnabled();
  });

  it("turns off at once, keeping the address", async () => {
    const { user } = settings({ ...OFF, enabled: true, url: URL, install_id: REPORT.install_id });
    const s = await section();
    const toggle = within(s).getByRole("switch", { name: "Send usage counts every hour" });
    await waitFor(() => expect(toggle).toBeChecked());
    await user.click(toggle);
    await waitFor(() => expect(toggle).not.toBeChecked());
    expect(backend.callsTo("set_usage_counts")).toEqual([{ enabled: false, url: null }]);
    expect(within(s).getByLabelText("Send to")).toHaveValue(URL);
  });

  it("shows the exact JSON of the next report", async () => {
    const { user } = settings({ ...OFF, enabled: true, url: URL, install_id: REPORT.install_id });
    const s = await section();
    await user.click(within(s).getByRole("button", { name: "Show what gets sent" }));
    const shown = await within(s).findByLabelText("The next report");
    await waitFor(() => expect(shown.textContent).toBe(JSON.stringify(REPORT, null, 2)));
  });

  it("shows the last send, the last error and resets the ID", async () => {
    const { user } = settings({
      ...OFF,
      enabled: true,
      url: URL,
      install_id: REPORT.install_id,
      last_sent_at: "2026-11-05T11:02:00Z",
      last_error: "the server answered 503",
    });
    const s = await section();
    expect(await within(s).findByRole("alert")).toHaveTextContent(
      "Last send failed: the server answered 503",
    );
    expect(within(s).queryByText("Not sent yet")).toBeNull();
    expect(within(s).getByText(new Date("2026-11-05T11:02:00Z").toLocaleString())).toBeVisible();
    await user.click(within(s).getByRole("button", { name: "Reset ID" }));
    expect(await within(s).findByText("0a0b0c0d-1111-4222-8333-444455556666")).toBeVisible();
    expect(backend.callsTo("reset_usage_counts_id")).toHaveLength(1);
  });

  it("shows why a change was refused", async () => {
    const { user } = settings({ ...OFF, url: URL });
    backend.on("set_usage_counts", () => {
      throw { kind: "invalid", message: "Enter the address to send the counts to first." };
    });
    const s = await section();
    await user.click(within(s).getByRole("switch", { name: "Send usage counts every hour" }));
    expect(
      await within(s).findByText("Enter the address to send the counts to first."),
    ).toBeVisible();
  });
});
