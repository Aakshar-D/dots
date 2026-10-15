import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox, Schedule } from "@/api/types";
import { specOf } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { invalidateDots } from "@/lib/query";
import { backend } from "@/test/backend";
import { blankSpec, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
const path = `/dots/${dot.id}`;

/// What the cron field saves.
const cron = (expr: string) =>
  ({ rule: { kind: "cron", expr }, catch_up: false, until: null }) satisfies Schedule;

describe("the dot editor, new dot", () => {
  it("creates a dot from the core's defaults and what was typed", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.click(screen.getByRole("button", { name: "Create dot" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      {
        spec: { ...blankSpec, name: "nightly", instructions: "Run the tests.", workdir: "C:/dev/app" },
      },
    ]);
    expect(await screen.findByText("Dot created")).toBeVisible();
  });

  it("marks what is missing instead of saving, and only after a save was tried", async () => {
    const { user } = renderApp("/dots/new");
    await screen.findByLabelText("Name");
    expect(screen.queryByText("Say what the dot should do.")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the marked fields");
    expect(screen.getByText("Use 1 to 64 letters, digits, '-' or '_'.")).toBeVisible();
    expect(screen.getByText("Say what the dot should do.")).toBeVisible();
    expect(screen.getByText("Choose the folder the dot works in.")).toBeVisible();
    expect(backend.callsTo("create_dot")).toEqual([]);

    // The marks follow the typing.
    await user.type(screen.getByLabelText("Name"), "nightly");
    expect(screen.queryByText("Use 1 to 64 letters, digits, '-' or '_'.")).toBeNull();
  });

  it("asks a local dot for its endpoint and hides what only Claude dots have", async () => {
    const { user } = renderApp("/dots/new");
    expect(await screen.findByLabelText("MCP servers")).toBeVisible();
    expect(screen.queryByLabelText("Endpoint")).toBeNull();

    await user.selectOptions(screen.getByLabelText("Engine"), "local");
    expect(screen.queryByLabelText("MCP servers")).toBeNull();
    expect(screen.queryByLabelText("Load my Claude Code user settings")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByText("A local dot needs its endpoint, starting with http://.")).toBeVisible();
    // Endpoints from Settings are offered.
    const endpoint = screen.getByLabelText("Endpoint");
    const offered = document.getElementById(endpoint.getAttribute("list")!)!;
    expect(within(offered).getAllByRole("option", { hidden: true })[0]).toHaveValue(
      sample.settings.local_endpoints[0],
    );
  });

  it("takes the folder from the native picker", async () => {
    backend.on("dialog_open", () => "C:\\dev\\picked");
    const { user } = renderApp("/dots/new");
    await user.click(await screen.findByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(screen.getByLabelText("Folder")).toHaveValue("C:\\dev\\picked"));
    expect(backend.callsTo("dialog_open")[0]).toMatchObject({ directory: true });

    // A cancelled picker changes nothing.
    backend.on("dialog_open", () => null);
    await user.click(screen.getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(backend.callsTo("dialog_open")).toHaveLength(2));
    expect(screen.getByLabelText("Folder")).toHaveValue("C:\\dev\\picked");
  });

  it("previews the next runs of a schedule and shows the core's objection to a bad one", async () => {
    const { user } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Cron expression"), "0 0 2 * * *");
    const next = await screen.findByRole("list", { name: "Next runs" });
    expect(within(next).getAllByRole("listitem").map((li) => li.textContent)).toEqual(
      ["2026-10-13", "2026-10-14", "2026-10-15"].map((d) => localDateTime(`${d}T02:00:00+01:00`)),
    );
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({
      schedule: cron("0 0 2 * * *"),
      count: 3,
    });

    backend.on("preview_schedule", () => {
      throw { kind: "invalid", message: "invalid cron expression: every" };
    });
    await user.clear(screen.getByLabelText("Cron expression"));
    await user.type(screen.getByLabelText("Cron expression"), "every");
    expect(await screen.findByText("invalid cron expression: every")).toBeVisible();
    expect(screen.queryByRole("list", { name: "Next runs" })).toBeNull();

    // A bad schedule blocks the save even when everything else is filled in.
    await user.type(screen.getByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(backend.callsTo("create_dot")).toEqual([]);
  });

  it("saves the cron expression as a cron schedule without catch-up", async () => {
    backend.on("create_dot", (args) => ({ ...dot, ...(args.spec as object), id: "dot-new" }));
    backend.on("get_dot", () => ({ ...dot, id: "dot-new", name: "nightly" }));
    const { user, router } = renderApp("/dots/new");
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.type(screen.getByLabelText("Cron expression"), " 0 0 2 * * * ");
    await user.click(screen.getByRole("button", { name: "Create dot" }));

    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      {
        spec: {
          ...blankSpec,
          name: "nightly",
          instructions: "Run the tests.",
          workdir: "C:/dev/app",
          schedule: cron("0 0 2 * * *"),
        },
      },
    ]);
  });

  it("refuses MCP servers that are not a JSON object", async () => {
    const { user } = renderApp("/dots/new");
    // user-event reads "{{" as one typed "{".
    await user.type(await screen.findByLabelText("MCP servers"), "{{nope");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByText("This is not valid JSON.")).toBeVisible();
  });
});

