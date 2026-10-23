import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message, RunEventRecord } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, said } from "@/test/home";
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
  it("shows the person's message as a bubble, then the dot's words under a meta line", async () => {
    const a = chat("a", "10:00");
    home([a], [said("a", 1, "All tests pass.")]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const words = await view.findByText("All tests pass.");
    const bubble = view.getByText("question a");
    expect(bubble).toHaveAttribute("data-from", "person");
    const meta = view.getByText(`${dot.name} · ${localTime(a.queued_at)}`);
    expect(follows(bubble, meta)).toBe(true);
    expect(follows(meta, words)).toBe(true);
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
    const steps = await view.findByRole("group", { name: "Tool steps" });
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
