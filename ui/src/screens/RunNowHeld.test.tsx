import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, noCeiling, sample } from "@/test/fixtures";
import { dot, home, overview } from "@/test/home";
import { renderApp } from "@/test/render";

const heldText =
  "Acme Engineering: this sol's tool policy is past what a Member may use in the Platform workspace (it is Trusted). Change the policy, or ask an Admin.";

/// The core holds the dot's runs: its policy is past the workspace's ceiling.
function heldByCeiling() {
  backend.on("dot_ceiling", () => ({ ...noCeiling, applies_as: "member", held: heldText }));
}

describe("Run now on a sol whose runs are held", () => {
  it("is off in the sol's editor, with the hold text", async () => {
    heldByCeiling();
    backend.on("get_dot", () => sample.dot);
    renderApp(`/dots/${sample.dot.id}`);
    const run = await screen.findByRole("button", { name: "Run now" });
    await waitFor(() => expect(run).toBeDisabled());
    expect(run).toHaveAttribute("title", heldText);
    expect(backend.callsTo("dot_ceiling")[0]).toEqual({ dotId: sample.dot.id });
  });

  it("is off in the editor with the role's refusal when the core refuses running it", async () => {
    const refusal = "Acme Engineering: a Viewer can't run sols. Ask an Admin or Owner.";
    backend.on("get_dot", () => sample.dot);
    backend.on("dot_access", () => ({ ...allowed, run: refusal }));
    renderApp(`/dots/${sample.dot.id}`);
    const run = await screen.findByRole("button", { name: "Run now" });
    await waitFor(() => expect(run).toHaveAttribute("title", refusal));
    expect(run).toBeDisabled();
  });

  it("is off in the sol's menu, with the hold text", async () => {
    heldByCeiling();
    home([]);
    backend.on("dot_overview", () => [overview]);
    backend.on("get_dot", () => dot);
    const { user } = renderApp(`/home/${dot.id}`);
    await waitFor(() => expect(backend.callsTo("dot_ceiling").length).toBeGreaterThan(0));
    await user.click(await screen.findByRole("button", { name: dot.name }));
    const menu = await screen.findByRole("menu");
    const run = within(menu).getByRole("menuitem", { name: "Run now" });
    await waitFor(() => expect(run).toHaveAttribute("data-disabled"));
    expect(run).toHaveAttribute("title", heldText);
  });

  it("is off in the Inbox's needs-attention row, with the hold text", async () => {
    heldByCeiling();
    const [flagged] = sample.inbox.attention;
    const inbox: Inbox = {
      approvals: [],
      review_runs: [],
      attention: [flagged!],
      tools_changed: [],
      watchers: [],
      held: [],
      instructions: [],
      app_reviews: [],
      team_scope: [],
      workflows: [],
      not_delivered: [],
      back: [],
      snoozed: [],
      done: [],
      hidden_messages: [],
    };
    backend.on("inbox", () => inbox);
    renderApp("/inbox");
    const row = await screen.findByRole("listitem", {
      name: `${flagged!.dot_name} needs attention`,
    });
    const run = within(row).getByRole("button", { name: "Run now" });
    await waitFor(() => expect(run).toBeDisabled());
    expect(run).toHaveAttribute("title", heldText);
  });

  it("stays on where nothing holds the sol", async () => {
    backend.on("get_dot", () => sample.dot);
    renderApp(`/dots/${sample.dot.id}`);
    const run = await screen.findByRole("button", { name: "Run now" });
    await waitFor(() => expect(backend.callsTo("dot_ceiling").length).toBeGreaterThan(0));
    expect(run).toBeEnabled();
  });
});