describe("the dot editor, stored dot", () => {
  /// A backend holding the dot. A test changes `held.dot` the way another command would.
  function stored() {
    const held = { dot };
    backend.on("get_dot", () => held.dot);
    backend.on("update_dot", (args) => {
      held.dot = { ...held.dot, ...(args.spec as object) };
      return held.dot;
    });
    return held;
  }

  it("loads the dot and saves a change", async () => {
    stored();
    const { user } = renderApp(path);
    expect(await screen.findByRole("heading", { name: dot.name })).toBeVisible();
    expect(screen.getByLabelText("Name")).toHaveValue(dot.name);
    expect(dot.schedule?.rule.kind).toBe("cron");
    expect(screen.getByLabelText("Cron expression")).toHaveValue(cron("0 0 2 * * *").rule.expr);
    expect(screen.getByLabelText("MCP servers")).toHaveValue(
      JSON.stringify(dot.mcp_servers, null, 2),
    );

    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
    await user.type(instructions, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), instructions: "Only summarize." } },
    ]);
  });

  it("shows the core's refusal when a save fails", async () => {
    stored();
    backend.on("update_dot", () => {
      throw { kind: "invalid", message: "policy rule \"Bash(*)\": '*' is only allowed as a trailing ':*'" };
    });
    const { user } = renderApp(path);
    await user.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("only allowed as a trailing");
  });

  it("clears the schedule when the cron expression is emptied", async () => {
    stored();
    const { user } = renderApp(path);
    await user.clear(await screen.findByLabelText("Cron expression"));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), schedule: null } },
    ]);
  });

  it("shows a schedule it cannot edit yet and keeps it on save", async () => {
    const daily: Schedule = {
      rule: { kind: "weekly", days: ["mon", "fri"], time: "09:00" },
      catch_up: true,
      until: { count: 10 },
    };
    const held = stored();
    held.dot = { ...dot, schedule: daily };
    const { user } = renderApp(path);
    expect(await screen.findByText("Weekly on mon, fri at 09:00, at most 10 runs")).toBeVisible();
    expect(screen.queryByLabelText("Cron expression")).toBeNull();
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ schedule: daily, count: 3 });

    const instructions = screen.getByLabelText("Instructions");
    await user.clear(instructions);
    await user.type(instructions, "Only summarize.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(held.dot), instructions: "Only summarize." } },
    ]);

    // Removed, the cron field is back.
    await user.click(screen.getByRole("button", { name: "Remove schedule" }));
    expect(screen.getByLabelText("Cron expression")).toHaveValue("");
  });

  const endpoint = "http://127.0.0.1:1234";

  it("saves a dot switched to the local engine without its MCP servers", async () => {
    stored();
    const { user } = renderApp(path);
    await user.selectOptions(await screen.findByLabelText("Engine"), "local");
    await user.type(screen.getByLabelText("Endpoint"), endpoint);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    // The dot was stored with MCP servers, which only the Claude engine has.
    expect(dot.mcp_servers).not.toBeNull();
    expect(backend.callsTo("update_dot")).toEqual([
      {
        id: dot.id,
        spec: { ...specOf(dot), engine: "local", endpoint_url: endpoint, mcp_servers: null },
      },
    ]);
  });

  it("does not hold a local dot back for MCP text its form no longer shows", async () => {
    stored();
    const { user } = renderApp(path);
    // user-event reads "{{" as one typed "{". Added to the stored servers, it is not JSON.
    await user.type(await screen.findByLabelText("MCP servers"), "{{nope");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Fix the marked fields");
    expect(screen.getByText("This is not valid JSON.")).toBeVisible();

    await user.selectOptions(screen.getByLabelText("Engine"), "local");
    await user.type(screen.getByLabelText("Endpoint"), endpoint);
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toHaveLength(1);
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      spec: { engine: "local", mcp_servers: null },
    });
  });

  it("deletes the dot only after a confirmation", async () => {
    stored();
    backend.on("delete_dot", () => null);
    const { user, router } = renderApp(path);
    await user.click(await screen.findByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("alertdialog", { name: `Delete ${dot.name}?` });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(backend.callsTo("delete_dot")).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(await screen.findByRole("button", { name: "Delete the dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots"));
    expect(backend.callsTo("delete_dot")).toEqual([{ id: dot.id }]);
  });

  it("shows the webhook, copies it, and makes a new token after a confirmation", async () => {
    const held = stored();
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, webhook_token: "tok-new" };
      return "tok-new";
    });
    const { user } = renderApp(path);
    const url = `http://127.0.0.1:47321/dots/${dot.id}/trigger`;
    await waitFor(() => expect(screen.getByLabelText("Webhook URL")).toHaveValue(url));
    expect(screen.getByLabelText("Token")).toHaveValue(dot.webhook_token);

    await user.click(screen.getByRole("button", { name: "Copy the webhook URL" }));
    expect(await navigator.clipboard.readText()).toBe(url);

    await user.click(screen.getByRole("button", { name: "Regenerate" }));
    await user.click(await screen.findByRole("button", { name: "Make a new token" }));
    await waitFor(() => expect(screen.getByLabelText("Token")).toHaveValue("tok-new"));
    expect(backend.callsTo("regenerate_webhook_token")).toEqual([{ id: dot.id }]);
  });

  /// Types new instructions and leaves them unsaved.
  async function typeInstructions(user: ReturnType<typeof renderApp>["user"], text: string) {
    const instructions = await screen.findByLabelText("Instructions");
    await user.clear(instructions);
    await user.type(instructions, text);
  }
  async function regenerate(user: ReturnType<typeof renderApp>["user"]) {
    await user.click(screen.getByRole("button", { name: "Regenerate" }));
    await user.click(await screen.findByRole("button", { name: "Make a new token" }));
  }

  it("keeps unsaved edits when the dot is loaded again", async () => {
    const held = stored();
    // The copy loaded after the new token differs from the form in more than the token.
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, name: "renamed", instructions: "Stored.", webhook_token: "tok-new" };
      return "tok-new";
    });
    const { user } = renderApp(path);
    await typeInstructions(user, "Only summarize.");
    await regenerate(user);

    // The reload has arrived: the page shows what it brought.
    expect(await screen.findByRole("heading", { name: "renamed" })).toBeVisible();
    expect(screen.getByLabelText("Token")).toHaveValue("tok-new");
    expect(screen.getByLabelText("Instructions")).toHaveValue("Only summarize.");
    expect(screen.getByLabelText("Name")).toHaveValue(dot.name);
  });

  it("keeps the editor and its unsaved edits when loading the dot again fails", async () => {
    const held = stored();
    backend.on("regenerate_webhook_token", () => {
      held.dot = { ...held.dot, webhook_token: "tok-new" };
      backend.on("get_dot", () => {
        throw { kind: "internal", message: "database is locked" };
      });
      return "tok-new";
    });
    const { user } = renderApp(path);
    await typeInstructions(user, "Only summarize.");
    await regenerate(user);

    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    expect(screen.getByLabelText("Instructions")).toHaveValue("Only summarize.");
    expect(screen.getByLabelText("Token")).toHaveValue("tok-new");
  });

  it("fills the form from the dot as it is now, not from a copy marked stale", async () => {
    const held = stored();
    const { user, router, queryClient } = renderApp(path);
    expect(await screen.findByLabelText("Name")).toHaveValue(dot.name);

    // Away from the editor, a rule is saved with an approval (in the Inbox): the dot's cached
    // copy is marked stale, and the core now holds the policy with the rule.
    await act(() => router.navigate("/inbox"));
    const rule = { tool: "Bash(git status:*)", action: "allow" as const };
    held.dot = {
      ...held.dot,
      policy: { ...held.dot.policy, preset: "custom", rules: [...held.dot.policy.rules, rule] },
    };
    act(() => invalidateDots(queryClient));

    await act(() => router.navigate(path));
    const added = `Rule ${dot.policy.rules.length + 1} tool`;
    expect(await screen.findByLabelText(added)).toHaveValue(rule.tool);
    // Saving keeps the rule.
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([{ id: dot.id, spec: specOf(held.dot) }]);
  });

  it("takes a new name to Chat's dot picker", async () => {
    const held = stored();
    backend.on("list_dots", () => [held.dot]);
    // Chat is opened first, so it holds the dot under its old name.
    const { user, router } = renderApp("/chat");
    expect(await screen.findByRole("option", { name: dot.name })).toBeInTheDocument();

    await act(() => router.navigate(path));
    const name = await screen.findByLabelText("Name");
    await user.clear(name);
    await user.type(name, "renamed");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();

    await user.click(screen.getByRole("link", { name: "Chat" }));
    expect(await screen.findByRole("option", { name: "renamed" })).toBeInTheDocument();
  });

  it("drops the deleted dot's runs from the Inbox and its count", async () => {
    stored();
    let waiting: Inbox = { approvals: [], review_runs: sample.inbox.review_runs, attention: [] };
    backend.on("inbox", () => waiting);
    backend.on("delete_dot", () => {
      waiting = { approvals: [], review_runs: [], attention: [] };
      return null;
    });
    const { user, router } = renderApp(path);
    const inboxLink = await screen.findByRole("link", { name: /^Inbox/ });
    await waitFor(() => expect(inboxLink).toHaveTextContent(/^Inbox1$/));

    await user.click(await screen.findByRole("button", { name: "Delete" }));
    await user.click(await screen.findByRole("button", { name: "Delete the dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots"));
    await waitFor(() => expect(inboxLink).toHaveTextContent(/^Inbox$/));
    await user.click(inboxLink);
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("says that webhooks are off instead of showing a URL nobody listens on", async () => {
    stored();
    backend.on("webhook_url", () => {
      throw {
        kind: "conflict",
        message: "webhooks are off because the local server is not running: port in use",
      };
    });
    renderApp(path);
    expect(await screen.findByText(/webhooks are off because the local server/)).toBeVisible();
    expect(screen.getByLabelText("Webhook URL")).toHaveValue("");
    expect(screen.queryByRole("button", { name: "Copy the webhook URL" })).toBeNull();
  });

  it("lists the dot's runs on their tab", async () => {
    stored();
    const failed = {
      ...sample.run,
      id: "run-2",
      status: "failed" as const,
      error: "timeout",
      queued_at: "2026-10-10T08:00:00.000Z",
    };
    backend.on("list_runs", () => [failed, sample.run]);
    const { user } = renderApp(path);
    await user.click(await screen.findByRole("tab", { name: "Runs" }));
    const link = await screen.findByRole("link", { name: localDateTime(failed.queued_at) });
    expect(link).toHaveAttribute("href", "/runs/run-2");
    expect(screen.getByText("timeout")).toBeVisible();
    expect(backend.callsTo("list_runs")).toEqual([{ dotId: dot.id, limit: 50 }]);
  });

  it("runs the dot now and opens the run", async () => {
    stored();
    backend.on("run_now", () => ({ ...sample.run, id: "run-new" }));
    const { user, router } = renderApp(path);
    await user.click(await screen.findByRole("button", { name: "Run now" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/runs/run-new"));
  });
});
