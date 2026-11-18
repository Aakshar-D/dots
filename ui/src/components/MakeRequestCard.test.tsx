import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { MakeRequestCard as Card } from "@/api/types";
import { MakeRequestCard } from "@/components/MakeRequestCard";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";

const cards = T.MakeRequestCard.array().parse(contract.MakeRequestCard);
const of = (id: string) => cards.find((c) => c.request.id === id)!;
const make = of("01J0MRQ");
const change = of("01J0MRQ2");
const job = of("01J0MRQ3");
const blocked = of("01J0MRQ4");
const gone = of("01J0MRQ5");
const allowed = of("01J0MRQ6");
const refused = of("01J0MRQ7");

function show(card: Card) {
  const client = createQueryClient();
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <MakeRequestCard card={card} />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  const again = (next: Card) =>
    view.rerender(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <MakeRequestCard card={next} />
        </MemoryRouter>
      </QueryClientProvider>,
    );
  return { ...view, again, user: userEvent.setup() };
}

const article = (card: Card) => screen.getByRole("article", { name: card.title });
const button = (name: string) => screen.getByRole("button", { name });
const decided = () => backend.callsTo("make_request_decide");

describe("a maker's request for a new sol", () => {
  it("says what is asked, what is beyond the maker's reach, and whose words the rest are", () => {
    show(make);
    const card = article(make);
    expect(card).toHaveTextContent("Researcher wants to make a sol with more than it has");
    expect(card).toHaveTextContent("Blog Watcher · Reads the feeds");
    const beyond = within(card).getByRole("region", { name: "Beyond what Researcher has" });
    expect(within(beyond).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "The browser",
      "4th sol in this run; the limit is 3",
    ]);
    // What the maker wrote stands apart, and says so.
    const instructions = within(card).getByRole("figure", { name: "Instructions" });
    expect(instructions).toHaveTextContent("written by Researcher, not by Sage");
    expect(instructions).toHaveTextContent("Read the feeds each morning.");
    expect(within(card).getByRole("figure", { name: "Model" })).toHaveTextContent(
      make.request.spec.model,
    );
    expect(within(card).getByRole("figure", { name: "First job" })).toHaveTextContent(
      "Read today's feeds.",
    );
    expect(button("Allow for this sol")).toBeEnabled();
    expect(button("Make it without these")).toBeEnabled();
    expect(button("Don't make it")).toBeEnabled();
    // The maker is linked by its id, never by anything it wrote.
    expect(within(card).getByRole("link", { name: "Researcher" })).toHaveAttribute(
      "href",
      "/dots/01J0DOT",
    );
  });

  it.each([
    ["Allow for this sol", "allow"],
    ["Make it without these", "without"],
    ["Don't make it", "refuse"],
  ])("sends %s with the limit and the digest of the card as it was read", async (name, decision) => {
    backend.on("make_request_decide", () => ({
      ...make.request,
      status: decision === "allow" ? "allowed" : decision === "without" ? "made_without" : "refused",
      made_dot_id: decision === "refuse" ? null : "01J0MADE",
    }));
    const { user } = show(make);
    await user.click(button(name));
    await waitFor(() =>
      expect(decided()).toEqual([
        { id: make.request.id, decision, limit: make.limit, shown: make.shown },
      ]),
    );
    expect(make.limit).toBe("4th sol in this run; the limit is 3");
    expect(make.shown).toMatch(/\S/);
    // The answer shows at once, and nothing more can be answered.
    await waitFor(() => expect(screen.queryByRole("button", { name })).toBeNull());
    expect(article(make)).toHaveTextContent(
      decision === "allow"
        ? "You allowed it: the sol was made as asked."
        : decision === "without"
          ? "You chose to make it without what was beyond the maker's reach."
          : "You chose not to make it.",
    );
  });

  it("sends null for a card that shows no limit and nothing beyond reach", async () => {
    const plain: Card = { ...make, limit: null, shown: null, lines: [], can_without: false };
    backend.on("make_request_decide", () => ({ ...plain.request, status: "allowed" }));
    const { user } = show(plain);
    // Nothing to leave out: no "without".
    expect(screen.queryByRole("button", { name: "Make it without these" })).toBeNull();
    await user.click(button("Allow for this sol"));
    await waitFor(() =>
      expect(decided()).toEqual([
        { id: plain.request.id, decision: "allow", limit: null, shown: null },
      ]),
    );
  });

  it("keeps Allow off with the core's reason, and still lets the person refuse", async () => {
    backend.on("make_request_decide", () => ({ ...blocked.request, status: "refused" }));
    const { user } = show(blocked);
    const allow = button("Allow for this sol");
    expect(allow).toBeDisabled();
    expect(allow).toHaveAttribute("title", blocked.blocked);
    expect(screen.getByLabelText("Why Allow is off")).toHaveTextContent(
      "Acme: a Viewer can't change sols. Ask an Admin or Owner.",
    );
    expect(screen.queryByRole("button", { name: "Make it without these" })).toBeNull();
    await user.click(button("Don't make it"));
    await waitFor(() => expect(decided()).toHaveLength(1));
    expect(decided()[0]).toMatchObject({ decision: "refuse" });
  });

  it("offers Without while Allow is off, when the core says so", () => {
    show({ ...blocked, can_without: true });
    expect(button("Allow for this sol")).toBeDisabled();
    expect(button("Make it without these")).toBeEnabled();
  });

  it("shows the core's refusal of an answer this PC may not give", async () => {
    backend.on("make_request_decide", () => {
      throw { kind: "not_allowed", message: "Acme: a Viewer can't change sols." };
    });
    const { user } = show(make);
    await user.click(button("Allow for this sol"));
    expect(await screen.findByText("Acme: a Viewer can't change sols.")).toBeVisible();
    // Not a changed card: nothing is read again.
    expect(backend.callsTo("make_request_card")).toEqual([]);
  });
});

