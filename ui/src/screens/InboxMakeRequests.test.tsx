import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { Inbox, MakeRequestCard, PendingMakeRequest } from "@/api/types";
import { inboxWaiting, makeRequestKey } from "@/lib/inbox";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";

const cards = T.MakeRequestCard.array().parse(contract.MakeRequestCard);
const make = cards.find((c) => c.request.id === "01J0MRQ")!;
const job = cards.find((c) => c.request.id === "01J0MRQ3")!;

/// The Inbox's slim item of a card, as the core builds it (`PendingMakeRequest::of`).
const item = (card: MakeRequestCard): PendingMakeRequest => ({
  id: card.request.id,
  dot_id: card.request.maker_dot_id,
  dot_name: card.maker_name,
  root_run_id: card.request.root_run_id,
  kind: card.request.kind,
  name: card.name,
  purpose: card.request.purpose,
  title: card.title,
  lines: card.lines,
  shown: card.shown,
  status: card.request.status,
  created_at: card.request.created_at,
});

const only = (make_requests: PendingMakeRequest[], over: Partial<Inbox> = {}): Inbox => ({
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  instructions: [],
  app_reviews: [],
  team_scope: [],
  workflows: [],
  make_requests,
  not_delivered: [], back: [], snoozed: [], done: [], hidden_messages: [],
  ...over,
});

const section = () => screen.findByRole("region", { name: "Sols asking to make sols" });

describe("makers' requests in the Inbox", () => {
  it("shows the full card of each waiting request, read from the core, and answers from it", async () => {
    let waiting = [make, job];
    backend.on("inbox", () => only(waiting.map(item)));
    backend.on("make_requests_waiting", () => waiting);
    backend.on("make_request_decide", ({ id }) => {
      const answered = waiting.find((card) => card.request.id === id)!;
      waiting = waiting.filter((card) => card !== answered);
      return { ...answered.request, status: "allowed", made_dot_id: "01J0MADE" };
    });
    const { user } = renderApp("/inbox");
    const rows = within(await section()).getAllByRole("listitem", { name: /request about/ });
    expect(rows).toHaveLength(2);
    const row = rows[0]!;
    expect(row).toHaveAccessibleName("Researcher's request about Blog Watcher");
    expect(within(row).getByRole("link", { name: "Open conversation" })).toHaveAttribute(
      "href",
      `/home/${make.request.maker_dot_id}/${make.request.root_run_id}`,
    );
    const card = await within(row).findByRole("article", { name: make.title });
    expect(card).toHaveTextContent("4th sol in this run; the limit is 3");
    // The rail counts both.
    expect(screen.getByRole("link", { name: "Inbox, 2 waiting" })).toBeVisible();

    await user.click(within(card).getByRole("button", { name: "Allow for this sol" }));
    await waitFor(() =>
      expect(backend.callsTo("make_request_decide")).toEqual([
        { id: make.request.id, decision: "allow", limit: make.limit, shown: make.shown },
      ]),
    );
    // The answered request leaves the section once the Inbox is read again.
    await waitFor(() =>
      expect(screen.getAllByRole("listitem", { name: /request about/ })).toHaveLength(1),
    );
  });

  it("offers no answer from the slim item while its card is not read", async () => {
    backend.on("inbox", () => only([item(make)]));
    backend.on("make_requests_waiting", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp("/inbox");
    const row = await within(await section()).findByRole("listitem", { name: /request about/ });
    expect(row).toHaveTextContent(make.title);
    expect(row).toHaveTextContent("The browser");
    expect(await within(await section()).findByText("database is locked")).toBeVisible();
    expect(within(row).queryByRole("button", { name: /Allow|without|Don't/ })).toBeNull();
    expect(within(row).getByRole("link", { name: "Open conversation" })).toBeVisible();
  });

  it("snoozes and marks done by the request's own key", async () => {
    backend.on("inbox", () => only([item(make)]));
    backend.on("make_requests_waiting", () => [make]);
    backend.on("inbox_done", () => null);
    const { user } = renderApp("/inbox");
    const row = await within(await section()).findByRole("listitem", { name: /request about/ });
    await user.click(within(row).getByRole("button", { name: /^Done/ }));
    await waitFor(() => expect(backend.callsTo("inbox_done")).toHaveLength(1));
    expect(backend.callsTo("inbox_done")[0]).toMatchObject({
      item: { kind: "make_request", id: make.request.id },
    });
    expect(makeRequestKey(item(make))).toEqual({ kind: "make_request", id: make.request.id });
  });

  it("shows one that is back from snooze first, with its card", async () => {
    const mark = {
      kind: "make_request" as const,
      id: make.request.id,
      title: make.title,
      dot_id: make.request.maker_dot_id,
      snoozed_until: null,
      done_at: null,
      back_at: "2026-10-28T10:00:00Z",
    };
    backend.on("inbox", () => only([item(make)], { back: [mark] }));
    backend.on("make_requests_waiting", () => [make]);
    renderApp("/inbox");
    const back = await screen.findByRole("region", { name: "Back from snooze" });
    expect(await within(back).findByRole("article", { name: make.title })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Sols asking to make sols" })).toBeNull();
  });

  it("counts each waiting request once", () => {
    expect(inboxWaiting(only([item(make), item(job)]), [])).toBe(2);
    expect(inboxWaiting(only([]), [])).toBe(0);
  });
});
