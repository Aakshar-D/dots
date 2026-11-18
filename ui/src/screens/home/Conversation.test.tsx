import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Approval, Message, Run, RunEventRecord } from "@/api/types";
import { localDayTime, localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, overview, reply, said, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const conversation = () => screen.getByRole("region", { name: "Conversation" });
/// The card of the scheduled run `run`, once it shows.
const card = async (run: Run) =>
  within(await screen.findByRole("region", { name: "Conversation" })).findByRole("article", {
    name: `Scheduled · ${localTime(run.queued_at)}`,
  });

describe("a sol's conversation", () => {
  it("shows the current chat topic and the run topics after it, in order", async () => {
    const old = chat("old", "08:00");
    const early = scheduled("early", "09:00");
    const current = chat("current", "10:00");
    const later = scheduled("later", "11:00");
    home(
      [old, early, current, later, reply("current2", current, "12:00")],
      [said("current", 1, "Answer to current.")],
    );
    renderApp(`/home/${dot.id}`);
    expect(await within(await screen.findByRole("region", { name: "Conversation" })).findByText("Answer to current.")).toBeVisible();
    const view = within(conversation());
    expect(view.getByText("question current")).toBeVisible();
    expect(view.getByText("reply current2")).toBeVisible();
    const laterCard = await card(later);
    expect(within(laterCard).getByText("Summary of later")).toBeVisible();
    // The transcript comes before the card.
    expect(
      view.getByText("question current").compareDocumentPosition(laterCard) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // Older topics are not in the conversation.
    expect(view.queryByText("question old")).toBeNull();
    expect(view.queryByRole("article", { name: `Scheduled · ${localTime(early.queued_at)}` })).toBeNull();
  });

  it("shows the working line while a chat reply is being written, and drops it once it ends", async () => {
    const current = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = home([current]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText(`${dot.name} is working`)).toBeInTheDocument();

    runs[0] = { ...current, status: "succeeded", ended_at: current.queued_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    await waitFor(() => expect(view.queryByText(`${dot.name} is working`)).toBeNull());
  });

  it("shows a closed run card without loading its events", async () => {
    const run = scheduled("cron", "09:00", { summary: "Sent 3 invoices\nto the usual people" });
    home([run]);
    const { listening } = renderApp(`/home/${dot.id}`);
    const shown = await card(run);
    await listening();
    // A rounded light-bordered card, not a rule on its left.
    expect(shown).toHaveClass("rounded-xl", "border", "bg-card");
    expect(within(shown).getByText("Succeeded")).toBeVisible();
    expect(within(shown).getByText("Sent 3 invoices")).toBeVisible();
    expect(within(shown).getByRole("link", { name: "Details" })).toHaveAttribute(
      "href",
      `/runs/${run.id}`,
    );
    expect(backend.callsTo("list_run_events")).toEqual([]);
    expect(backend.callsTo("approvals_for_run")).toEqual([]);
  });

  it("says why a run failed on its card", async () => {
    const run = scheduled("cron", "09:00", { status: "failed", summary: null, error: "timeout\nafter 600 s" });
    home([run]);
    renderApp(`/home/${dot.id}`);
    expect(within(await card(run)).getByText("timeout")).toBeVisible();
  });

  it("opens a run card that waits for an approval and answers it there", async () => {
    const run = scheduled("cron", "09:00", { status: "awaiting_approval", ended_at: null, summary: null });
    home([run], [], [{ ...sample.approval, run_id: run.id }]);
    backend.on("decide_approval", () => ({ ...sample.approval, run_id: run.id, status: "approved" }));
    const { user } = renderApp(`/home/${dot.id}`);
    const shown = await card(run);
    const approval = await within(shown).findByRole("article", { name: "Approval for Bash" });
    await user.click(within(approval).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: sample.approval.id, approved: true, note: null, allowRule: null },
      ]),
    );
  });

  it("opens a run card whose run asked a question", async () => {
    const run = scheduled("cron", "09:00");
    const question: Message = { ...sample.messages[0], run_id: run.id };
    home([run]);
    backend.on("messages_list", () => [question]);
    renderApp(`/home/${dot.id}`);
    const asked = await within(await card(run)).findByRole("group", { name: "Question from the sol" });
    expect(asked).toHaveTextContent(question.text);
    expect(asked).toHaveTextContent("Question · waiting for your answer");
  });

  it("shows the replies to a run topic under its card", async () => {
    const run = scheduled("cron", "09:00");
    home([run, reply("r1", run, "09:30")], [said("r1", 1, "Done, as you asked.")]);
    renderApp(`/home/${dot.id}`);
    await card(run);
    const view = within(conversation());
    expect(await view.findByText("Done, as you asked.")).toBeVisible();
    expect(view.getByText("reply r1")).toBeVisible();
  });

  it("shows skipped slots among the topics in time order, apart from the messages", async () => {
    const early = scheduled("early", "09:00");
    const later = scheduled("later", "11:00");
    home([early, later]);
    const skip = { ...sample.scheduleSkip, dot_id: dot.id };
    backend.on("schedule_skips", () => [
      { ...skip, id: "s2", slot_at: "2026-10-09T12:00:00Z" },
      { ...skip, id: "s1", slot_at: "2026-10-09T10:00:00Z" },
    ]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const lines = await view.findAllByText(/Schedule skipped at .*: still working on the previous run/);
    expect(lines).toHaveLength(2);
    const first = await card(early);
    const second = await card(later);
    const after = (a: Node, b: Node) => a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING;
    // early, the 10:00 skip, later, the 12:00 skip.
    expect(after(first, lines[0])).toBeTruthy();
    expect(after(lines[0], second)).toBeTruthy();
    expect(after(second, lines[1])).toBeTruthy();
    expect(backend.callsTo("schedule_skips")).toEqual([{ dotId: dot.id, limit: undefined }]);
  });

  it("never counts skipped slots as unread, in the column or on the rail", async () => {
    const run = scheduled("cron", "09:00");
    home([run]);
    const state = sample.overview.schedule_state!;
    // Known counts: 2 unread on the dot, 2 in the Home total, 3 skips since the last run.
    backend.on("dot_overview", () => [
      {
        ...overview,
        unread: 2,
        schedule_state: { ...state, last_skipped_at: "2026-10-09T10:00:00Z", skipped_since_run: 3 },
      },
    ]);
    backend.on("unread_total", () => 2);
    const skip = { ...sample.scheduleSkip, dot_id: dot.id };
    const skips = [1, 2, 3].map((n) => ({ ...skip, id: `s${n}`, slot_at: `2026-10-09T10:0${n}:00Z` }));
    backend.on("schedule_skips", () => skips);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findAllByText(/Schedule skipped at/)).toHaveLength(3);
    const counts = () => {
      const rail = screen.getByRole("navigation", { name: "Main" });
      const column = screen.getByRole("navigation", { name: "Sols" });
      return [
        within(rail).getByRole("link", { name: /^Home, \d+ unread$/ }).getAttribute("aria-label"),
        within(column).getByText(/^\d+ unread$/).textContent,
      ];
    };
    await waitFor(() => expect(counts()).toEqual(["Home, 2 unread", "2 unread"]));

    // A new skip arrives: the skip lines and the overview are read again, the counts stay.
    const before = backend.callsTo("schedule_skips").length;
    act(() => backend.emit("runtime", { type: "schedule_skipped", dot_id: dot.id, skip: skips[0] }));
    await waitFor(() => expect(backend.callsTo("schedule_skips").length).toBeGreaterThan(before));
    await waitFor(() => expect(backend.callsTo("dot_overview").length).toBeGreaterThan(1));
    expect(counts()).toEqual(["Home, 2 unread", "2 unread"]);
  });

  it("tells the last skipped slot in the sol's header, and nothing without one", async () => {
    home([scheduled("cron", "09:00")]);
    const state = sample.overview.schedule_state!;
    const last = "2026-10-09T09:00:00Z";
    backend.on("dot_overview", () => [
      { ...overview, schedule_state: { ...state, last_skipped_at: last, skipped_since_run: 2 } },
    ]);
    // Only the clock is faked (to the day of the skip), so nothing waits.
    vi.useFakeTimers({ toFake: ["Date"], now: Date.parse("2026-10-09T11:00:00Z") });
    try {
      renderApp(`/home/${dot.id}`);
      expect(
        await screen.findByText(`Last skipped ${localTime(last)} · 2 skipped since the last run`),
      ).toBeVisible();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows a failed load of the skipped slots, as it does for the proposals", async () => {
    home([scheduled("cron", "09:00")]);
    backend.on("schedule_skips", () => {
      throw { kind: "internal", message: "could not read the skipped slots" };
    });
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("could not read the skipped slots")).toBeVisible();
  });

  it("shows no skip line in the header of a sol that never skipped", async () => {
    const run = scheduled("cron", "09:00");
    home([run]);
    const state = sample.overview.schedule_state!;
    backend.on("dot_overview", () => [
      { ...overview, schedule_state: { ...state, last_skipped_at: null, skipped_since_run: 0 } },
    ]);
    renderApp(`/home/${dot.id}`);
    await card(run);
    expect(screen.queryByText(/Last skipped/)).toBeNull();
  });

  it("shows the last 20 run topics of a sol that never chatted", async () => {
    home(
      Array.from({ length: 25 }, (_, i) =>
        scheduled(`r${i}`, `0${Math.floor(i / 10)}:${String((i % 10) * 5).padStart(2, "0")}`),
      ),
    );
    renderApp(`/home/${dot.id}`);
    await within(await screen.findByRole("region", { name: "Conversation" })).findAllByRole("article");
    const cards = within(conversation()).getAllByRole("article");
    expect(cards).toHaveLength(20);
    expect(within(cards.at(-1)!).getByText("Summary of r24")).toBeVisible();
    expect(within(conversation()).queryByText("Summary of r4")).toBeNull();
  });

  it("adds a new run of the sol at the end, and changes a status in place", async () => {
    const current = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = home([current]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("question a")).toBeVisible();

    const fired = scheduled("cron", "11:00", { status: "running", ended_at: null, summary: null });
    runs.push(fired);
    act(() => backend.emit("runtime", { type: "run_updated", run: fired }));
    const shown = await card(fired);
    expect(within(shown).getByText("Running")).toBeVisible();

    runs[1] = { ...fired, status: "succeeded", summary: "All good." };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[1] }));
    expect(await within(shown).findByText("Succeeded")).toBeVisible();
    expect(within(shown).getByText("All good.")).toBeVisible();
  });

  it("follows new activity at the bottom, and offers it after the person scrolled up", async () => {
    const runs = home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    const view = await screen.findByRole("region", { name: "Conversation" });
    await within(view).findByText("question a");
    // jsdom lays nothing out: the conversation is 1000 px tall in a 200 px window.
    let top = 800;
    Object.defineProperty(view, "scrollHeight", { configurable: true, value: 1000 });
    Object.defineProperty(view, "clientHeight", { configurable: true, value: 200 });
    Object.defineProperty(view, "scrollTop", {
      configurable: true,
      get: () => top,
      set: (value: number) => {
        top = value;
      },
    });
    const arrive = (run: Run) => {
      runs.push(run);
      act(() => backend.emit("runtime", { type: "run_updated", run }));
    };

    top = 790;
    arrive(scheduled("one", "11:00"));
    await card(runs[1]);
    await waitFor(() => expect(top).toBe(1000));
    expect(screen.queryByRole("button", { name: /New activity/ })).toBeNull();

    top = 100;
    fireEvent.scroll(view);
    arrive(scheduled("two", "12:00"));
    await card(runs[2]);
    const jump = await screen.findByRole("button", { name: /New activity/ });
    expect(top).toBe(100);
    await user.click(jump);
    expect(top).toBe(1000);
    expect(screen.queryByRole("button", { name: /New activity/ })).toBeNull();
  });

  it("shows an older topic alone, with the way back", async () => {
    const old = chat("old", "08:00");
    home([old, reply("old2", old, "08:30"), chat("current", "10:00")]);
    const { user, router } = renderApp(`/home/${dot.id}/old`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("question old")).toBeVisible();
    expect(view.getByText("reply old2")).toBeVisible();
    expect(view.queryByText("question current")).toBeNull();
    await user.click(screen.getByRole("link", { name: "Back to the current conversation" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
    expect(await screen.findByText("question current")).toBeVisible();
  });

  it("says under a message that was sent later whether it went late, and a new topic", async () => {
    const late = chat("late", "10:00", {
      payload: {
        message: "Ship it",
        typed: true,
        send_later: { id: "s1", due_at: "2026-10-09T09:00:00.000Z", late: true, new_topic: true },
      },
    });
    home([late]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const due = localDayTime("2026-10-09T09:00:00.000Z", Date.parse(late.queued_at));
    expect(await view.findByText(`Sent late (was due ${due})`)).toBeVisible();
    expect(
      view.getByText("Started a new topic: the one it was written in can't be continued."),
    ).toBeVisible();
  });

  it("shows the scheduled messages after the topics, and does not greet a sol with one", async () => {
    home([]);
    backend.on("scheduled_messages", () => [{ ...sample.scheduledMessage, root_run_id: null }]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const card = await view.findByRole("region", { name: "Scheduled message" });
    expect(card).toHaveTextContent(sample.scheduledMessage.text);
    expect(card).toHaveTextContent("· new topic");
    expect(screen.queryByText(`Ask ${dot.name} something`)).toBeNull();
  });

  it("greets with the sol's name over the composer when the conversation is empty", async () => {
    home([]);
    renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("heading", { level: 2, name: dot.name });
    expect(name).toHaveClass("text-2xl", "font-semibold");
    const invite = screen.getByText(`Ask ${dot.name} something`);
    expect(invite).toHaveClass("text-muted-foreground");
    // The old line is gone, and there is no conversation to scroll.
    expect(screen.queryByText(/It answers with its own instructions/)).toBeNull();
    expect(screen.queryByRole("region", { name: "Conversation" })).toBeNull();
    // The composer is right under the greeting.
    const box = screen.getByLabelText("Message");
    expect(invite.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send" })).toBeVisible();
  });

  it("does not greet when the sol's runs could not be loaded", async () => {
    home([]);
    backend.on("list_runs", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("alert")).toHaveTextContent("database is locked");
    expect(screen.queryByText(`Ask ${dot.name} something`)).toBeNull();
  });

  it("turns the greeting into the conversation with the first message", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    await screen.findByText(`Ask ${dot.name} something`);
    const box = screen.getByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "Hello there{Enter}");
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Hello there")).toBeVisible();
    expect(screen.queryByText(`Ask ${dot.name} something`)).toBeNull();
    expect(screen.queryByRole("heading", { level: 2, name: dot.name })).toBeNull();
  });

  it("shows a failure once when the run also stored it as an event", async () => {
    const failed: RunEventRecord = {
      run_id: "a",
      seq: 2,
      ts: sample.run.queued_at,
      kind: "failed",
      data: { kind: "failed", error: "engine: boom" },
    };
    home(
      [chat("a", "10:00", { status: "failed", error: "engine: boom" })],
      [said("a", 1, "Trying."), failed],
    );
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Trying.")).toBeVisible();
    expect(view.getAllByRole("alert")).toHaveLength(1);
    expect(view.getByRole("alert")).toHaveTextContent("engine: boom");
  });

  it("says a message was resent after Sage restarted, and not for one sent once", async () => {
    home([
      chat("a", "10:00", { status: "failed", error: "interrupted" }),
      chat("b", "10:01", {
        root_run_id: "a",
        retry_of: "a",
        payload: { message: "question a", resent: "restart" },
      }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findAllByText("question a")).toHaveLength(2);
    expect(view.getAllByText("Resent after Sage restarted")).toHaveLength(1);
    // Only the topic's last run is failed-and-last; the resend succeeded, so no Send again.
    expect(view.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("offers one way to send a message that never got there again: its Retry", async () => {
    home([
      chat("a", "10:00", {
        status: "failed",
        error: "interrupted",
        delivered_at: null,
        undelivered_at: "2026-10-09T10:00:30.000Z",
        undelivered_reason: "interrupted",
        dismissed_at: null,
        delivery: "not_delivered",
      }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("button", { name: "Retry" })).toBeVisible();
    expect(view.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("offers to send a failed message again", async () => {
    home([
      chat("a", "10:00", {
        status: "failed",
        error: "interrupted",
        delivered_at: "2026-10-09T10:00:05.000Z",
        delivery: "delivered",
      }),
    ]);
    backend.on("resend_run", () =>
      chat("b", "10:05", { status: "queued", root_run_id: "a", retry_of: "a" }),
    );
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    fireEvent.click(await view.findByRole("button", { name: "Send again" }));
    await waitFor(() => expect(backend.callsTo("resend_run")).toEqual([{ runId: "a" }]));
  });

  it("does not offer to send again a failed message of a workflow step", async () => {
    home([
      chat("a", "10:00", { status: "failed", error: "interrupted", workflow_step_run_id: "step-1" }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("interrupted")).toBeVisible();
    expect(view.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("does not offer to send again a failed run that was not a message", async () => {
    const run = scheduled("a", "10:00", { status: "failed", summary: null, error: "interrupted" });
    home([run]);
    renderApp(`/home/${dot.id}`);
    expect(within(await card(run)).getByText("interrupted")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("shows a failed load of a topic's events and keeps what did load", async () => {
    home([chat("a", "10:00")]);
    backend.on("list_run_events", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("alert")).toHaveTextContent("database is locked");
    expect(view.getByText("question a")).toBeVisible();
  });

  it("shows a failed load of the sol's runs", async () => {
    home([]);
    backend.on("list_runs", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("alert")).toHaveTextContent("database is locked");
  });

  it("says what became of a call Sage performed", async () => {
    const performed: Approval = {
      ...sample.approval,
      run_id: "a",
      status: "approved",
      parked: true,
      self_apply: true,
      apply_state: "applied",
    };
    home([chat("a", "10:00")], [], [performed]);
    renderApp(`/home/${dot.id}`);
    const calls = await screen.findByRole("list", { name: "Approved calls" });
    expect(within(calls).getByText("Performed by Sage")).toBeVisible();
  });

  it("names the sol in the rules a pending call in a chat topic offers", async () => {
    const waiting = chat("a", "10:00", { status: "awaiting_approval", ended_at: null });
    home([waiting], [], [{ ...sample.approval, run_id: "a" }]);
    backend.on("approval_rule_candidates", () => sample.rules);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: "Approve and always allow" }));
    expect(screen.getByRole("group", { name: `Always allow for ${dot.name}` })).toBeVisible();
  });

  it("shows the sol's browser calls as lines", async () => {
    const events: RunEventRecord[] = [
      {
        run_id: "a",
        seq: 1,
        ts: sample.run.queued_at,
        kind: "tool_call",
        data: {
          kind: "tool_call",
          id: "b1",
          tool: "mcp__browser__browser_navigate",
          input: { url: "https://shop.com/cart" },
        },
      },
      {
        run_id: "a",
        seq: 2,
        ts: sample.run.queued_at,
        kind: "tool_result",
        data: {
          kind: "tool_result",
          id: "b1",
          output: "Sage: Opened shop.com/cart\n### Page",
          is_error: false,
        },
      },
    ];
    home([chat("a", "10:00")], events);
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("Opened shop.com/cart")).toBeVisible();
  });
});

describe("the run's plan in the conversation", () => {
  const event = (seq: number, data: Record<string, unknown>): RunEventRecord => ({
    run_id: "a",
    seq,
    ts: sample.run.queued_at,
    kind: String(data.kind),
    data,
  });
  const todos = (...steps: [string, string][]) =>
    steps.map(([content, status]) => ({ content, status, activeForm: `Doing ${content}` }));

  it("shows the sol's plan above its reply and follows it live", async () => {
    const events = [
      event(1, {
        kind: "tool_call",
        id: "p1",
        tool: "TodoWrite",
        input: { todos: todos(["Read", "in_progress"], ["Fix", "pending"]) },
      }),
      event(2, { kind: "assistant_text", text: "On it." }),
    ];
    home([chat("a", "10:00", { status: "running", ended_at: null })], events);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const card = await view.findByRole("region", { name: `Plan of ${dot.name}` });
    expect(within(card).getByText("Doing Read…")).toBeVisible();
    // The card stands above the dot's words.
    expect(
      card.compareDocumentPosition(view.getByText("On it.")) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // A plan update the core sends live moves the card on.
    const next = event(3, {
      kind: "tool_call",
      id: "p2",
      tool: "TodoWrite",
      input: { todos: todos(["Read", "completed"], ["Fix", "in_progress"]) },
    });
    events.push(next);
    act(() => backend.emit("runtime", { type: "run_event", event: next }));
    expect(await within(card).findByText("Doing Fix…")).toBeVisible();
    expect(within(card).getByText("1 of 2 steps done")).toBeInTheDocument();
  });
});

describe("what the self-QA found", () => {
  const event = (runId: string, seq: number, data: Record<string, unknown>): RunEventRecord => ({
    run_id: runId,
    seq,
    ts: sample.run.queued_at,
    kind: String(data.kind),
    data,
  });

  it("says a call parked for approval was sent for approval, not that it failed", async () => {
    home(
      [chat("a", "10:00", { status: "awaiting_approval", ended_at: null })],
      [
        event("a", 1, { kind: "tool_call", id: "t", tool: "Bash", input: { command: "echo hi" } }),
        event("a", 2, {
          kind: "tool_result",
          id: "t",
          output:
            "Queued for human approval #01APPROVAL. Do not retry this action. Finish any other work and end with your summary.",
          is_error: true,
        }),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    // The chip is under the steps line.
    await user.click(await view.findByText("Ran a command"));
    expect(view.getByText("sent for approval")).toBeVisible();
    expect(view.queryByText("failed")).toBeNull();
  });

  it("names each run of a topic apart, even two queued in the same minute", async () => {
    const a = chat("a", "10:00");
    home([a, reply("a2", a, "10:00"), reply("a3", a, "10:01")]);
    renderApp(`/home/${dot.id}`);
    await screen.findByText("question a");
    const names = [...conversation().querySelectorAll("section[aria-label]")].map((s) =>
      s.getAttribute("aria-label"),
    );
    expect(names).toHaveLength(3);
    expect(new Set(names).size).toBe(3);
  });

  it("goes back to the sol's conversation when an older topic is gone", async () => {
    home([chat("current", "10:00")]);
    backend.on("run_thread", (args) => {
      if (args.runId === "gone") throw { kind: "not_found", message: "run gone" };
      return [chat("current", "10:00")];
    });
    const { router } = renderApp(`/home/${dot.id}/gone`);
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
    expect(await screen.findByText("question current")).toBeVisible();
    expect(screen.queryByText("run gone")).toBeNull();
  });

  it("shows Not delivered above the retry's bubble, and no Cancelled", async () => {
    const first = chat("a", "10:00", {
      status: "cancelled",
      delivery: "not_delivered",
      undelivered_at: "2026-10-09T10:30:00.000Z",
      undelivered_reason: "claude CLI not found",
      dismissed_at: "2026-10-09T10:31:00.000Z",
    });
    home([first, reply("b", first, "10:32")]);
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    const line = await view.findByText("Not delivered");
    const retry = await view.findByText("reply b");
    expect(line.compareDocumentPosition(retry) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(view.queryByText("Cancelled")).toBeNull();
    expect(view.queryByRole("button", { name: "Retry" })).toBeNull();
  });
});