describe("text a sol wrote on the card", () => {
  const hostile: Card = {
    ...make,
    name: "<img src=x onerror=alert(1)>",
    instructions: "# Heading\n[click](javascript:alert(1)) <script>alert(2)</script> **bold**",
    lines: ["The MCP server [x](javascript:alert(3))"],
    request: {
      ...make.request,
      purpose: "<b>bold</b> [p](https://evil.example)",
      first_job: "<img src=x onerror=alert(4)> [job](javascript:alert(5))",
      spec: { ...make.request.spec, model: "<a href='javascript:alert(6)'>m</a>" },
    },
  };

  it("is drawn as plain text: no markup, no Markdown, no link", () => {
    show(hostile);
    const card = article(hostile);
    expect(card.querySelector("img, script, b, strong, h1")).toBeNull();
    // The only links are Sage's own, to ids the core gave.
    const links = within(card).getAllByRole("link");
    expect(links.map((a) => a.getAttribute("href"))).toEqual(["/dots/01J0DOT"]);
    expect(card.innerHTML).not.toContain('href="javascript');
    // The text is there, character for character.
    expect(card).toHaveTextContent("<img src=x onerror=alert(1)>");
    expect(within(card).getByRole("figure", { name: "Instructions" })).toHaveTextContent(
      "# Heading [click](javascript:alert(1)) <script>alert(2)</script> **bold**",
    );
    expect(within(card).getByRole("figure", { name: "First job" })).toHaveTextContent(
      "<img src=x onerror=alert(4)> [job](javascript:alert(5))",
    );
    expect(within(card).getByRole("figure", { name: "Model" })).toHaveTextContent(
      "<a href='javascript:alert(6)'>m</a>",
    );
    expect(card).toHaveTextContent("The MCP server [x](javascript:alert(3))");
  });

  it("names no button", () => {
    show(hostile);
    expect(
      within(article(hostile))
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Allow for this sol", "Make it without these", "Don't make it"]);
  });
});

describe("a card that changed before the answer", () => {
  const now: Card = {
    ...make,
    lines: ["The browser", "The MCP server 'github'", "5th sol in this run; the limit is 3"],
    limit: "5th sol in this run; the limit is 3",
    shown: "digest-of-the-new-lines",
  };
  const conflict = {
    kind: "conflict",
    message:
      "What this request would give changed since the card was shown. Look at the card again, and answer again.",
  };

  it("reads the card again, says it changed, and only then allows with the new card", async () => {
    let refuse = true;
    backend.on("make_request_decide", () => {
      if (refuse) throw conflict;
      return { ...now.request, status: "allowed", made_dot_id: "01J0MADE" };
    });
    backend.on("make_request_card", () => now);
    const { user } = show(make);
    await user.click(button("Allow for this sol"));
    // The card as it is now, with the core's words for why.
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("What this request would give changed since the card was shown");
    expect(alert).toHaveTextContent("Read the card again before you answer.");
    expect(backend.callsTo("make_request_card")).toEqual([{ id: make.request.id }]);
    expect(await screen.findByText("The MCP server 'github'")).toBeVisible();
    // Nothing was sent again by itself.
    expect(decided()).toHaveLength(1);

    refuse = false;
    await user.click(button("Allow for this sol"));
    await waitFor(() => expect(decided()).toHaveLength(2));
    expect(decided()[1]).toEqual({
      id: make.request.id,
      decision: "allow",
      limit: "5th sol in this run; the limit is 3",
      shown: "digest-of-the-new-lines",
    });
    expect(await screen.findByText("You allowed it: the sol was made as asked.")).toBeVisible();
  });

  it("shows the answer it already has, without an error", async () => {
    backend.on("make_request_decide", () => {
      throw { kind: "conflict", message: "That request was already answered." };
    });
    backend.on("make_request_card", () => refused);
    const { user } = show({ ...make, request: { ...make.request, id: refused.request.id } });
    await user.click(button("Allow for this sol"));
    expect(await screen.findByText("You chose not to make it.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Allow for this sol" })).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says the request is gone when it went with its maker", async () => {
    backend.on("make_request_decide", () => {
      throw { kind: "conflict", message: "That request was already answered." };
    });
    backend.on("make_request_card", () => {
      throw { kind: "not_found", message: "make request 01J0MRQ" };
    });
    const { user } = show(make);
    await user.click(button("Allow for this sol"));
    expect(await screen.findByText(/This request is gone/)).toBeVisible();
    expect(screen.queryAllByRole("button")).toEqual([]);
  });

  it("keeps Allow off when the card cannot be read again", async () => {
    backend.on("make_request_decide", () => {
      throw conflict;
    });
    backend.on("make_request_card", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    const { user } = show(make);
    await user.click(button("Allow for this sol"));
    expect(await screen.findByText(/The card could not be read again/)).toBeVisible();
    expect(button("Allow for this sol")).toBeDisabled();
    expect(button("Make it without these")).toBeDisabled();
    expect(button("Don't make it")).toBeEnabled();
  });

  it("says so when the list brings a card that says something else", () => {
    const { again } = show(make);
    expect(screen.queryByRole("alert")).toBeNull();
    again(now);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "This request changed since you first saw it. Read the card again before you answer.",
    );
    expect(screen.getByText("The MCP server 'github'")).toBeVisible();
  });
});

