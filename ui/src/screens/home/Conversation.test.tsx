import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, Message, Run, RunEventRecord } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply, said, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const conversation = () => screen.getByRole("region", { name: "Conversation" });
/// The card of the scheduled run `run`, once it shows.
const card = async (run: Run) =>
  within(await screen.findByRole("region", { name: "Conversation" })).findByRole("article", {
    name: `Scheduled · ${localTime(run.queued_at)}`,
  });

describe("a dot's conversation", () => {
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

  it("shows a closed run card without loading its events", async () => {
    const run = scheduled("cron", "09:00", { summary: "Sent 3 invoices\nto the usual people" });
    home([run]);
    const { listening } = renderApp(`/home/${dot.id}`);
    const shown = await card(run);
    await listening();
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
    const asked = await within(await card(run)).findByRole("group", { name: "Question from the dot" });
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

  it("shows the last 20 run topics of a dot that never chatted", async () => {
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

  it("adds a new run of the dot at the end, and changes a status in place", async () => {
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

  it("invites the first message to a dot that never ran", async () => {
    home([]);
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText(`Ask ${dot.name} something.`, { exact: false })).toBeVisible();
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

  it("shows a failed load of the dot's runs", async () => {
    home([]);
    backend.on("list_runs", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByRole("alert")).toHaveTextContent("database is locked");
  });

  it("says what became of a call Dots performed", async () => {
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
    expect(within(calls).getByText("Performed by Dots")).toBeVisible();
  });

  it("names the dot in the rules a pending call in a chat topic offers", async () => {
    const waiting = chat("a", "10:00", { status: "awaiting_approval", ended_at: null });
    home([waiting], [], [{ ...sample.approval, run_id: "a" }]);
    backend.on("approval_rule_candidates", () => sample.rules);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("button", { name: "Approve and always allow" }));
    expect(screen.getByRole("group", { name: `Always allow for ${dot.name}` })).toBeVisible();
  });

  it("shows the dot's browser calls as lines", async () => {
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
          output: "Dots: Opened shop.com/cart\n### Page",
          is_error: false,
        },
      },
    ];
    home([chat("a", "10:00")], events);
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("Opened shop.com/cart")).toBeVisible();
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
    renderApp(`/home/${dot.id}`);
    const view = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await view.findByText("sent for approval")).toBeVisible();
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

  it("goes back to the dot's conversation when an older topic is gone", async () => {
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
});
