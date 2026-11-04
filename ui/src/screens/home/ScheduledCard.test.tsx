import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ScheduledMessage } from "@/api/types";
import { localWeekdayTime } from "@/lib/format";
import { topicOf } from "@/lib/home-topics";
import { fromPicker, toPicker } from "@/lib/later";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat } from "@/test/home";
import {
  editTime,
  ScheduledCard,
  ScheduledCards,
  scheduledTarget,
} from "@/screens/home/ScheduledCard";

const NOW = new Date(2026, 10, 4, 10, 0);
const MIN = 60_000;
const at = (ms: number) => new Date(NOW.getTime() + ms).toISOString();

const row = (over: Partial<ScheduledMessage> = {}): ScheduledMessage => ({
  ...sample.scheduledMessage,
  root_run_id: "a",
  waiting: null,
  due_at: at(2 * 60 * MIN + 13 * MIN + 10_000),
  ...over,
});

function show(message: ScheduledMessage, target = "current topic") {
  const qc = createQueryClient();
  render(
    <QueryClientProvider client={qc}>
      <ScheduledCard message={message} dotName="nightly" target={target} />
    </QueryClientProvider>,
  );
  return { card: screen.getByRole("region", { name: "Scheduled message" }), qc };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("a scheduled message's card", () => {
  it("counts down to its time", () => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
    const message = row();
    const { card } = show(message);
    expect(card).toHaveTextContent(
      `Scheduled for ${localWeekdayTime(message.due_at, NOW.getTime())} · current topic`,
    );
    expect(card).toHaveTextContent(message.text);
    expect(card).toHaveTextContent("Sends in 2h 14m");
    for (const name of ["Send now", "Edit", "Cancel"]) {
      expect(within(card).getByRole("button", { name })).toBeVisible();
    }
    act(() => vi.advanceTimersByTime(2 * 60 * MIN + 13 * MIN));
    expect(card).toHaveTextContent("Sends in 1m");
  });

  it("says why a due message has not gone, or that it was not sent", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
    const cases: [Partial<ScheduledMessage>, string][] = [
      [{ waiting: "busy", due_at: at(-MIN) }, "Due · waiting for nightly to finish this topic"],
      [{ waiting: "approval", due_at: at(-MIN) }, "Due · waiting for your approval in this topic"],
      [{ waiting: "usage_limit", due_at: at(-MIN) }, "Due · waiting for the usage limit to reset"],
      [{ due_at: at(-MIN) }, "Sending…"],
    ];
    for (const [over, text] of cases) {
      const { card } = show(row(over));
      expect(card).toHaveTextContent(text);
      card.parentElement!.remove();
    }
    const { card: sending } = show(row({ status: "sending", due_at: at(-MIN) }));
    expect(sending).toHaveTextContent("Sending…");
    expect(within(sending).queryByRole("button")).toBeNull();
    sending.parentElement!.remove();

    const { card: failed } = show(row({ status: "failed", error: "org removed" }));
    const line = within(failed).getByText("Not sent: org removed");
    expect(line).toHaveClass("text-needs");
    expect(within(failed).getByRole("button", { name: "Send now" })).toBeVisible();
  });

  it("calls the core from Send now, Edit and Cancel", async () => {
    // Before its time, so the edit starts with it.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
    const message = row();
    backend.on("send_scheduled_now", () => ({ ...message, status: "sent" }));
    backend.on("cancel_scheduled_message", () => ({ ...message, status: "cancelled" }));
    backend.on("edit_scheduled_message", (args) => ({
      ...message,
      text: args.text as string,
      due_at: args.dueAt as string,
    }));
    const user = userEvent.setup();
    const { card } = show(message);
    await user.click(within(card).getByRole("button", { name: "Send now" }));
    await waitFor(() => expect(backend.callsTo("send_scheduled_now")).toEqual([{ id: message.id }]));
    await user.click(within(card).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(backend.callsTo("cancel_scheduled_message")).toEqual([{ id: message.id }]),
    );

    await user.click(within(card).getByRole("button", { name: "Edit" }));
    const text = within(card).getByLabelText("Scheduled message");
    expect(text).toHaveValue(message.text);
    const field = within(card).getByLabelText("Send at");
    expect(field).toHaveValue(toPicker(new Date(message.due_at)));
    await user.clear(text);
    await user.type(text, "Ship it today");
    await user.click(within(card).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("edit_scheduled_message")).toEqual([
        {
          id: message.id,
          text: "Ship it today",
          dueAt: fromPicker(toPicker(new Date(message.due_at)))!.toISOString(),
        },
      ]),
    );
    await waitFor(() => expect(within(card).queryByLabelText("Send at")).toBeNull());
  });

  it("starts the edit of a message already due an hour from now", async () => {
    const message = row({ waiting: "busy", due_at: new Date(Date.now() - 5 * MIN).toISOString() });
    const user = userEvent.setup();
    const { card } = show(message);
    const opened = Date.now();
    await user.click(within(card).getByRole("button", { name: "Edit" }));
    const field = within(card).getByLabelText("Send at");
    const shown = fromPicker((field as HTMLInputElement).value)!.getTime();
    // An hour from when the form opened, to the minute.
    expect(shown).toBeGreaterThanOrEqual(opened + 59 * MIN);
    expect(shown).toBeLessThanOrEqual(Date.now() + 60 * MIN);
  });

  it("keeps a message's own time for the edit unless it is due or within two minutes", () => {
    const now = NOW.getTime();
    expect(editTime(at(3 * MIN), now)).toEqual(new Date(at(3 * MIN)));
    expect(editTime(at(90_000), now)).toEqual(new Date(now + 60 * MIN));
    expect(editTime(at(-MIN), now)).toEqual(new Date(now + 60 * MIN));
  });

  it("discards an edit", async () => {
    const message = row();
    const user = userEvent.setup();
    const { card } = show(message);
    await user.click(within(card).getByRole("button", { name: "Edit" }));
    await user.type(within(card).getByLabelText("Scheduled message"), " now");
    await user.click(within(card).getByRole("button", { name: "Discard changes" }));
    expect(within(card).queryByLabelText("Scheduled message")).toBeNull();
    expect(card).toHaveTextContent(message.text);
    expect(backend.callsTo("edit_scheduled_message")).toEqual([]);
  });

  it("shows why an edit lost, and goes once the core no longer lists it", async () => {
    const message = row();
    let rows = [message];
    backend.on("scheduled_messages", () => rows);
    backend.on("edit_scheduled_message", () => {
      rows = [];
      throw { kind: "conflict", message: "This message has already been sent." };
    });
    const qc = createQueryClient();
    const topics = [topicOf([chat("a", "10:00")])];
    render(
      <QueryClientProvider client={qc}>
        <ScheduledCards dotId={message.dot_id} dotName="nightly" topics={topics} />
      </QueryClientProvider>,
    );
    const user = userEvent.setup();
    const card = await screen.findByRole("region", { name: "Scheduled message" });
    await user.click(within(card).getByRole("button", { name: "Edit" }));
    await user.click(within(card).getByRole("button", { name: "Save" }));
    // The card asks for the list again, which no longer has it: it gives way to the sent run.
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Scheduled message" })).toBeNull(),
    );
    expect(backend.callsTo("edit_scheduled_message")).toHaveLength(1);
  });

  it("keeps the edit open with the reason when the list still has it", async () => {
    const message = row();
    backend.on("scheduled_messages", () => [message]);
    backend.on("edit_scheduled_message", () => {
      throw { kind: "conflict", message: "This message is being sent now." };
    });
    const qc = createQueryClient();
    render(
      <QueryClientProvider client={qc}>
        <ScheduledCards dotId={message.dot_id} dotName="nightly" topics={[]} />
      </QueryClientProvider>,
    );
    const user = userEvent.setup();
    const card = await screen.findByRole("region", { name: "Scheduled message" });
    await user.click(within(card).getByRole("button", { name: "Edit" }));
    await user.click(within(card).getByRole("button", { name: "Save" }));
    expect(await within(card).findByRole("alert")).toHaveTextContent(
      "This message is being sent now.",
    );
    expect(within(card).getByLabelText("Scheduled message")).toBeVisible();
  });

  it("shows only an older topic's own cards", async () => {
    backend.on("scheduled_messages", () => [
      row({ id: "s1", root_run_id: "old", text: "For the old topic" }),
      row({ id: "s2", root_run_id: "a", text: "For the current topic" }),
      row({ id: "s3", root_run_id: null, text: "For a new topic" }),
    ]);
    const qc = createQueryClient();
    render(
      <QueryClientProvider client={qc}>
        <ScheduledCards
          dotId={sample.scheduledMessage.dot_id}
          dotName="nightly"
          topics={[topicOf([chat("old", "08:00")])]}
          root="old"
        />
      </QueryClientProvider>,
    );
    const card = await screen.findByRole("region", { name: "Scheduled message" });
    expect(card).toHaveTextContent("For the old topic");
    expect(screen.getAllByRole("region", { name: "Scheduled message" })).toHaveLength(1);
  });
});

describe("scheduledTarget", () => {
  it("names the topic a message goes to", () => {
    const old = topicOf([chat("old", "08:00")]);
    const current = topicOf([chat("a", "10:00")]);
    const topics = [old, current];
    expect(scheduledTarget(row({ root_run_id: null }), topics, "a")).toBe("new topic");
    expect(scheduledTarget(row({ root_run_id: "a" }), topics, "a")).toBe("current topic");
    expect(scheduledTarget(row({ root_run_id: "old" }), topics, "a")).toBe("question old");
    expect(scheduledTarget(row({ root_run_id: "gone" }), topics, "a")).toBe("an older topic");
  });
});
