import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message, RunEventRecord } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply, said } from "@/test/home";
import { renderApp } from "@/test/render";

const conversation = async () =>
  within(await screen.findByRole("region", { name: "Conversation" }));

const event = (runId: string, seq: number, data: Record<string, unknown>): RunEventRecord => ({
  run_id: runId,
  seq,
  ts: sample.run.queued_at,
  kind: String(data.kind),
  data,
});

const follows = (a: Node, b: Node) =>
  (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

describe("the conversation as a document (deep-teal spec section 4)", () => {
  it("shows the person's message as a bubble, then the dot's words with no label", async () => {
    const a = chat("a", "10:00");
    home([a], [said("a", 1, "All tests pass.")]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const words = await view.findByText("All tests pass.");
    const bubble = view.getByText("question a");
    expect(bubble).toHaveAttribute("data-from", "person");
    expect(bubble).toHaveClass("max-w-[75%]", "break-words");
    expect(follows(bubble, words)).toBe(true);
    // A person's message has no header line, and the dot's words no name label; the run's
    // section keeps its name.
    expect(view.queryByText("Chat message")).toBeNull();
    expect(view.queryByText(`${dot.name} · ${localTime(a.queued_at)}`)).toBeNull();
    expect(view.getByRole("region", { name: /^Chat message, / })).toContainElement(bubble);
  });

  it("keeps the header line of a run another trigger started", async () => {
    const resumed = chat("b", "10:05", { trigger: "resume", root_run_id: "a", parent_run_id: "a" });
    home([chat("a", "10:00"), resumed], [said("b", 1, "Pushed.")]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    await view.findByText("Pushed.");
    expect(view.getByText("Continued after approval")).toBeVisible();
    expect(view.queryByText("Chat message")).toBeNull();
  });

  it("shows a group of tool steps as one line that opens to their chips", async () => {
    home(
      [chat("a", "10:00")],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        event("a", 2, { kind: "tool_result", id: "t1", output: "fn a", is_error: false }),
        event("a", 3, { kind: "tool_call", id: "t2", tool: "Bash", input: { command: "cargo test" } }),
        event("a", 4, { kind: "tool_result", id: "t2", output: "ok", is_error: false }),
        event("a", 5, { kind: "tool_call", id: "t3", tool: "Read", input: { file_path: "b.rs" } }),
        event("a", 6, { kind: "tool_result", id: "t3", output: "fn b", is_error: false }),
        event("a", 7, { kind: "assistant_text", text: "All tests pass." }),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Read 2 files · Ran a command");
    expect(line).toBeVisible();
    const steps = line.closest("details")!;
    expect(steps).toHaveAttribute("data-slot", "steps");
    expect(steps.querySelector("summary")).toHaveClass("text-xs", "text-muted-foreground");
    expect(within(steps).getByRole("group", { name: "Tool steps" })).not.toBeVisible();
    expect(within(steps.querySelector("summary")!).queryByText(/failed|waiting|running/)).toBeNull();

    await user.click(line);
    expect(steps).toHaveAttribute("open");
    const chips = within(steps).getByRole("group", { name: "Tool steps" });
    expect(chips).toBeVisible();
    expect(within(chips).getAllByText("Read")).toHaveLength(2);
    expect(within(chips).getByText("Bash")).toBeVisible();
  });

  it("says in the steps line that a step failed or waits for approval", async () => {
    home(
      [chat("a", "10:00", { status: "awaiting_approval", ended_at: null })],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Read", input: { file_path: "a.rs" } }),
        event("a", 2, { kind: "tool_result", id: "t1", output: "no such file", is_error: true }),
        event("a", 3, { kind: "tool_call", id: "t2", tool: "Bash", input: { command: "git push" } }),
        event("a", 4, {
          kind: "tool_result",
          id: "t2",
          output: "Queued for human approval #01APPROVAL. Do not retry this action.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Read a file · Ran a command");
    const summary = within(line.closest("summary")!);
    // The parked call waits: only the read counts as failed.
    expect(summary.getByText("· 1 failed")).toHaveClass("text-destructive");
    expect(summary.getByText("· waiting for approval")).toHaveClass("text-needs");
    expect(summary.queryByText(/running/)).toBeNull();
  });

  it("says in the steps line that a parked call was sent for approval once its run has ended", async () => {
    home(
      [chat("a", "10:00", { status: "succeeded" })],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Bash", input: { command: "git push" } }),
        event("a", 2, {
          kind: "tool_result",
          id: "t1",
          output: "Queued for human approval #01APPROVAL. Do not retry this action.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Ran a command");
    const summary = within(line.closest("summary")!);
    // Muted by the line it sits in, not the needs colour of a wait.
    expect(summary.getByText("· sent for approval")).not.toHaveClass("text-needs");
    expect(line.closest("summary")).toHaveClass("text-muted-foreground");
    expect(summary.queryByText(/waiting for approval/)).toBeNull();
  });

  it("marks a cancelled chat run in the conversation, and no other", async () => {
    const a = chat("a", "10:00", { status: "cancelled" });
    home(
      [a, reply("b", a, "10:05")],
      [said("a", 1, "Half an answer."), said("b", 1, "A whole answer.")],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    await view.findByText("A whole answer.");
    const cancelled = view.getByText("Cancelled");
    expect(cancelled).toHaveClass("text-xs", "text-muted-foreground");
    expect(view.getAllByText("Cancelled")).toHaveLength(1);
    expect(follows(view.getByText("Half an answer."), cancelled)).toBe(true);
  });

  it("says in the steps line that a step is running while its run goes on", async () => {
    home(
      [chat("a", "10:00", { status: "running", ended_at: null })],
      [event("a", 1, { kind: "tool_call", id: "t1", tool: "Bash", input: { command: "cargo test" } })],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const line = await view.findByText("Ran a command");
    expect(within(line.closest("summary")!).getByText("· running…")).toBeVisible();
  });

  it("shows a run's tool steps as one row of chips, each opening to its details", async () => {
    home(
      [chat("a", "10:00")],
      [
        event("a", 1, { kind: "tool_call", id: "t1", tool: "Bash", input: { command: "cargo test" } }),
        event("a", 2, { kind: "tool_result", id: "t1", output: "test result: ok", is_error: false }),
        event("a", 3, { kind: "tool_call", id: "t2", tool: "Read", input: { file_path: "a.rs" } }),
        event("a", 4, { kind: "tool_result", id: "t2", output: "no such file", is_error: true }),
      ],
    );
    const { user } = renderApp(`/home/${dot.id}`);
    const view = await conversation();
    // The chips are under the steps line.
    await user.click(await view.findByText("Ran a command · Read a file"));
    const steps = view.getByRole("group", { name: "Tool steps" });
    const [bash, read] = [...steps.querySelectorAll("details")];
    expect(steps.querySelectorAll("details")).toHaveLength(2);
    const bashChip = within(bash!.querySelector("summary")!);
    expect(bashChip.getByText("Bash")).toBeVisible();
    expect(bashChip.getByText("✓")).toHaveAttribute("aria-hidden", "true");
    expect(bashChip.getByText("done")).toBeInTheDocument();
    const readChip = within(read!.querySelector("summary")!);
    expect(readChip.getByText("✕")).toHaveAttribute("aria-hidden", "true");
    expect(readChip.getByText("failed")).toBeInTheDocument();
    expect(read).toHaveAttribute("data-tone", "failed");

    expect(within(read!).getByText("no such file")).not.toBeVisible();
    await user.click(readChip.getByText("Read"));
    expect(read).toHaveAttribute("open");
    expect(within(read!).getByText("no such file")).toBeVisible();
    expect(within(read!).getByText("a.rs")).toBeVisible();
  });

  it("shows a call sent for approval with its chip in the needs colour", async () => {
    home(
      [chat("a", "10:00", { status: "awaiting_approval", ended_at: null })],
      [
        event("a", 1, { kind: "tool_call", id: "t", tool: "Bash", input: { command: "git push" } }),
        event("a", 2, {
          kind: "tool_result",
          id: "t",
          output: "Queued for human approval #01APPROVAL. Do not retry this action.",
          is_error: true,
        }),
      ],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const steps = await view.findByRole("group", { name: "Tool steps" });
    const chip = steps.querySelector("details")!;
    expect(chip).toHaveAttribute("data-tone", "needs");
    expect(within(chip).getByText("sent for approval")).toBeInTheDocument();
  });

  it("says why a run failed once, as an alert with a rule, not a filled box", async () => {
    home(
      [chat("a", "10:00", { status: "failed", error: "timeout after 600 s" })],
      [event("a", 1, { kind: "failed", error: "timeout after 600 s" })],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const failure = await view.findByRole("alert");
    expect(failure).toHaveTextContent("timeout after 600 s");
    expect(failure).toHaveAttribute("data-slot", "failure");
    expect(view.getAllByRole("alert")).toHaveLength(1);
  });

  it("puts a question's state in its meta line, above its words", async () => {
    const asked: Message = { ...sample.messages[0], run_id: "a" };
    home([chat("a", "10:00")]);
    backend.on("messages_list", () => [asked]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const question = await view.findByRole("group", { name: "Question from the dot" });
    expect(question.firstElementChild).toHaveTextContent("Question · waiting for your answer");
    expect(question.lastElementChild).toHaveTextContent(asked.text);
  });
});
