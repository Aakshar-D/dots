import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { ResultCheck, Run } from "@/api/types";
import { checkLabel, ResultCheckBadge } from "@/components/ResultCheckBadge";
import { transcriptItems } from "@/lib/transcript";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

const done = (
  verdict: "looks_good" | "has_concerns" | "failed_task",
  concerns: string[] = [],
): ResultCheck => ({ state: "done", model: "haiku", verdict, concerns, confidence: 0.8 });

const withCheck = (result_check: ResultCheck | null, status: Run["status"] = "succeeded"): Run => ({
  ...sample.run,
  status,
  result_check,
});

function show(run: Run, canFix = true) {
  const user = userEvent.setup();
  const view = render(
    <QueryClientProvider client={createQueryClient()}>
      <ResultCheckBadge run={run} canFix={canFix} />
    </QueryClientProvider>,
  );
  return { user, ...view };
}

describe("the result check badge", () => {
  it("names every state", () => {
    expect(checkLabel({ state: "checking", model: "haiku" })).toBe("Checking…");
    expect(checkLabel({ state: "skipped", reason: "x" })).toBe("Check skipped");
    expect(checkLabel(done("looks_good"))).toBe("✓ Checked");
    expect(checkLabel(done("has_concerns", ["a"]))).toBe("⚠ 1 concern");
    expect(checkLabel(done("has_concerns", ["a", "b"]))).toBe("⚠ 2 concerns");
    expect(checkLabel(done("failed_task", ["a"]))).toBe("⚠ May have missed the task");
  });

  it("shows Checking… while the check runs, and why a skipped check has no verdict", () => {
    const { rerender } = show(withCheck({ state: "checking", model: "haiku" }));
    expect(screen.getByRole("status")).toHaveTextContent("Checking…");
    rerender(
      <QueryClientProvider client={createQueryClient()}>
        <ResultCheckBadge run={withCheck({ state: "skipped", reason: "The check took too long." })} />
      </QueryClientProvider>,
    );
    expect(screen.getByText("Check skipped")).toHaveAttribute("title", "The check took too long.");
  });

  it("shows ✓ Checked with nothing to open for a good answer", () => {
    show(withCheck(done("looks_good")));
    expect(screen.getByText("✓ Checked")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("is not there without a check, or for a run that did not succeed", () => {
    const { container } = show(withCheck(null));
    expect(container).toBeEmptyDOMElement();
    const failed = show(withCheck(done("has_concerns", ["a"]), "failed"));
    expect(failed.container).toBeEmptyDOMElement();
  });

  it("opens the concerns and asks the sol to fix them in one click", async () => {
    const run = withCheck(done("has_concerns", ["Two tests were skipped.", "No numbers."]));
    backend.on("result_check_fix", () => ({ ...sample.run, id: "fix-run" }));
    const { user } = show(run);
    const badge = screen.getByRole("button", { name: "⚠ 2 concerns" });
    expect(badge).toHaveAttribute("aria-expanded", "false");
    await user.click(badge);
    const list = screen.getByRole("region", { name: "Result check concerns" });
    expect(list).toHaveTextContent("Two tests were skipped.");
    expect(list).toHaveTextContent("No numbers.");
    await user.click(screen.getByRole("button", { name: "Ask the sol to fix this" }));
    await waitFor(() => expect(backend.callsTo("result_check_fix")).toEqual([{ runId: run.id }]));
  });

  it("waits while the sol is still busy in the conversation, and shows a refusal", async () => {
    const run = withCheck(done("failed_task", ["Nothing was written."]));
    const busy = show(run, false);
    await busy.user.click(screen.getByRole("button", { name: "⚠ May have missed the task" }));
    expect(screen.getByRole("button", { name: "Ask the sol to fix this" })).toBeDisabled();
    expect(screen.getByText("Wait until the sol has finished in this conversation.")).toBeVisible();
    busy.unmount();

    backend.on("result_check_fix", () => {
      throw { kind: "conflict", message: "the sol is still working on this chat" };
    });
    const { user } = show(run);
    await user.click(screen.getByRole("button", { name: "⚠ May have missed the task" }));
    await user.click(screen.getByRole("button", { name: "Ask the sol to fix this" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("still working");
  });

  it("is never a transcript line", () => {
    const items = transcriptItems([
      {
        run_id: "r",
        seq: 1,
        ts: "2026-11-06T00:00:00Z",
        kind: "result_check",
        data: {
          kind: "result_check",
          model: "haiku",
          verdict: "looks_good",
          skipped: null,
          tokens_in: 1,
          tokens_out: 1,
          cache_read: 0,
          cost_usd: null,
        },
      },
    ]);
    expect(items).toEqual([]);
  });
});
