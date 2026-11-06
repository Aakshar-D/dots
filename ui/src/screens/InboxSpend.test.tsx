import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { HeldRun, Inbox } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const capped = sample.inbox.held.find((h) => h.reason.kind === "spend_cap")!;
const only = (held: HeldRun[]): Inbox => ({
  ...sample.inbox,
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held,
  instructions: [],
  app_reviews: [],
  team_scope: [],
  workflows: [],
  not_delivered: [],
});

describe("runs waiting for a spending cap in the Inbox", () => {
  it("name the cap and offer Raise cap and Run once anyway for a chat", async () => {
    backend.on("inbox", () => only([capped]));
    backend.on("run_release_cap", () => ({ ...sample.run, id: capped.run_id, cap_released: true }));
    const { user, router } = renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    const [item] = within(section).getAllByRole("listitem");
    expect(item).toHaveTextContent(
      `${capped.dot_name} · Chat · nightly-tests reached its $2.00 daily cap`,
    );
    await user.click(within(item!).getByRole("button", { name: "Run once anyway" }));
    await waitFor(() =>
      expect(backend.callsTo("run_release_cap")).toEqual([{ runId: capped.run_id }]),
    );
    await user.click(within(item!).getByRole("link", { name: "Raise cap" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/dots/${capped.dot_id}`));
  });

  it("offer a background run only Raise cap, and send the cap of all dots to Settings", async () => {
    const all: HeldRun = {
      ...capped,
      run_id: "01J0RUN9",
      trigger: "schedule",
      reason: { kind: "spend_cap", text: "All dots reached the $10.00 daily cap", scope: "all" },
    };
    backend.on("inbox", () => only([all]));
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    const [item] = within(section).getAllByRole("listitem");
    expect(within(item!).queryByRole("button", { name: "Run once anyway" })).toBeNull();
    expect(within(item!).getByRole("link", { name: "Raise cap" })).toHaveAttribute(
      "href",
      "/settings/runs",
    );
  });
});
