import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { InstructionProposal } from "@/api/types";
import { backend } from "@/test/backend";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const a = chat("a", "10:00", {
  payload: { message: "buy the closest brand from now on", typed: true },
});

const proposal = (over: Partial<InstructionProposal> = {}): InstructionProposal => ({
  id: "p1",
  dot_id: dot.id,
  run_id: "a",
  root_run_id: "a",
  base: "Order the usual groceries.\nPay with the saved card.",
  instructions:
    "Order the usual groceries.\nIf an item is out of stock, buy the closest brand.\n" +
    "Pay with the saved card.",
  reason: "Buy the closest brand when an item is out of stock.",
  status: "pending",
  answer_run_id: null,
  created_at: "2026-10-09T10:01:00.000Z",
  decided_at: null,
  ...over,
});

const named = `New instructions for ${dot.name}`;
const card = () => screen.findByRole("article", { name: named });

/// The dot's proposals are `list` until the person decides one; then there are none.
function deciding(list: InstructionProposal[]) {
  let now = list;
  backend.on("instruction_proposals_list", () => now);
  backend.on("instruction_proposal_decide", (args) => {
    now = [];
    return proposal({ status: args.approve ? "approved" : "rejected" });
  });
}

describe("instructions a dot proposed", () => {
  it("shows the waiting one in the conversation with its reason and changes", async () => {
    home([a]);
    backend.on("instruction_proposals_list", () => [
      proposal(),
      proposal({ id: "p0", status: "replaced", reason: "An older idea." }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = await screen.findByRole("region", { name: "Conversation" });
    const shown = await within(view).findByRole("article", { name: named });
    expect(shown).toHaveTextContent(`${dot.name} wants to change its instructions`);
    expect(shown).toHaveTextContent("Buy the closest brand when an item is out of stock.");
    const changes = within(shown).getByLabelText("Changes to the instructions");
    const added = [...changes.querySelectorAll('[data-kind="added"]')];
    expect(added.map((l) => l.textContent)).toEqual([
      "+ If an item is out of stock, buy the closest brand.",
    ]);
    expect(changes.querySelectorAll('[data-kind="removed"]')).toHaveLength(0);
    expect(within(view).getAllByRole("article", { name: /^New instructions/ })).toHaveLength(1);
    expect(screen.queryByText("An older idea.")).toBeNull();
    expect(backend.callsTo("instruction_proposals_list")).toContainEqual({ dotId: dot.id });
  });

  it("approves it, and reads the dot and the list again", async () => {
    home([a]);
    deciding([proposal()]);
    const { user } = renderApp(`/home/${dot.id}`);
    const shown = await card();
    const dotsAsked = backend.callsTo("dot_overview").length;
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(backend.callsTo("instruction_proposal_decide")).toEqual([
        { id: "p1", approve: true },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("article", { name: named })).toBeNull());
    // Approving changed the dot.
    await waitFor(() =>
      expect(backend.callsTo("dot_overview").length).toBeGreaterThan(dotsAsked),
    );
  });

  it("rejects it", async () => {
    home([a]);
    deciding([proposal()]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(within(await card()).getByRole("button", { name: "Reject" }));
    await waitFor(() =>
      expect(backend.callsTo("instruction_proposal_decide")).toEqual([
        { id: "p1", approve: false },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("article", { name: named })).toBeNull());
  });

  it("shows a refused decision as the core words it and keeps the card", async () => {
    const refusal =
      "the dot's instructions changed after it proposed these; ask it to propose them again";
    home([a]);
    backend.on("instruction_proposals_list", () => [proposal()]);
    backend.on("instruction_proposal_decide", () => {
      throw { kind: "conflict", message: refusal };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const shown = await card();
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    expect(await screen.findByText(refusal)).toBeVisible();
    expect(within(shown).getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("lists the waiting ones in the Inbox with their dot and conversation", async () => {
    backend.on("inbox", () => ({
      approvals: [],
      review_runs: [],
      attention: [],
      tools_changed: [],
      watchers: [],
      held: [],
      instructions: [{ proposal: proposal(), dot_name: dot.name }],
      app_reviews: [],
    }));
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "New instructions" });
    expect(within(group).getByRole("article", { name: named })).toBeVisible();
    expect(within(group).getByRole("link", { name: "Open conversation" })).toHaveAttribute(
      "href",
      `/chat/${dot.id}/a`,
    );
    expect(within(group).getByRole("link", { name: dot.name })).toHaveAttribute(
      "href",
      `/dots/${dot.id}`,
    );
  });
});
