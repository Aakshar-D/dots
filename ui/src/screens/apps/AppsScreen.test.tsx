import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AppRow, KeptData } from "@/api/types";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";
import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";

const [projects, broken] = T.AppRow.array().parse(contract.AppRow) as [AppRow, AppRow];
const [kept] = T.KeptData.array().parse(contract.KeptData) as [KeptData];

/// The Expenses app as a row: used by one dot, version 3, with a version to roll back to.
function expenses(over: Partial<{ every_dot: boolean; previous: number | null; dots: string[] }> = {}): AppRow {
  if (projects.item.kind !== "app") throw new Error("an app row");
  return {
    ...projects,
    item: {
      ...projects.item,
      id: "01J0APP9",
      name: "expenses",
      config: {
        ...projects.item.config,
        title: "Expenses",
        description: "Logs expenses and totals them by month",
        version: 3,
        previous_version: over.previous === undefined ? 2 : over.previous,
        every_dot: over.every_dot ?? false,
      },
    },
    dots: over.dots ?? ["builder"],
    permissions: [],
    tools: [{ name: "list_expenses", description: "List", writes: false }],
  };
}

describe("the Apps screen", () => {
  it("sits on the rail between Inbox and Library", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => []);
    renderApp("/apps");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const names = within(nav)
      .getAllByRole("link")
      .map((link) => link.getAttribute("aria-label"));
    expect(names.indexOf("Apps")).toBe(names.indexOf("Inbox") + 1);
    expect(names.indexOf("Library")).toBe(names.indexOf("Apps") + 1);
    expect(await screen.findByText(/No apps yet/)).toBeVisible();
  });

  it("lists each app with its version, its users, its error and its switches", async () => {
    backend.on("apps_list", () => [projects, broken, expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_set_every_dot", () => projects.item);
    backend.on("app_set_can_start_dots", () => projects.item);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    expect(row).toHaveTextContent("version 3");
    expect(row).toHaveTextContent("Used by builder");
    expect(within(row).getByRole("link", { name: "Expenses" })).toHaveAttribute(
      "href",
      "/apps/expenses",
    );
    // Expenses does not ask to start dots: no switch for it.
    expect(within(row).queryByRole("switch", { name: "Can start dots" })).toBeNull();
    await user.click(within(row).getByRole("switch", { name: "Every dot" }));
    await waitFor(() =>
      expect(backend.callsTo("app_set_every_dot")).toEqual([{ name: "expenses", on: true }]),
    );
    // Projects asks to start dots, and the sample has it switched on: the click switches it off.
    const asking = screen.getAllByRole("listitem")[0]!;
    await user.click(within(asking).getByRole("switch", { name: "Can start dots" }));
    await waitFor(() =>
      expect(backend.callsTo("app_set_can_start_dots")).toEqual([
        { name: projects.item.name, on: false },
      ]),
    );
    const broke = screen.getAllByRole("listitem")[1]!;
    expect(broke).toHaveTextContent("EOF while parsing an object");
  });

  it("rolls back after asking", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_roll_back", () => expenses().item);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Roll back" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Roll back Expenses?" });
    expect(dialog).toHaveTextContent("Version 2 becomes the active one again.");
    await user.click(within(dialog).getByRole("button", { name: "Roll back" }));
    await waitFor(() => expect(backend.callsTo("app_roll_back")).toEqual([{ name: "expenses" }]));
  });

  it("offers no roll back once an update changed the data's schema", async () => {
    backend.on("apps_list", () => [expenses({ previous: null })]);
    backend.on("apps_kept_list", () => []);
    renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    expect(within(row).queryByRole("button", { name: "Roll back" })).toBeNull();
  });

  it("removes an app after asking twice, keeping its data unless told", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_remove", () => null);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const first = await screen.findByRole("alertdialog", { name: "Remove Expenses?" });
    expect(first).toHaveTextContent("Used by builder. Removing it takes it out of them.");
    await user.click(within(first).getByRole("button", { name: "Remove app" }));
    const second = await screen.findByRole("alertdialog", { name: "Delete Expenses's data too?" });
    await user.click(within(second).getByRole("button", { name: "Keep the data" }));
    await waitFor(() =>
      expect(backend.callsTo("app_remove")).toEqual([
        { name: "expenses", confirm: true, deleteData: false },
      ]),
    );

    await user.click(within(row).getByRole("button", { name: "Remove" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Remove app" }),
    );
    await user.click(
      within(await screen.findByRole("alertdialog", { name: "Delete Expenses's data too?" }))
        .getByRole("button", { name: "Delete the data" }),
    );
    await waitFor(() => expect(backend.callsTo("app_remove")).toHaveLength(2));
    expect(backend.callsTo("app_remove")[1]).toEqual({
      name: "expenses",
      confirm: true,
      deleteData: true,
    });
  });

  it("lists the data removed apps kept, and deletes it after asking", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => [kept]);
    backend.on("app_kept_delete", () => null);
    const { user } = renderApp("/apps");
    const section = await screen.findByRole("region", { name: "Kept data" });
    const item = within(section).getByRole("listitem", { name: "Expenses, kept data" });
    expect(item).toHaveTextContent("24 KB");
    await user.click(within(item).getByRole("button", { name: "Delete data" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete data" }),
    );
    await waitFor(() => expect(backend.callsTo("app_kept_delete")).toEqual([{ id: kept.id }]));
  });
});

describe("an app's own screen", () => {
  it("fills the content area with the app's frame and tells it of changes", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("app_call", () => [{ item: "Ink" }]);
    renderApp("/apps/expenses");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses");
    expect(frame).toHaveAttribute("src", "http://dots-app.localhost/app/expenses/3/index.html");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    port.postMessage({ type: "call", id: 1, tool: "list_expenses", input: { month: "2026-10" } });
    await waitFor(() =>
      expect(got).toContainEqual({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] }),
    );
    expect(backend.callsTo("app_call")).toEqual([
      { name: "expenses", tool: "list_expenses", input: { month: "2026-10" } },
    ]);
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "expenses" }));
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
  });

  it("says so for an app it does not have", async () => {
    backend.on("apps_list", () => []);
    renderApp("/apps/nothing");
    expect(await screen.findByText("There is no app nothing.")).toBeVisible();
  });
});
