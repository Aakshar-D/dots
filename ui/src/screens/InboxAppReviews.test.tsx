import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox, PendingAppReview } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [review] = sample.inbox.app_reviews as [PendingAppReview];
const only = (app_reviews: PendingAppReview[]): Inbox => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  instructions: [],
  app_reviews,
});

describe("apps sent for review in the Inbox", () => {
  it("lists each with its dot and opens the card in the dot's conversation", async () => {
    backend.on("inbox", () => only([review]));
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Apps to review" });
    const item = within(section).getByRole("listitem", { name: "Expenses to review" });
    expect(item).toHaveTextContent("builder sent the app Expenses for review");
    expect(item).toHaveTextContent(review.proposal.note);
    expect(within(item).getByRole("link", { name: "Open" })).toHaveAttribute(
      "href",
      `/home/${review.dot_id}/${review.proposal.root_run_id}`,
    );
  });

  it("names a change by the app it changes, and counts it in the needs colour", async () => {
    const change: PendingAppReview = {
      ...review,
      proposal: { ...review.proposal, summary: { ...review.proposal.summary, update_from: 2 } },
    };
    backend.on("inbox", () => only([change]));
    renderApp("/inbox");
    const item = await screen.findByRole("listitem", { name: "Expenses to review" });
    expect(item).toHaveTextContent("builder sent a change to Expenses for review");
    const link = screen.getByRole("link", { name: "Inbox, 1 waiting" });
    expect(within(link).getByText("1")).toHaveAttribute("data-tone", "needs");
  });
});
