import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { AppProposal } from "@/api/types";
import { AppReviewCard } from "@/components/AppReviewCard";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { chat, dot, home, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const proposals = T.AppProposal.array().parse(contract.AppProposal);
const fresh = proposals.find((p) => p.id === "01J0PROP")!;
const onKept = proposals.find((p) => p.id === "01J0PROP5")!;
const change: AppProposal = {
  ...fresh,
  id: "01J0PROP6",
  app_id: "01J0APP2",
  base_version: 2,
  summary: { ...fresh.summary, update_from: 2, migrations: ["0002_category.sql"] },
};

function show(proposal: AppProposal) {
  const client = createQueryClient();
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <AppReviewCard proposal={proposal} dotName="builder" />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { ...view, user: userEvent.setup() };
}

function previewing() {
  backend.on("app_preview_open", () => "01J0PREV");
  backend.on("app_preview_close", () => null);
}

describe("an app sent for review", () => {
  it("runs the preview on its own database and closes it with the card", async () => {
    previewing();
    backend.on("app_preview_call", () => [{ item: "Coffee" }]);
    const { unmount } = show(fresh);
    expect(screen.getByRole("article", { name: "Review of Expenses" })).toHaveTextContent(
      "New app: Expenses · from ● builder",
    );
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses preview");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/preview/01J0PREV/index.html");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(backend.callsTo("app_preview_open")).toEqual([{ id: fresh.id }]);
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    port.postMessage({ type: "call", id: 1, tool: "add_expense", input: { item: "Tea" } });
    await waitFor(() => expect(got).toContainEqual(expect.objectContaining({ id: 1, ok: true })));
    expect(backend.callsTo("app_preview_call")).toEqual([
      { preview: "01J0PREV", tool: "add_expense", input: { item: "Tea" } },
    ]);
    unmount();
    await waitFor(() =>
      expect(backend.callsTo("app_preview_close")).toEqual([{ preview: "01J0PREV" }]),
    );
  });

  it("sums up what the app is, what it reaches and its tests", async () => {
    previewing();
    const { user } = show(fresh);
    await user.click(screen.getByRole("tab", { name: "Summary" }));
    const panel = screen.getByRole("tabpanel", { name: "Summary" });
    expect(panel).toHaveTextContent("Logs expenses and totals them by month");
    expect(panel).toHaveTextContent("add_expense (writes)");
    expect(panel).toHaveTextContent("Can start dots");
    expect(panel).toHaveTextContent("1 of 1 test passed");
    expect(panel).toHaveTextContent("7 files · 2.0 KB");
    expect(panel).toHaveTextContent("0001_expenses.sql");
    expect(screen.getByText(/It is added with "Can start dots" off/)).toBeVisible();
  });

  it("shows every file of a new app, and the diff of a change first", async () => {
    previewing();
    backend.on("app_proposal_files", () => [
      { path: "logic.js", text: "export function list() {}", base64: null },
      { path: "ui/logo.png", text: null, base64: "iVBORw0KGgo=" },
    ]);
    backend.on("app_proposal_diff", () => ({
      files: [{ path: "logic.js", status: "modified", patch: "@@ -1 +1 @@\n-a\n+b\n", binary: false }],
      truncated: false,
      exists: true,
    }));
    const { user, unmount } = show(fresh);
    await user.click(screen.getByRole("tab", { name: "Code" }));
    const files = await screen.findByRole("region", { name: "Every file" });
    expect(await within(files).findByText("logic.js")).toBeVisible();
    expect(within(files).getByText("Binary file.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "What it changes" })).toBeNull();
    expect(backend.callsTo("app_proposal_diff")).toEqual([]);
    unmount();

    const again = show(change);
    expect(screen.getByRole("article")).toHaveTextContent("Change to Expenses: version 2 and on");
    await again.user.click(screen.getByRole("tab", { name: "Code" }));
    const changes = await screen.findByRole("region", { name: "What it changes" });
    expect(await within(changes).findByText("+b")).toBeVisible();
    expect(screen.getByRole("button", { name: "Update" })).toBeVisible();
  });

  it("adds the app, on the kept data unless the person unticks it", async () => {
    previewing();
    backend.on("app_proposal_decide", () => onKept);
    const { user } = show(onKept);
    const box = screen.getByRole("checkbox", { name: /Start on the data kept from the removed Expenses/ });
    expect(box).toBeChecked();
    expect(box.parentElement).toHaveTextContent("it then runs 0002_category.sql on it");
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    await waitFor(() =>
      expect(backend.callsTo("app_proposal_decide")).toEqual([
        { id: onKept.id, decision: { kind: "add_with_kept_data" } },
      ]),
    );
    await user.click(box);
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    await waitFor(() => expect(backend.callsTo("app_proposal_decide")).toHaveLength(2));
    expect(backend.callsTo("app_proposal_decide")[1]).toEqual({
      id: onKept.id,
      decision: { kind: "add" },
    });
  });

  it("asks for changes in the person's words, and discards after asking", async () => {
    previewing();
    backend.on("app_proposal_decide", () => fresh);
    const { user } = show(fresh);
    await user.click(screen.getByRole("button", { name: "Ask for changes" }));
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "What to change" }), "Add a category.");
    await user.click(send);
    await waitFor(() =>
      expect(backend.callsTo("app_proposal_decide")).toEqual([
        { id: fresh.id, decision: { kind: "ask_for_changes", text: "Add a category." } },
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Discard" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Discard" }),
    );
    await waitFor(() => expect(backend.callsTo("app_proposal_decide")).toHaveLength(2));
    expect(backend.callsTo("app_proposal_decide")[1]).toEqual({
      id: fresh.id,
      decision: { kind: "discard" },
    });
  });

  it("keeps waiting with the refusal shown when the decision is refused", async () => {
    previewing();
    backend.on("app_proposal_decide", () => {
      throw {
        kind: "conflict",
        message: "the dot is still working on this chat; wait until it has finished",
      };
    });
    const { user } = show(fresh);
    await user.click(screen.getByRole("button", { name: "Add to library" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("the dot is still working on this chat");
    expect(screen.getByRole("button", { name: "Add to library" })).toBeEnabled();
  });
});

describe("a review card in the conversation", () => {
  it("stands under the run that sent the app, while it waits", async () => {
    previewing();
    const run = chat("c1", "10:00");
    home([run]);
    const waiting = { ...fresh, dot_id: dot.id, run_id: run.id, root_run_id: run.id };
    const decided = { ...waiting, id: "01J0PROP7", status: "added" as const };
    backend.on("app_proposals_list", () => [waiting, decided]);
    renderApp(`/home/${dot.id}`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    const cards = await within(conversation).findAllByRole("article", { name: "Review of Expenses" });
    expect(cards).toHaveLength(1);
    expect(backend.callsTo("app_proposals_list")).toContainEqual({ dotId: dot.id });
  });

  it("stands under the card of a scheduled or manual run that sent the app", async () => {
    previewing();
    const run = scheduled("s1", "09:00");
    home([run]);
    const waiting = { ...fresh, dot_id: dot.id, run_id: run.id, root_run_id: run.id };
    backend.on("app_proposals_list", () => [waiting]);
    renderApp(`/home/${dot.id}`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    expect(
      await within(conversation).findByRole("article", { name: "Review of Expenses" }),
    ).toBeVisible();
  });
});
