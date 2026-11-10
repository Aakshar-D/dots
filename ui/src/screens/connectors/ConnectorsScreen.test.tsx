import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ConnectorEntry } from "@/api/types";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import { fieldError } from "./fields";

const byId = (id: string) => sample.connectors.find((c) => c.id === id)!;
const personal = { ...sample.dot, id: "d1", name: "writer", org_id: null };
const orgDot = { ...sample.dot, id: "d2", name: "acme-bot", org_id: "acme" };

async function openPanel(name: string, action: RegExp) {
  const view = renderApp("/connectors");
  const card = await screen.findByRole("listitem", { name });
  await view.user.click(within(card).getByRole("button", { name: action }));
  const dialog = await screen.findByRole("dialog");
  return { ...view, dialog };
}

describe("the Connectors screen", () => {
  it("has a place in the rail between Teams and Settings", async () => {
    renderApp("/connectors");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const labels = within(nav)
      .getAllByRole("link")
      .map((l) => l.getAttribute("aria-label"));
    expect(labels).toEqual([
      "Home",
      "Inbox",
      "Intents",
      "Apps",
      "Library",
      "Teams",
      "Connectors",
      "Usage",
      "Settings",
    ]);
    expect(await screen.findByRole("heading", { name: "Connectors" })).toBeVisible();
  });

  it("lists every connector by what it is for, with its maker and its state in words", async () => {
    renderApp("/connectors");
    const email = await screen.findByRole("region", { name: "Email and chat" });
    const gmail = within(email).getByRole("listitem", { name: "Gmail" });
    expect(within(gmail).getByText("by codefuturist · community")).toBeVisible();
    expect(within(gmail).getByText("Connected")).toBeVisible();
    expect(within(gmail).getByText("Used by nightly-tests")).toBeVisible();
    const github = screen.getByRole("listitem", { name: "GitHub" });
    expect(within(github).getByText("by GitHub · official")).toBeVisible();
    expect(within(github).getByText("Error")).toBeVisible();
    expect(within(github).getByText("the server answered HTTP 401 Unauthorized")).toBeVisible();
    const notion = screen.getByRole("listitem", { name: "Notion" });
    expect(within(notion).getByText("Needs sign-in again")).toBeVisible();
    const calendar = screen.getByRole("listitem", { name: "Google Calendar" });
    expect(within(calendar).getByText("Coming soon")).toBeVisible();
    const sentry = screen.getByRole("listitem", { name: "Sentry" });
    expect(within(sentry).getByRole("button", { name: "Sign in Sentry" })).toBeVisible();
    // Advanced users add any server in the Library.
    expect(screen.getByRole("link", { name: "Add your own server" })).toHaveAttribute(
      "href",
      "/library",
    );
  });

  it("shows how to get a key, opens the right page, and connects with the dots picked", async () => {
    backend.on("list_dots", () => [personal, orgDot]);
    backend.on("connector_open_page", () => null);
    backend.on("connector_connect", () => ({ ...byId("huggingface"), state: "connected" }));
    const { user, dialog } = await openPanel("Hugging Face", /Connect/);
    const steps = within(dialog).getByRole("region", { name: "How to get your key" });
    expect(within(steps).getAllByRole("listitem").length).toBe(
      byId("huggingface").instructions.steps.length,
    );
    await user.click(within(dialog).getByRole("button", { name: "Open Hugging Face" }));
    expect(backend.callsTo("connector_open_page")).toEqual([
      { id: "huggingface", page: "instructions" },
    ]);

    const connect = within(dialog).getByRole("button", { name: "Connect" });
    expect(connect).toBeDisabled();
    const token = within(dialog).getByLabelText("Access token");
    expect(token).toHaveAttribute("type", "password");
    await user.type(token, "abc");
    expect(within(dialog).getByText("It should start with hf_.")).toBeVisible();
    expect(connect).toBeDisabled();
    await user.clear(token);
    await user.type(token, "hf_secret");
    // Only Personal dots can be picked.
    expect(within(dialog).queryByLabelText("acme-bot")).toBeNull();
    await user.click(within(dialog).getByLabelText("writer"));
    await user.click(connect);
    await waitFor(() =>
      expect(backend.callsTo("connector_connect")).toEqual([
        { id: "huggingface", values: { token: "hf_secret" }, dotIds: ["d1"] },
      ]),
    );
  });

  it("asks for Node.js before a connector that runs a program can connect", async () => {
    backend.on("connector_runtime", () => ({
      needed: byId("brave-search").runtime,
      found: null,
      ok: false,
    }));
    backend.on("connector_open_page", () => null);
    const { user, dialog } = await openPanel("Brave Search", /Connect/);
    expect(await within(dialog).findByText(/Node\.js was not found/)).toBeVisible();
    await user.type(within(dialog).getByLabelText("API key"), "BSAkey");
    expect(within(dialog).getByRole("button", { name: "Connect" })).toBeDisabled();
    await user.click(within(dialog).getByRole("button", { name: "Get Node.js" }));
    expect(backend.callsTo("connector_open_page")).toEqual([
      { id: "brave-search", page: "runtime" },
    ]);
  });

  it("names a community maker and says its program gets the key", async () => {
    const { dialog } = await openPanel("Slack", /Manage/);
    expect(within(dialog).getByText("Made by korotovsky · community")).toBeVisible();
    expect(within(dialog).getByText(/Made by korotovsky, not Slack/)).toBeVisible();
    // Its name is taken by a server of the person's own: the Library is where to fix it.
    expect(within(dialog).getByRole("link", { name: "Open the Library" })).toBeVisible();
    expect(within(dialog).queryByRole("button", { name: "Connect" })).toBeNull();
  });

  it("signs in with the browser and can cancel while it waits", async () => {
    backend.on("list_dots", () => [personal]);
    const signIn = later<ConnectorEntry>();
    backend.on("connector_sign_in", () => signIn.answer);
    backend.on("connector_sign_in_cancel", () => null);
    const { user, dialog } = await openPanel("Sentry", /Sign in/);
    expect(within(dialog).getByRole("region", { name: "How signing in works" })).toBeVisible();
    await user.click(within(dialog).getByLabelText("writer"));
    await user.click(within(dialog).getByRole("button", { name: "Sign in with Sentry" }));
    expect(await within(dialog).findByText("Finish signing in in your browser…")).toBeVisible();
    expect(backend.callsTo("connector_sign_in")).toEqual([{ id: "sentry", dotIds: ["d1"] }]);
    await user.click(within(dialog).getByRole("button", { name: "Cancel sign-in" }));
    expect(backend.callsTo("connector_sign_in_cancel")).toEqual([{ id: "sentry" }]);
    signIn.fail({ kind: "cancelled", message: "Sign-in cancelled." });
    expect(
      await within(dialog).findByRole("button", { name: "Sign in with Sentry" }),
    ).toBeVisible();
  });

  it("tests, moves and disconnects a connected connector", async () => {
    backend.on("list_dots", () => [personal, { ...personal, id: "01J0DOT", name: "nightly-tests" }]);
    backend.on("library_check", () => sample.library[0]!.catalog);
    backend.on("connector_set_dots", () => byId("linear"));
    backend.on("connector_disconnect", () => ({ ...byId("linear"), state: "not_connected" }));
    const { user, dialog } = await openPanel("Linear", /Manage/);
    expect(within(dialog).getByText(/12 tools available\. Used by nightly-tests\./)).toBeVisible();

    await user.click(within(dialog).getByRole("button", { name: "Test" }));
    await waitFor(() =>
      expect(backend.callsTo("library_check")).toEqual([
        { id: byId("linear").server_id, scope: undefined },
      ]),
    );

    const save = within(dialog).getByRole("button", { name: "Save dots" });
    expect(save).toBeDisabled();
    await user.click(within(dialog).getByLabelText("writer"));
    await user.click(save);
    await waitFor(() =>
      expect(backend.callsTo("connector_set_dots")).toEqual([
        { id: "linear", dotIds: ["01J0DOT", "d1"] },
      ]),
    );

    await user.click(within(dialog).getByRole("button", { name: "Disconnect" }));
    const confirm = await screen.findByRole("alertdialog");
    expect(within(confirm).getByText(/nightly-tests will stop using it/)).toBeVisible();
    await user.click(within(confirm).getByRole("button", { name: "Disconnect" }));
    await waitFor(() =>
      expect(backend.callsTo("connector_disconnect")).toEqual([{ id: "linear" }]),
    );
  });

  it("explains a connector that is not available yet", async () => {
    const { dialog } = await openPanel("Google Calendar", /Details/);
    expect(within(dialog).getByRole("region", { name: "Why it is not available" })).toBeVisible();
    expect(within(dialog).queryByRole("button", { name: /Connect|Sign in/ })).toBeNull();
  });

  it("reaches the screen from Settings", async () => {
    const { user } = renderApp("/settings");
    await user.click(await screen.findByRole("link", { name: "Open Connectors" }));
    expect(await screen.findByRole("heading", { name: "Connectors" })).toBeVisible();
  });
});

describe("a connector field", () => {
  const gmail = byId("gmail");
  const [address, password] = gmail.fields as [ConnectorEntry["fields"][0], ConnectorEntry["fields"][0]];

  it("is checked as the core checks it", () => {
    expect(fieldError(address, "me@gmail.com")).toBeNull();
    expect(fieldError(address, "me")).toBe("Enter an email address.");
    expect(fieldError(password, "abcd efgh ijkl mnop")).toBeNull();
    expect(fieldError(password, "abcd efgh")).toBe("It should be 16 letters.");
    expect(fieldError(password, "   ")).toBe("Enter a value.");
  });
});
