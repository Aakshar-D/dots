import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { specOf } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { blankSpec, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
const path = `/dots/${dot.id}`;

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
    expect(backend.callsTo("preview_schedule").at(-1)).toEqual({ expr: "0 0 2 * * *", count: 3 });

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

  it("refuses MCP servers that are not a JSON object", async () => {
    const { user } = renderApp("/dots/new");
    // user-event reads "{{" as one typed "{".
    await user.type(await screen.findByLabelText("MCP servers"), "{{nope");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    expect(screen.getByText("This is not valid JSON.")).toBeVisible();
  });
});

describe("the dot editor, stored dot", () => {
  function stored() {
    let current = dot;
    backend.on("get_dot", () => current);
    backend.on("update_dot", (args) => {
      current = { ...current, ...(args.spec as object) };
      return current;
    });
  }

  it("loads the dot and saves a change", async () => {
    stored();
    const { user } = renderApp(path);
    expect(await screen.findByRole("heading", { name: dot.name })).toBeVisible();
    expect(screen.getByLabelText("Name")).toHaveValue(dot.name);
    expect(screen.getByLabelText("Cron expression")).toHaveValue(dot.schedule);
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
    stored();
    backend.on("regenerate_webhook_token", () => "tok-new");
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
