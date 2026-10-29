import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, PendingApproval } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const card = {
  action: "click",
  site: "shop.com",
  page: "shop.com/cart",
  labels: ["Pay now"],
  risky: true,
  dangerous: false,
};
const pay: PendingApproval = {
  ...sample.inbox.approvals[0]!,
  tool: "mcp__browser__browser_click",
  input: { target: "e16", dots_browser: card },
};

function inbox(approvals: PendingApproval[]) {
  backend.on("inbox", () => ({
    approvals,
    review_runs: [],
    attention: [],
    tools_changed: [],
    watchers: [], held: [], instructions: [], app_reviews: [],
  }));
  backend.on("decide_approval", (args): Approval => ({
    ...sample.approval,
    id: String(args.id),
    status: args.approved ? "approved" : "denied",
  }));
}

describe("a browser approval", () => {
  it("says what and where, and offers its site's rules", async () => {
    inbox([pay]);
    backend.on("approval_rule_candidates", () => [
      { pattern: "Browser(shop.com)", label: "always allow on shop.com" },
      { pattern: "BrowserRisky(shop.com)", label: "always allow risky actions on shop.com" },
    ]);
    const { user } = renderApp("/inbox");
    const article = await screen.findByRole("article", {
      name: `Browser approval for ${pay.dot_name}`,
    });
    expect(article).toHaveTextContent(`${pay.dot_name} wants to click "Pay now" on shop.com`);
    expect(within(article).getByText('"Pay now"').tagName).toBe("STRONG");
    expect(within(article).getByText("shop.com").tagName).toBe("STRONG");
    expect(within(article).getByRole("button", { name: "Allow once" })).toBeVisible();
    expect(
      await within(article).findByRole("button", { name: "Always allow on shop.com" }),
    ).toBeVisible();
    await user.click(
      within(article).getByRole("button", { name: "Always allow risky actions on shop.com" }),
    );
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: pay.id, approved: true, note: null, allowRule: "BrowserRisky(shop.com)" },
      ]),
    );
  });

  it("asks to open a site, says a parked approval is not performed, and denies", async () => {
    const open: PendingApproval = {
      ...pay,
      id: "01J0APR2",
      tool: "mcp__browser__browser_navigate",
      parked: true,
      input: {
        url: "https://shop.com/cart",
        dots_browser: { ...card, action: "open", labels: [], risky: false },
      },
    };
    inbox([open]);
    backend.on("approval_rule_candidates", () => [
      { pattern: "Browser(shop.com)", label: "always allow on shop.com" },
    ]);
    const { user } = renderApp("/inbox");
    const article = await screen.findByRole("article", {
      name: `Browser approval for ${pay.dot_name}`,
    });
    expect(article).toHaveTextContent(`${pay.dot_name} wants to open shop.com/cart`);
    expect(article).toHaveTextContent(
      "The run stopped waiting. Approving now does not perform this action: the dot is told to take a snapshot and do it again if it is still right.",
    );
    expect(
      await within(article).findByRole("button", { name: "Always allow on shop.com" }),
    ).toBeVisible();
    expect(within(article).queryByRole("button", { name: /risky actions/ })).toBeNull();
    await user.click(within(article).getByRole("button", { name: "Deny" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: "01J0APR2", approved: false, note: null, allowRule: null },
      ]),
    );
  });

  it("keeps the tool card for every other call", async () => {
    inbox([sample.inbox.approvals[0]!]);
    renderApp("/inbox");
    expect(
      await screen.findByRole("article", { name: `Approval for ${sample.approval.tool}` }),
    ).toBeVisible();
    expect(screen.queryByRole("article", { name: /^Browser approval/ })).toBeNull();
  });

  // Review fix: the card lists what the call sends.
  it("lists what an upload would send", async () => {
    const upload: PendingApproval = {
      ...pay,
      id: "01J0APR9",
      tool: "mcp__browser__browser_file_upload",
      input: {
        paths: ["C:\Users\me\.ssh\id_ed25519"],
        dots_browser: { ...card, action: "upload", labels: [], dangerous: true },
      },
    };
    inbox([upload]);
    backend.on("approval_rule_candidates", () => []);
    renderApp("/inbox");
    const article = await screen.findByRole("article", {
      name: `Browser approval for ${pay.dot_name}`,
    });
    const sent = within(article).getByRole("list", { name: "What it sends" });
    expect(sent).toHaveTextContent("File: C:\Users\me\.ssh\id_ed25519");
  });
});
