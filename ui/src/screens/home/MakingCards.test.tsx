import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { MakingCard, Run } from "@/api/types";
import { chatMessage, runTrigger } from "@/components/Transcript";
import { triggerLabel } from "@/lib/home-topics";
import { backend } from "@/test/backend";
import { chat, dot, home, reply, said, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const samples = T.MakingCard.array().parse(contract.MakingCard);
/// The contract's card of `kind` (the first of them), as done by the run `runId` of its topic.
function card<K extends MakingCard["kind"]>(kind: K, run: Run, over: object = {}) {
  const found = samples.find((c) => c.kind === kind)!;
  return {
    ...found,
    id: `${kind}-${run.id}`,
    run_id: run.id,
    root_run_id: run.root_run_id,
    ...over,
  } as Extract<MakingCard, { kind: K }>;
}

const conversation = async () =>
  within(await screen.findByRole("region", { name: "Conversation" }));
/// Whether `a` stands before `b` on the page.
const before = (a: Element, b: Element) =>
  (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

describe("a maker's chat topic", () => {
  it("draws what each run did after that run, in the order it happened", async () => {
    const first = chat("a", "10:00");
    const second = reply("b", first, "10:05");
    home([first, second], [said("a", 1, "Making it now."), said("b", 1, "All done.")]);
    const cards = [
      card("made", first),
      card("job_started", first),
      card("job_ended", second),
      card("changed", second),
      card("deleted", second),
    ];
    backend.on("making_cards", ({ rootRunId }) => (rootRunId === "a" ? cards : []));
    renderApp(`/home/${dot.id}`);
    const view = await conversation();

    const made = await view.findByRole("article", { name: "Made sol: Blog Watcher" });
    expect(made).toHaveTextContent("made by Researcher");
    expect(made).toHaveTextContent("Reads the feeds");
    expect(made).toHaveTextContent("Schedule: daily at 09:00");
    expect(made).toHaveTextContent("Uses: the browser, folder research/blogs. No secrets.");
    expect(within(made).getByRole("link", { name: "Open sol" })).toHaveAttribute(
      "href",
      "/dots/01J0MADE",
    );

    const working = view.getByRole("article", { name: "Blog Watcher is working" });
    const text = within(working).getByRole("figure", { name: "Job from Researcher" });
    expect(text).toHaveTextContent("Read today's feeds.");
    expect(within(working).getByRole("link", { name: "Open run" })).toHaveAttribute(
      "href",
      "/runs/01J0RUN2",
    );

    const finished = view.getByRole("article", { name: "Blog Watcher finished" });
    expect(finished).toHaveTextContent("Sage tells Researcher how it ended.");
    const changed = view.getByRole("article", { name: "Researcher changed Blog Watcher" });
    expect(within(changed).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "New instructions",
      "Schedule: daily at 09:00",
    ]);
    const deleted = view.getByRole("article", { name: "Researcher deleted Blog Watcher" });

    // After the run each names: the first run's cards, then the second run's words and cards.
    const reply2 = view.getByText("reply b");
    expect(before(view.getByText("Making it now."), made)).toBe(true);
    expect(before(made, working)).toBe(true);
    expect(before(working, reply2)).toBe(true);
    expect(before(reply2, finished)).toBe(true);
    expect(before(finished, changed)).toBe(true);
    expect(before(changed, deleted)).toBe(true);
    expect(backend.callsTo("making_cards")).toContainEqual({ rootRunId: "a" });
  });

  it("says how a job stands and ended, and what is gone", async () => {
    const first = chat("a", "10:00");
    home([first]);
    backend.on("making_cards", () => [
      // The made sol was deleted since: no link to it.
      card("made", first, { id: "m", dot_id: null, schedule: null, uses: null }),
      // A job that moved on no longer says its sol is working.
      card("job_started", first, { id: "s", status: "ended" }),
      card("job_waiting", first, { id: "w" }),
      card("job_ended", first, { id: "f", how: "failed" }),
      card("job_ended", first, { id: "c", how: "cancelled" }),
      card("job_ended", first, {
        id: "d",
        how: "deleted",
        status: null,
        run_exists: false,
      }),
      // The request went with the sol it was for.
      card("asked", first, { id: "g", card: null }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const made = await view.findByRole("article", { name: "Made sol: Blog Watcher" });
    expect(made).toHaveTextContent("This sol has been deleted since.");
    expect(within(made).queryByRole("link")).toBeNull();
    expect(view.getByRole("article", { name: "Blog Watcher was given a job" })).toBeVisible();
    const waiting = view.getByRole("article", { name: "Blog Watcher is waiting for your answer" });
    expect(within(waiting).getByRole("link", { name: "Open run" })).toBeVisible();
    expect(view.getByRole("article", { name: "Blog Watcher failed" })).toBeVisible();
    expect(view.getByRole("article", { name: "Blog Watcher was stopped" })).toBeVisible();
    const gone = view.getByRole("article", {
      name: "Blog Watcher was deleted before its job ended",
    });
    // Its run went with it.
    expect(within(gone).queryByRole("link")).toBeNull();
    expect(
      view.getByRole("article", { name: "Researcher asked about Blog Watcher" }),
    ).toHaveTextContent("This request is gone");
  });

  it("shows the ask card in the topic, and reads the cards again when making changes", async () => {
    const first = chat("a", "10:00");
    home([first]);
    const asked = card("asked", first);
    let cards: MakingCard[] = [asked];
    backend.on("making_cards", () => cards);
    backend.on("make_request_decide", () => ({ ...asked.card!.request, status: "refused" }));
    const { user, listening } = renderApp(`/home/${dot.id}`);
    await listening;
    const view = await conversation();
    const ask = await view.findByRole("article", { name: asked.card!.title });
    await user.click(within(ask).getByRole("button", { name: "Don't make it" }));
    await waitFor(() => expect(backend.callsTo("make_request_decide")).toHaveLength(1));

    // The core says the topic's making changed: a new card shows without a reload.
    cards = [asked, card("made", first)];
    act(() =>
      backend.emit("runtime", { type: "making_updated", dot_id: dot.id, root_run_id: "a" }),
    );
    expect(await view.findByRole("article", { name: "Made sol: Blog Watcher" })).toBeVisible();
  });

  it("draws the cards of a scheduled run under its card, and of its replies under them", async () => {
    const root = scheduled("cron", "09:00");
    const answer = reply("r2", root, "09:30");
    home([root, answer]);
    backend.on("making_cards", ({ rootRunId }) =>
      rootRunId === "cron"
        ? [card("made", root), card("deleted", answer), card("changed", answer, { run_id: "lost" })]
        : [],
    );
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const made = await view.findByRole("article", { name: "Made sol: Blog Watcher" });
    const deleted = await view.findByRole("article", { name: "Researcher deleted Blog Watcher" });
    // Each once; a card whose run is not shown is not lost.
    expect(view.getAllByRole("article", { name: "Made sol: Blog Watcher" })).toHaveLength(1);
    expect(before(made, view.getByText("reply r2"))).toBe(true);
    expect(before(view.getByText("reply r2"), deleted)).toBe(true);
    expect(view.getAllByRole("article", { name: "Researcher changed Blog Watcher" })).toHaveLength(
      1,
    );
  });

  it("draws a sol's name and a job's text as plain text", async () => {
    const first = chat("a", "10:00");
    home([first]);
    backend.on("making_cards", () => [
      card("job_started", first, {
        dot_name: "<img src=x onerror=alert(1)>",
        message: "[run](javascript:alert(2)) <script>alert(3)</script> **now**",
      }),
    ]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const working = await view.findByRole("article", {
      name: "<img src=x onerror=alert(1)> is working",
    });
    expect(working.querySelector("img, script, strong")).toBeNull();
    expect(working).toHaveTextContent(
      "[run](javascript:alert(2)) <script>alert(3)</script> **now**",
    );
    expect(within(working).getAllByRole("link").map((a) => a.getAttribute("href"))).toEqual([
      "/runs/01J0RUN2",
    ]);
  });
});

describe("a chat run the person did not type", () => {
  const job = (id: string, at: string, over: Partial<Run> = {}) =>
    chat(id, at, {
      payload: { message: "Read today's feeds.", job_id: "01J0JOB", job_from: "Researcher" },
      ...over,
    });

  it("shows a maker's job under its own header, not as the person's message", async () => {
    const run = job("a", "10:00");
    home([run], [said("a", 1, "Three new posts.")]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    const text = await view.findByText("Read today's feeds.");
    expect(text).toHaveAttribute("data-from", "maker");
    expect(view.getByText("Job from Researcher")).toBeVisible();
    expect(document.querySelector('[data-from="person"]')).toBeNull();
    // Nothing of the person's to move to another topic.
    expect(view.queryByRole("button", { name: /Move to topic/ })).toBeNull();
    expect(runTrigger(run)).toBe("Job from Researcher");
    expect(triggerLabel(run)).toBe("Job from Researcher");
    expect(chatMessage(run)).toBeNull();
  });

  it("shows Sage's report to a maker as Sage's, and a job sent again as the maker's", async () => {
    const asked = chat("a", "10:00");
    const wake = reply("b", asked, "10:20", {
      payload: {
        message: 'The sol "Blog Watcher" finished.\n> <b>Three</b> [new](javascript:alert(1)) posts.',
        wake_id: "01J0WAKE",
      },
    });
    const again = reply("c", wake, "10:30", {
      payload: { message: "Read today's feeds.", from_maker: "Researcher" },
    });
    home([asked, wake, again]);
    renderApp(`/home/${dot.id}`);
    const view = await conversation();
    // The person's own message keeps its bubble.
    expect(await view.findByText("question a")).toHaveAttribute("data-from", "person");
    const report = view.getByText(/The sol "Blog Watcher" finished\./);
    expect(report).toHaveAttribute("data-from", "sage");
    // Quoted text of a sol stays text.
    expect(report.querySelector("b, a")).toBeNull();
    expect(report).toHaveTextContent("<b>Three</b> [new](javascript:alert(1)) posts.");
    expect(view.getByText("Report from Sage")).toBeVisible();
    expect(view.getByText("Job from Researcher, sent again")).toBeVisible();
    expect(view.getByText("Read today's feeds.")).toHaveAttribute("data-from", "maker");
    expect(document.querySelectorAll('[data-from="person"]')).toHaveLength(1);
    expect(runTrigger(wake)).toBe("Report from Sage");
    expect(chatMessage(wake)).toBeNull();
    expect(chatMessage(again)).toBeNull();
    expect(chatMessage(asked)).toBe("question a");
  });
});
