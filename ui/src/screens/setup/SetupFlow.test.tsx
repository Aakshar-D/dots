import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { SetupAnswer, SetupApplied, SetupProposal } from "@/api/types";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const HEADING = "What should your first sol do?";
const PLACEHOLDER =
  "For example: every weekday at 9, read my Downloads folder and tell me what's new";
const FAILED = "Setup could not finish. Try describing the task again, or fill in the form.";
const DESCRIPTION = "Read my Downloads folder every weekday";

const proposal: SetupProposal = {
  name: "Downloads digest",
  instructions: "Read the Downloads folder and tell me what is new.",
  folder: null,
  schedule: null,
  preset: "read-only",
  extras: ["web", "mcp:gmail"],
  uses: { mcp_servers: ["gmail"], skills: ["pdf"], plugins: [], context: [], apps: [] },
  first_message: "Start with today's new files.",
  notes: "Runs when you ask.",
};
const proposed = (p: SetupProposal = proposal): SetupAnswer => ({
  kind: "proposal",
  proposal: p,
  new_dot_root: "C:\\Users\\me\\Dots",
});
const questions = (...qs: string[]): SetupAnswer => ({ kind: "questions", questions: qs });

/// The empty home, with `description` typed into the composer.
async function describe_(description = DESCRIPTION) {
  const view = renderApp("/home");
  const box = await screen.findByRole("textbox", { name: HEADING });
  await view.user.type(box, description);
  return view;
}

/// The empty home, past the composer, on the card of `proposal`.
async function onCard() {
  backend.on("setup_ask", () => proposed());
  const view = await describe_();
  await view.user.click(screen.getByRole("button", { name: "Set it up" }));
  await screen.findByRole("textbox", { name: "Name" });
  return view;
}

const request = (n: number) => backend.callsTo("setup_ask")[n]?.request;

