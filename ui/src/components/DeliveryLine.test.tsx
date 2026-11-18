import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { DeliveryLine } from "@/components/DeliveryLine";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

const marked = (over: Partial<Run> = {}): Run => ({
  ...sample.run,
  trigger: "chat",
  delivery: "not_delivered",
  undelivered_at: "2026-11-02T11:00:00Z",
  undelivered_reason: "claude CLI not found",
  ...over,
});

function show(run: Run) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <DeliveryLine run={run} />
    </QueryClientProvider>,
  );
  return userEvent.setup();
}

describe("the delivery line", () => {
  it("shows a chat message that did not get there, with Retry and Discard", () => {
    show(marked());
    expect(screen.getByText("Not delivered: claude CLI not found")).toBeVisible();
    expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Discard" })).toBeVisible();
  });

  it("retries with the run's id", async () => {
    backend.on("retry_delivery", () => sample.run);
    const user = show(marked());
    await user.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() =>
      expect(backend.callsTo("retry_delivery")).toEqual([{ runId: sample.run.id }]),
    );
  });

  it("shows the reason a Discard was refused next to the buttons", async () => {
    backend.on("discard_delivery", () => {
      throw { kind: "conflict", message: "this message has just reached the sol" };
    });
    const user = show(marked());
    await user.click(screen.getByRole("button", { name: "Discard" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "this message has just reached the sol",
    );
  });

  it("gives a watcher run the line and no buttons", () => {
    show(marked({ trigger: "watcher" }));
    expect(screen.getByText("Not delivered: claude CLI not found")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows a dismissed run as plain Not delivered with no buttons", () => {
    show(marked({ dismissed_at: "2026-11-02T11:30:00Z" }));
    expect(screen.getByText("Not delivered")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
