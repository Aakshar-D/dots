import { screen, waitFor, within } from "@testing-library/react";
import type { UserEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import type { Dot, EnvEntry, Watcher } from "@/api/types";
import { localDateTime } from "@/lib/format";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [folder, email, github] = sample.watchers as [Watcher, Watcher, Watcher];
const TS = "2026-10-09T12:00:00.000Z";
const entry = (name: string, kind: "secret" | "plain"): EnvEntry => ({
  name,
  kind,
  value: kind === "plain" ? "debug" : null,
  created_at: TS,
  updated_at: TS,
});
const INVOICES = "C:\\Users\\u\\Downloads\\Invoices";

/// A backend holding the dot, its watchers and a vault with one secret and one plain entry; a
/// vault entry added is kept, as the core would.
function holding(watchers: Watcher[], dot: Dot = sample.dot) {
  const vault = [entry("GITHUB_TOKEN", "secret"), entry("LOG_LEVEL", "plain")];
  backend.on("get_dot", () => dot);
  backend.on("watchers_list", (args) => (args.dotId === dot.id ? watchers : []));
  backend.on("env_list", () => vault);
  backend.on("env_add", (args) => {
    const added = entry(args.name as string, "secret");
    vault.push(added);
    return added;
  });
  backend.on("dialog_open", () => INVOICES);
}

/// The editor of the dot, opened on its Watchers tab.
async function openTab() {
  const view = renderApp(`/dots/${sample.dot.id}?tab=watchers`);
  expect(await screen.findByRole("tab", { name: "Watchers", selected: true })).toBeVisible();
  return view;
}

const items = () => within(screen.getByRole("list", { name: "Watchers" })).getAllByRole("listitem");

/// Opens the form for a new watcher of `kind` and returns it.
async function add(user: UserEvent, kind: string, title: string) {
  await user.click(await screen.findByRole("button", { name: "Add watcher" }));
  await user.click(await screen.findByRole("menuitem", { name: kind }));
  return screen.findByRole("dialog", { name: title });
}

describe("a dot's Watchers tab", () => {
  it("lists each watcher with its summary, last event and state, after Memory", async () => {
    holding([folder, email, github]);
    await openTab();
    await screen.findByRole("list", { name: "Watchers" });
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "Settings", "Runs", "Follow-ups", "Memory", "Watchers",
    ]);
    const [f, e, g] = items() as [HTMLElement, HTMLElement, HTMLElement];
    expect(f).toHaveAccessibleName("Invoices folder");
    expect(within(f).getByText(`*.pdf in ${INVOICES}, with subfolders`)).toBeVisible();
    expect(within(f).getByText(`Last event ${localDateTime(folder.last_event_at)}`)).toBeVisible();
    expect(within(f).getByText("on")).toBeVisible();
    expect(within(f).getByRole("switch", { name: "Invoices folder is on" })).toBeChecked();
    expect(within(e).getByText("No event yet")).toBeVisible();
    expect(
      within(e).getByText("paused: Gmail refused the app password for me@gmail.com, or IMAP is off for it."),
    ).toBeVisible();
    expect(within(e).getByRole("switch", { name: "Billing mail is on" })).not.toBeChecked();
    expect(within(g).getByText("paused: too many runs")).toBeVisible();
  });

  it("says what a watcher is for when the dot has none", async () => {
    holding([]);
    await openTab();
    expect(
      await screen.findByText(
        "No watchers. A watcher starts this dot when a file changes, an email arrives or something happens on GitHub.",
      ),
    ).toBeVisible();
  });

  it("adds a folder watcher with the folder picker, named after the folder", async () => {
    holding([]);
    backend.on("watcher_add", () => folder);
    const { user } = await openTab();
    const dialog = await add(user, "Folder", "New folder watcher");
    await user.click(within(dialog).getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(within(dialog).getByLabelText("Folder")).toHaveValue(INVOICES));
    expect(within(dialog).getByLabelText("Name")).toHaveValue("Invoices folder");
    for (const name of ["Created", "Changed", "Deleted"]) {
      expect(within(dialog).getByRole("checkbox", { name })).toBeChecked();
    }
    expect(within(dialog).getByRole("switch", { name: "Subfolders" })).toBeChecked();
    await user.type(within(dialog).getByLabelText("Pattern"), "*.pdf");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_add")).toEqual([
        {
          dotId: sample.dot.id,
          name: "Invoices folder",
          config: { kind: "folder", path: INVOICES, pattern: "*.pdf", events: ["created", "changed", "deleted"], recursive: true },
        },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("notes a folder the dot cannot open files in", async () => {
    holding([], { ...sample.dot, workdir: "C:\\work", workspace_mode: "folder" });
    const { user } = await openTab();
    const dialog = await add(user, "Folder", "New folder watcher");
    await user.click(within(dialog).getByRole("button", { name: "Browse…" }));
    const note = "The dot can only open files inside its folder (C:\\work); this folder is outside it.";
    expect(await within(dialog).findByText(note)).toBeVisible();
    const path = within(dialog).getByLabelText("Folder");
    await user.clear(path);
    await user.type(path, "C:\\work\\in");
    expect(within(dialog).queryByText(note)).toBeNull();
  });

  it("adds an email watcher with a secret from the vault, or one added there", async () => {
    holding([]);
    backend.on("watcher_add", () => email);
    const { user } = await openTab();
    const dialog = await add(user, "Email", "New email watcher");
    expect(
      within(dialog).getByText("Gmail needs an app password, which your Google account offers once 2-step verification is on."),
    ).toBeVisible();
    expect(within(dialog).getByRole("link", { name: "Make an app password" })).toHaveAttribute(
      "href",
      "https://myaccount.google.com/apppasswords",
    );
    const chooser = within(dialog).getByLabelText("App password");
    await waitFor(() =>
      expect(within(chooser).getAllByRole("option").map((o) => o.textContent)).toEqual([
        "Choose a vault entry", "GITHUB_TOKEN", "Add to vault…",
      ]),
    );
    await user.type(within(dialog).getByLabelText("Gmail address"), "me@gmail.com");
    expect(within(dialog).getByLabelText("Gmail label")).toHaveValue("INBOX");
    await user.selectOptions(chooser, "Add to vault…");
    const vault = await screen.findByRole("dialog", { name: "Add variable" });
    expect(within(vault).getByLabelText("Name")).toHaveValue("GMAIL_APP_PASSWORD");
    await user.type(within(vault).getByLabelText("Value"), "app-pass-1234567890");
    await user.click(within(vault).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(chooser).toHaveValue("GMAIL_APP_PASSWORD"));
    expect(backend.callsTo("env_add")).toEqual([
      { name: "GMAIL_APP_PASSWORD", kind: "secret", value: "app-pass-1234567890" },
    ]);
    await user.type(within(dialog).getByLabelText("From"), "billing@acme.com");
    await user.type(within(dialog).getByLabelText("Subject contains"), "invoice");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_add")).toEqual([
        {
          dotId: sample.dot.id,
          name: "me@gmail.com mail",
          config: {
            kind: "email", address: "me@gmail.com", secret: "GMAIL_APP_PASSWORD", label: "INBOX",
            from: "billing@acme.com", subject: "invoice", has_attachment: null,
          },
        },
      ]),
    );
  });

  it("adds a GitHub watcher for the repositories and events chosen", async () => {
    holding([]);
    backend.on("watcher_add", () => github);
    const { user } = await openTab();
    const dialog = await add(user, "GitHub", "New GitHub watcher");
    await user.type(within(dialog).getByLabelText("Repositories"), "owner/repo{Enter}owner/other");
    expect(within(dialog).getByRole("checkbox", { name: "Issues" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "Pull requests" })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: "CI runs" })).not.toBeChecked();
    await user.click(within(dialog).getByRole("checkbox", { name: "Comments" }));
    await waitFor(() => expect(within(dialog).getAllByRole("option", { name: "GITHUB_TOKEN" })).toHaveLength(1));
    await user.selectOptions(within(dialog).getByLabelText("Token"), "GITHUB_TOKEN");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_add")).toEqual([
        {
          dotId: sample.dot.id,
          name: "owner/repo",
          config: {
            kind: "github", repos: ["owner/repo", "owner/other"], events: ["issue", "pull_request", "comment"],
            secret: "GITHUB_TOKEN", label: null, authors: [], skip_bots: true, branches: [],
          },
        },
      ]),
    );
  });

  it("edits a watcher in the same form", async () => {
    holding([folder]);
    backend.on("watcher_update", () => folder);
    const { user } = await openTab();
    await screen.findByRole("list", { name: "Watchers" });
    await user.click(within(items()[0]!).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit Invoices folder" });
    const pattern = within(dialog).getByLabelText("Pattern");
    expect(pattern).toHaveValue("*.pdf");
    await user.clear(pattern);
    await user.type(pattern, "invoice-*.pdf");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_update")).toEqual([
        { id: folder.id, name: "Invoices folder", config: { ...folder.config, pattern: "invoice-*.pdf" } },
      ]),
    );
  });

  it("shows the core's refusal in the form and keeps it open", async () => {
    holding([]);
    backend.on("watcher_add", () => {
      throw { kind: "invalid", message: "No folder at C:\\gone." };
    });
    const { user } = await openTab();
    const dialog = await add(user, "Folder", "New folder watcher");
    await user.type(within(dialog).getByLabelText("Folder"), "C:\\gone");
    expect(within(dialog).getByLabelText("Name")).toHaveValue("gone folder");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByText("No folder at C:\\gone.")).toBeVisible();
    expect(screen.getByRole("dialog", { name: "New folder watcher" })).toBeVisible();
  });

  it("switches a watcher off, on, and resumes one Dots paused", async () => {
    holding([folder, email, { ...github, paused_reason: null, enabled: false }]);
    backend.on("watcher_set_enabled", () => folder);
    backend.on("watcher_resume", () => email);
    const { user } = await openTab();
    await screen.findByRole("list", { name: "Watchers" });
    await user.click(screen.getByRole("switch", { name: "Invoices folder is on" }));
    await user.click(screen.getByRole("switch", { name: "Billing mail is on" }));
    await user.click(screen.getByRole("switch", { name: "Repo events is on" }));
    await waitFor(() =>
      expect(backend.callsTo("watcher_set_enabled")).toEqual([
        { id: folder.id, enabled: false },
        { id: github.id, enabled: true },
      ]),
    );
    expect(backend.callsTo("watcher_resume")).toEqual([{ id: email.id }]);
  });

  it("deletes a watcher after asking", async () => {
    holding([folder]);
    backend.on("watcher_delete", () => null);
    const { user } = await openTab();
    await screen.findByRole("list", { name: "Watchers" });
    await user.click(within(items()[0]!).getByRole("button", { name: "Delete" }));
    const ask = await screen.findByRole("alertdialog", { name: "Delete the watcher Invoices folder?" });
    await user.click(within(ask).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(backend.callsTo("watcher_delete")).toEqual([{ id: folder.id }]));
  });
});
