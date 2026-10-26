import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { triggerLabel } from "@/lib/home-topics";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { dot, home, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const payload = {
  source: "folder",
  event: "created",
  path: "C:\\Users\\u\\Downloads\\Invoices\\invoice-12.pdf",
  relative: "invoice-12.pdf",
  size: 1200,
  watcher_id: "01J0WAT1",
  watcher_name: "Invoices folder",
};

describe("a watcher run", () => {
  it("is a card named by the watcher and the event in the conversation", async () => {
    home([scheduled("w1", "08:00", { trigger: "watcher", payload })]);
    renderApp(`/home/${dot.id}`);
    expect(
      await screen.findByRole("article", { name: "Invoices folder · New file: invoice-12.pdf" }),
    ).toBeVisible();
  });

  it("is a Watcher where a word is enough, and its page says what woke the dot", async () => {
    const run: Run = { ...sample.run, trigger: "watcher", payload, branch: null, base_commit: null };
    expect(triggerLabel(run)).toBe("Watcher");
    backend.on("run_thread", () => [run]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    renderApp(`/runs/${run.id}`);
    const section = await screen.findByRole("region", {
      name: `Watcher run, ${localDateTime(run.queued_at)}`,
    });
    expect(within(section).getByText("Invoices folder · New file: invoice-12.pdf")).toBeVisible();
  });
});