describe("setting up a sol by describing it", () => {
  it("meets a person with no sols with the composer and the form link", async () => {
    renderApp("/home");
    expect(await screen.findByRole("heading", { name: HEADING })).toBeVisible();
    const box = screen.getByRole("textbox", { name: HEADING });
    expect(box).toHaveAttribute("placeholder", PLACEHOLDER);
    expect(screen.getByRole("button", { name: "Set it up" })).toBeDisabled();
    expect(screen.getByRole("link", { name: "Fill in the form instead" })).toHaveAttribute(
      "href",
      "/dots/new",
    );
  });

  it("asks questions, sends the answers with Continue, and shows the card", async () => {
    backend.on("setup_ask", () => questions("Which folder?", "How often?"));
    const { user } = await describe_();
    await user.click(screen.getByRole("button", { name: "Set it up" }));
    await user.type(await screen.findByRole("textbox", { name: "Which folder?" }), "Downloads");
    await user.type(screen.getByRole("textbox", { name: "How often?" }), "Weekdays at 9");
    backend.on("setup_ask", () => proposed());
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("textbox", { name: "Name" })).toHaveValue("Downloads digest");
    expect(backend.callsTo("setup_ask")).toEqual([
      {
        id: expect.stringMatching(/^setup-\d+$/),
        request: { description: DESCRIPTION, rounds: [], current: null, change: null, skip: false },
      },
      {
        id: backend.callsTo("setup_ask")[0]!.id,
        request: {
          description: DESCRIPTION,
          rounds: [{ questions: ["Which folder?", "How often?"], answers: ["Downloads", "Weekdays at 9"] }],
          current: null,
          change: null,
          skip: false,
        },
      },
    ]);
  });

  it("skips the questions with Skip, just propose", async () => {
    backend.on("setup_ask", () => questions("Which folder?"));
    const { user } = await describe_();
    await user.click(screen.getByRole("button", { name: "Set it up" }));
    await screen.findByRole("textbox", { name: "Which folder?" });
    backend.on("setup_ask", () => proposed());
    await user.click(screen.getByRole("button", { name: "Skip, just propose" }));
    await screen.findByRole("textbox", { name: "Name" });
    expect(request(1)).toEqual({
      description: DESCRIPTION,
      rounds: [],
      current: null,
      change: null,
      skip: true,
    });
  });

  it("shows the proposal as one card, and removing a server removes its extra", async () => {
    const { user } = await onCard();
    expect(screen.getByText("Runs when you ask.")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "What it does" })).toHaveValue(proposal.instructions);
    expect(screen.getByText("Only when you talk to it")).toBeVisible();
    expect(screen.getByText("A new folder: C:\\Users\\me\\Dots\\downloads-digest")).toBeVisible();
    expect(
      screen.getByText(
        "May read files. Never writes, runs commands, reads websites or uses MCP servers.",
      ),
    ).toBeVisible();
    expect(screen.getByText("Read websites and search the web without asking")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove Read websites and search the web without asking" }),
    ).toBeVisible();
    expect(screen.getByText("Use gmail without asking")).toBeVisible();
    const uses = screen.getByRole("list", { name: "Uses" });
    expect(within(uses).getByText("gmail")).toBeVisible();
    expect(within(uses).getByText("pdf")).toBeVisible();

    await user.click(within(uses).getByRole("button", { name: "Remove gmail" }));
    expect(within(uses).queryByText("gmail")).toBeNull();
    expect(screen.queryByText("Use gmail without asking")).toBeNull();
    // The folder follows the name.
    await user.type(screen.getByRole("textbox", { name: "Name" }), " 2");
    expect(screen.getByText("A new folder: C:\\Users\\me\\Dots\\downloads-digest-2")).toBeVisible();
  });

  it("picks a folder with Change, and goes back to a new one", async () => {
    backend.on("dialog_open", () => "D:\\Work\\Reports");
    const { user } = await onCard();
    await user.click(screen.getByRole("button", { name: "Change" }));
    expect(await screen.findByText("D:\\Work\\Reports")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use a new folder" }));
    expect(screen.getByText("A new folder: C:\\Users\\me\\Dots\\downloads-digest")).toBeVisible();
  });

  it("asks for changes with the card as edited, and replaces it", async () => {
    const { user } = await onCard();
    await user.click(
      screen.getByRole("button", { name: "Remove Read websites and search the web without asking" }),
    );
    expect(screen.getByRole("button", { name: "Ask for changes" })).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "What should change?" }), "Run at 8");
    backend.on("setup_ask", () => proposed({ ...proposal, name: "Morning digest" }));
    await user.click(screen.getByRole("button", { name: "Ask for changes" }));
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Morning digest"),
    );
    expect(request(1)).toEqual({
      description: DESCRIPTION,
      rounds: [],
      current: { ...proposal, extras: ["mcp:gmail"] },
      change: "Run at 8",
      skip: false,
    });
  });

  it("keeps the card and shows the error when a change fails", async () => {
    const { user } = await onCard();
    await user.type(screen.getByRole("textbox", { name: "What should change?" }), "Run at 8");
    backend.on("setup_ask", () => {
      throw { kind: "invalid", message: FAILED };
    });
    await user.click(screen.getByRole("button", { name: "Ask for changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(FAILED);
    expect(screen.getByRole("textbox", { name: "Name" })).toHaveValue("Downloads digest");
  });

  it("runs setup in the current account and opens the new sol", async () => {
    backend.on("scope_get", () => ({ account: "org-1", workspace: "ws-2" }));
    const applied = later<SetupApplied>();
    backend.on("setup_apply", () => applied.answer);
    const { user, router } = await onCard();
    const name = screen.getByRole("textbox", { name: "Name" });
    await user.clear(name);
    await user.type(name, "My digest");
    const run = screen.getByRole("button", { name: "Run setup" });
    await user.click(run);
    await waitFor(() => expect(run).toBeDisabled());
    expect(backend.callsTo("setup_apply")).toEqual([
      { proposal: { ...proposal, name: "My digest" }, orgId: "org-1", workspaceId: "ws-2" },
    ]);
    const dot = { ...sample.dot, id: "dot-new", name: "My digest", org_id: null };
    backend.on("dot_overview", () => [{ ...sample.overview, dot }]);
    applied.give({ dot, run_id: "run-1", run_error: null, notes: "" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/dot-new"));
  });

  it("sends Personal and the default workspace as none", async () => {
    backend.on("setup_apply", () => {
      throw { kind: "internal", message: "stop here" };
    });
    const { user } = await onCard();
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    await waitFor(() =>
      expect(backend.callsTo("setup_apply")).toEqual([
        { proposal, orgId: null, workspaceId: null },
      ]),
    );
  });

  it("says when the first run did not start", async () => {
    const dot = { ...sample.dot, id: "dot-new" };
    backend.on("setup_apply", () => {
      backend.on("dot_overview", () => [{ ...sample.overview, dot }]);
      return { dot, run_id: null, run_error: "the queue is full", notes: "" };
    });
    const { user } = await onCard();
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    expect(
      await screen.findByText(
        "The sol was created but its first run did not start: the queue is full",
      ),
    ).toBeVisible();
  });

  it("shows the first message after What it does, and Run setup sends it as edited", async () => {
    backend.on("setup_apply", () => {
      throw { kind: "internal", message: "stop here" };
    });
    const { user } = await onCard();
    const first = screen.getByRole("textbox", { name: "First message" });
    expect(first).toHaveValue(proposal.first_message);
    const does = screen.getByRole("textbox", { name: "What it does" });
    expect(does.compareDocumentPosition(first) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.clear(first);
    await user.type(first, "Show me yesterday's files.");
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    await waitFor(() =>
      expect(backend.callsTo("setup_apply")).toEqual([
        {
          proposal: { ...proposal, first_message: "Show me yesterday's files." },
          orgId: null,
          workspaceId: null,
        },
      ]),
    );
  });

  it("allows a blank first message", async () => {
    backend.on("setup_apply", () => {
      throw { kind: "internal", message: "stop here" };
    });
    const { user } = await onCard();
    await user.clear(screen.getByRole("textbox", { name: "First message" }));
    const run = screen.getByRole("button", { name: "Run setup" });
    expect(run).toBeEnabled();
    await user.click(run);
    await waitFor(() =>
      expect(backend.callsTo("setup_apply")).toEqual([
        { proposal: { ...proposal, first_message: "" }, orgId: null, workspaceId: null },
      ]),
    );
  });

  it("shows what the check at Run setup changed", async () => {
    const dot = { ...sample.dot, id: "dot-new" };
    const notes = "Left out the extra telepathy: Sage does not know it.";
    backend.on("setup_apply", () => {
      backend.on("dot_overview", () => [{ ...sample.overview, dot }]);
      return { dot, run_id: "run-1", run_error: null, notes };
    });
    const { user } = await onCard();
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    expect(await screen.findByText(notes)).toBeVisible();
  });

  it("says the extras add to the preset sentence, only while there are extras", async () => {
    const { user } = await onCard();
    expect(screen.getByText("Plus the extras below.")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Remove Read websites and search the web without asking" }),
    );
    await user.click(screen.getByRole("button", { name: "Remove Use gmail without asking" }));
    expect(screen.queryByText("Plus the extras below.")).toBeNull();
    expect(
      screen.getByText(
        "May read files. Never writes, runs commands, reads websites or uses MCP servers.",
      ),
    ).toBeVisible();
  });

  it("keeps the card with a name clash above Run setup", async () => {
    backend.on("setup_apply", () => {
      throw { kind: "conflict", message: 'A sol named "Downloads digest" already exists' };
    });
    const { user, router } = await onCard();
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent('A sol named "Downloads digest" already exists');
    const run = screen.getByRole("button", { name: "Run setup" });
    expect(run).toBeEnabled();
    expect(alert.compareDocumentPosition(run) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(router.state.location.pathname).toBe("/home");
  });

  it("shows a name clash after a failed change, not the old error", async () => {
    const { user } = await onCard();
    await user.type(screen.getByRole("textbox", { name: "What should change?" }), "Run at 8");
    backend.on("setup_ask", () => {
      throw { kind: "invalid", message: FAILED };
    });
    await user.click(screen.getByRole("button", { name: "Ask for changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(FAILED);
    backend.on("setup_apply", () => {
      throw { kind: "conflict", message: 'A sol named "X" already exists' };
    });
    await user.click(screen.getByRole("button", { name: "Run setup" }));
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent('A sol named "X" already exists'),
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent(FAILED);
  });

  it("keeps Run setup off while the schedule cannot be saved", async () => {
    const { user } = await onCard();
    expect(screen.getByRole("button", { name: "Run setup" })).toBeEnabled();
    await user.selectOptions(screen.getByRole("combobox", { name: "Kind" }), "cron");
    expect(await screen.findByText("Write a cron expression.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Run setup" })).toBeDisabled();
  });

  it("sends New sol to the composer on the empty home, without a second setup", async () => {
    const { user } = renderApp("/home");
    const box = await screen.findByRole("textbox", { name: HEADING });
    const column = screen.getByRole("navigation", { name: "Sols" });
    await user.click(within(column).getByRole("button", { name: "New sol" }));
    expect(box).toHaveFocus();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("cancels the request when the view goes away while the agent works", async () => {
    backend.on("setup_ask", () => later<SetupAnswer>().answer);
    backend.on("setup_cancel", () => null);
    const { user, unmount } = await describe_();
    await user.click(screen.getByRole("button", { name: "Set it up" }));
    expect(await screen.findByText("Setting up...")).toBeVisible();
    unmount();
    expect(backend.callsTo("setup_cancel")).toEqual([{ id: backend.callsTo("setup_ask")[0]!.id }]);
  });

  it("goes back to the composer with Cancel, and drops the late answer", async () => {
    const answer = later<SetupAnswer>();
    backend.on("setup_ask", () => answer.answer);
    backend.on("setup_cancel", () => null);
    const { user } = await describe_();
    await user.click(screen.getByRole("button", { name: "Set it up" }));
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("setup_cancel")).toHaveLength(1);
    answer.give(proposed());
    expect(await screen.findByRole("textbox", { name: HEADING })).toHaveValue(DESCRIPTION);
    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
  });

  it("confirms Start over when a card is on screen", async () => {
    const { user } = await onCard();
    await user.click(screen.getByRole("button", { name: "Start over" }));
    const confirm = await screen.findByRole("alertdialog", { name: "Discard the proposed sol?" });
    await user.click(within(confirm).getByRole("button", { name: "Start over" }));
    expect(await screen.findByRole("textbox", { name: HEADING })).toHaveValue("");
    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
  });

  it("points to the Claude Code CLI when it is not found", async () => {
    backend.on("status", () => ({ ...quietStatus, claude: { ...quietStatus.claude, program: null } }));
    backend.on("setup_ask", () => {
      throw { kind: "internal", message: "Claude could not be started" };
    });
    const { user } = await describe_();
    await user.click(screen.getByRole("button", { name: "Set it up" }));
    expect(await screen.findByText("Claude could not be started")).toBeVisible();
    expect(
      screen.getByText("Sage uses your Claude Code CLI. Install it and sign in, then try again."),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Fill in the form instead" })).toBeVisible();
  });
});

describe("New sol", () => {
  const reader = { ...sample.dot, id: "dot-a", name: "reader" };

  /// The home with `reader` open and the New dot dialog showing.
  async function openDialog() {
    backend.on("dot_overview", () => [{ ...sample.overview, dot: reader }]);
    const view = renderApp("/home/dot-a");
    await screen.findByRole("heading", { level: 1, name: "reader" });
    const column = screen.getByRole("navigation", { name: "Sols" });
    await view.user.click(within(column).getByRole("button", { name: "New sol" }));
    const dialog = await screen.findByRole("dialog", { name: "New sol" });
    return { ...view, dialog };
  }

  it("opens the new sol's conversation after Run setup", async () => {
    backend.on("setup_ask", () => proposed());
    const dot = { ...sample.dot, id: "dot-new", name: "Downloads digest" };
    backend.on("setup_apply", () => {
      // The overview that lists the new dot comes back a moment later, as the real one may.
      backend.on(
        "dot_overview",
        () =>
          new Promise((give) =>
            setTimeout(
              () => give([{ ...sample.overview, dot }, { ...sample.overview, dot: reader }]),
              100,
            ),
          ),
      );
      return { dot, run_id: "run-1", run_error: null, notes: "" };
    });
    const { user, dialog, router } = await openDialog();
    await user.type(within(dialog).getByRole("textbox", { name: "What should the sol do?" }), "x");
    await user.click(within(dialog).getByRole("button", { name: "Set it up" }));
    await user.click(await within(dialog).findByRole("button", { name: "Run setup" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Downloads digest" }),
    ).toBeVisible();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(router.state.location.pathname).toBe("/home/dot-new");
  });

  it("asks before closing with a proposed sol", async () => {
    backend.on("setup_ask", () => proposed());
    const { user, dialog } = await openDialog();
    await user.type(within(dialog).getByRole("textbox", { name: "What should the sol do?" }), "x");
    await user.click(within(dialog).getByRole("button", { name: "Set it up" }));
    await within(dialog).findByRole("button", { name: "Run setup" });
    await user.keyboard("{Escape}");
    const confirm = await screen.findByRole("alertdialog", { name: "Discard the proposed sol?" });
    await user.click(within(confirm).getByRole("button", { name: "Keep it" }));
    expect(screen.getByRole("dialog", { name: "New sol" })).toBeVisible();
    await user.keyboard("{Escape}");
    await user.click(
      within(
        await screen.findByRole("alertdialog", { name: "Discard the proposed sol?" }),
      ).getByRole("button", { name: "Close" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("cannot be closed while Run setup is making the sol", async () => {
    backend.on("setup_ask", () => proposed());
    const applied = later<SetupApplied>();
    backend.on("setup_apply", () => applied.answer);
    const { user, dialog, router } = await openDialog();
    await user.type(within(dialog).getByRole("textbox", { name: "What should the sol do?" }), "x");
    await user.click(within(dialog).getByRole("button", { name: "Set it up" }));
    await user.click(await within(dialog).findByRole("button", { name: "Run setup" }));
    await waitFor(() => expect(backend.callsTo("setup_apply")).toHaveLength(1));
    expect(within(dialog).queryByRole("button", { name: "Close" })).toBeNull();
    await user.keyboard("{Escape}");
    await user.click(document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]')!);
    expect(screen.getByRole("dialog", { name: "New sol" })).toBeVisible();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    const dot = { ...sample.dot, id: "dot-new", name: "Downloads digest" };
    backend.on("dot_overview", () => [{ ...sample.overview, dot }, { ...sample.overview, dot: reader }]);
    applied.give({ dot, run_id: "run-1", run_error: null, notes: "" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/dot-new"));
  });

  it("opens the setup in a dialog, with the form link", async () => {
    const dot = { ...sample.dot, id: "dot-a", name: "reader" };
    backend.on("dot_overview", () => [{ ...sample.overview, dot }]);
    const { user } = renderApp("/home/dot-a");
    const column = await screen.findByRole("navigation", { name: "Sols" });
    await user.click(within(column).getByRole("button", { name: "New sol" }));
    const dialog = await screen.findByRole("dialog", { name: "New sol" });
    expect(within(dialog).getByRole("button", { name: "Set it up" })).toBeVisible();
    expect(within(dialog).getByRole("textbox", { name: "What should the sol do?" })).toHaveAttribute(
      "placeholder",
      PLACEHOLDER,
    );
    expect(within(dialog).getByRole("link", { name: "Fill in the form instead" })).toHaveAttribute(
      "href",
      "/dots/new",
    );
  });
});
