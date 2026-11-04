import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { InboxKey } from "@/api/types";
import { InboxActions } from "@/components/InboxActions";
import { Toaster } from "@/components/ui/sonner";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";

const question: InboxKey = { kind: "message", id: "m1" };

function show(items: InboxKey[], done: boolean) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <InboxActions items={items} done={done} what="courier's question" />
      <Toaster />
    </QueryClientProvider>,
  );
  return userEvent.setup();
}

describe("Snooze and Done on an Inbox item", () => {
  beforeEach(() => {
    // Wednesday 4 November 2026, 10:00 local: the presets are fixed.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 10, 4, 10, 0));
    backend.on("inbox_snooze", () => null);
    backend.on("inbox_done", () => null);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("snoozes until a preset", async () => {
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Snooze: courier's question" }));
    expect(
      (await screen.findAllByRole("menuitem")).map((item) => item.textContent),
    ).toEqual([
      "In 1 hour",
      "This evening (6 PM)",
      "Tomorrow morning (9 AM)",
      "Next week (Monday 9 AM)",
      "Pick a date and time…",
    ]);
    await user.click(screen.getByRole("menuitem", { name: "Tomorrow morning (9 AM)" }));
    await waitFor(() =>
      expect(backend.callsTo("inbox_snooze")).toEqual([
        { items: [question], until: new Date(2026, 10, 5, 9, 0).toISOString() },
      ]),
    );
  });

  it("refuses a picked time that is not in the future", async () => {
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Snooze: courier's question" }));
    await user.click(await screen.findByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Snooze until" });
    const field = screen.getByLabelText("Date and time");
    await user.clear(field);
    await user.type(field, "2026-11-04T09:00");
    expect(screen.getByRole("alert")).toHaveTextContent("Pick a time in the future.");
    const confirm = screen.getAllByRole("button", { name: "Snooze" }).find((b) => dialog.contains(b));
    expect(confirm).toBeDisabled();

    await user.clear(field);
    await user.type(field, "2026-11-06T14:30");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(confirm).toBeEnabled();
    await user.click(confirm!);
    await waitFor(() =>
      expect(backend.callsTo("inbox_snooze")).toEqual([
        { items: [question], until: new Date(2026, 10, 6, 14, 30).toISOString() },
      ]),
    );
  });

  it("drops a preset that passed while the menu was open", async () => {
    vi.setSystemTime(new Date(2026, 10, 4, 17, 59, 40));
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Snooze: courier's question" }));
    const evening = await screen.findByRole("menuitem", { name: "This evening (6 PM)" });
    vi.setSystemTime(new Date(2026, 10, 4, 18, 0, 5));
    await user.click(evening);
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    expect(backend.callsTo("inbox_snooze")).toEqual([]);
  });

  it("checks a picked time again when Snooze is pressed", async () => {
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Snooze: courier's question" }));
    await user.click(await screen.findByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Snooze until" });
    const field = screen.getByLabelText("Date and time");
    await user.clear(field);
    await user.type(field, "2026-11-04T10:01");
    const confirm = within(dialog).getByRole("button", { name: "Snooze" });
    expect(confirm).toBeEnabled();
    // The minute passes before the button is pressed.
    vi.setSystemTime(new Date(2026, 10, 4, 10, 2));
    await user.click(confirm);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("Pick a time in the future.");
    expect(backend.callsTo("inbox_snooze")).toEqual([]);
  });

  it("gives focus back to Snooze when the dialog closes", async () => {
    const user = show([question], true);
    const trigger = screen.getByRole("button", { name: "Snooze: courier's question" });
    await user.click(trigger);
    await user.click(await screen.findByRole("menuitem", { name: "Pick a date and time…" }));
    const dialog = await screen.findByRole("dialog", { name: "Snooze until" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(trigger));
  });

  it("marks one item done", async () => {
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Done: courier's question" }));
    await waitFor(() => expect(backend.callsTo("inbox_done")).toEqual([{ item: question }]));
  });

  it("has no Done for approvals, and snoozes every approval of the group", async () => {
    const approvals: InboxKey[] = [
      { kind: "approval", id: "a1" },
      { kind: "approval", id: "a2" },
    ];
    const user = show(approvals, false);
    expect(screen.queryByRole("button", { name: /^Done/ })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Snooze: courier's question" }));
    await user.click(await screen.findByRole("menuitem", { name: "In 1 hour" }));
    await waitFor(() =>
      expect(backend.callsTo("inbox_snooze")).toEqual([
        { items: approvals, until: new Date(2026, 10, 4, 11, 0).toISOString() },
      ]),
    );
  });

  it("shows why a call was refused", async () => {
    backend.on("inbox_done", () => {
      throw { kind: "not_found", message: "That is no longer in the Inbox." };
    });
    const user = show([question], true);
    await user.click(screen.getByRole("button", { name: "Done: courier's question" }));
    expect(await screen.findByText("That is no longer in the Inbox.")).toBeVisible();
  });
});