describe("a request to change a made sol", () => {
  it("lists every setting the change names, and words the answers for a change", () => {
    show(change);
    const card = article(change);
    expect(card).toHaveTextContent("Researcher wants to give Blog Watcher more than it has");
    const sets = within(card).getByRole("region", { name: "What the change sets" });
    expect(within(sets).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "New instructions",
      "The browser",
    ]);
    expect(card).toHaveTextContent("Schedule: daily at 09:00");
    expect(within(card).getByRole("figure", { name: "New instructions" })).toBeVisible();
    // A change shows no model of its own and no first job.
    expect(within(card).queryByRole("figure", { name: "Model" })).toBeNull();
    expect(within(card).queryByRole("figure", { name: "First job" })).toBeNull();
    expect(button("Allow for this sol")).toBeEnabled();
    expect(button("Change it without these")).toBeEnabled();
    expect(button("Don't change it")).toBeEnabled();
  });
});

describe("a request to give a made sol a job", () => {
  it("shows the job as the maker's, where its answer goes, and no Without", async () => {
    backend.on("make_request_decide", () => ({ ...job.request, status: "allowed" }));
    // Even if the core said Without were on, a job has none.
    const { user } = show({ ...job, can_without: true });
    const card = article(job);
    expect(card).toHaveTextContent(
      "Researcher wants to give a job to Blog Watcher, which has more than Researcher has",
    );
    const held = within(card).getByRole("region", {
      name: "What Blog Watcher has that Researcher does not",
    });
    expect(within(held).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "The browser",
    ]);
    const text = within(card).getByRole("figure", { name: "Job from Researcher" });
    expect(text).toHaveTextContent("written by Researcher, not by Sage");
    expect(text).toHaveTextContent("Read today's feeds.");
    expect(card).toHaveTextContent("Its final message goes back to Researcher.");
    expect(
      within(card)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual(["Allow the job", "Don't give it"]);
    await user.click(button("Allow the job"));
    await waitFor(() =>
      expect(decided()).toEqual([
        { id: job.request.id, decision: "allow", limit: job.limit, shown: job.shown },
      ]),
    );
    expect(await screen.findByText("You allowed it: the job was given.")).toBeVisible();
  });
});

describe("a request that is no longer waiting", () => {
  it("leaves only the refusal for one whose named things have gone, and says what went", () => {
    show(gone);
    expect(screen.getByLabelText("Why Allow is off")).toHaveTextContent(
      "The folder C:\\feeds no longer exists.",
    );
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(["Don't make it"]);
  });

  it("shows how it was answered, with the sol it made", () => {
    show(allowed);
    const card = article(allowed);
    expect(card).toHaveTextContent("You allowed it: the sol was made as asked.");
    expect(within(card).getByRole("link", { name: "Open sol" })).toHaveAttribute(
      "href",
      "/dots/01J0MADE",
    );
    expect(within(card).queryAllByRole("button")).toEqual([]);
  });

  it("shows a refusal and a dropped request as their end, not as an error", () => {
    const { unmount } = show(refused);
    expect(article(refused)).toHaveTextContent("You chose not to make it.");
    expect(screen.queryAllByRole("button")).toEqual([]);
    unmount();
    const dropped: Card = { ...refused, request: { ...refused.request, status: "discarded" } };
    show(dropped);
    expect(article(dropped)).toHaveTextContent("This request was dropped without an answer.");
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
