import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AppRow, BundledOffer, KeptData } from "@/api/types";
import { backend, later } from "@/test/backend";
import { renderApp } from "@/test/render";
import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";

const [projects, broken] = T.AppRow.array().parse(contract.AppRow) as [AppRow, AppRow];
const [kept] = T.KeptData.array().parse(contract.KeptData) as [KeptData];
const [onKept, fresh] = T.BundledOffer.array().parse(contract.BundledOffer) as [
  BundledOffer,
  BundledOffer,
];

/// Projects as Dots installs it: bundled, asking to start dots, the switch off and the prompt
/// not answered unless `asked`.
function bundled(asked = false): AppRow {
  if (projects.item.kind !== "app") throw new Error("an app row");
  return {
    ...projects,
    item: {
      ...projects.item,
      config: { ...projects.item.config, can_start_dots: false, start_asked: asked },
    },
  };
}

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
    await user.click(within(row).getByRole("button", { name: "Roll back Expenses" }));
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
    expect(within(row).queryByRole("button", { name: "Roll back Expenses" })).toBeNull();
  });

  it("removes an app after asking twice, keeping its data unless told", async () => {
    backend.on("apps_list", () => [expenses()]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_remove", () => null);
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Remove Expenses" }));
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

    await user.click(within(row).getByRole("button", { name: "Remove Expenses" }));
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

  it("removes an app no dot uses, which the core confirms only when told", async () => {
    backend.on("apps_list", () => [expenses({ dots: [] })]);
    backend.on("apps_kept_list", () => []);
    backend.on("app_remove", (args) => {
      if (args.confirm !== true) throw { kind: "conflict", message: "dots use this app" };
      return null;
    });
    const { user } = renderApp("/apps");
    const row = await screen.findByRole("listitem", { name: "Expenses" });
    await user.click(within(row).getByRole("button", { name: "Remove Expenses" }));
    const first = await screen.findByRole("alertdialog", { name: "Remove Expenses?" });
    expect(first).toHaveTextContent("No dot uses it.");
    await user.click(within(first).getByRole("button", { name: "Remove app" }));
    const second = await screen.findByRole("alertdialog", { name: "Delete Expenses's data too?" });
    await user.click(within(second).getByRole("button", { name: "Keep the data" }));
    await waitFor(() =>
      expect(backend.callsTo("app_remove")).toEqual([
        { name: "expenses", confirm: true, deleteData: false },
      ]),
    );
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

describe("Projects, which comes with Dots", () => {
  it("offers Install Projects on the kept data, which the person may untick", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => [kept]);
    backend.on("apps_bundled_offers", () => [onKept]);
    backend.on("app_install_bundled", () => projects.item);
    const { user } = renderApp("/apps");
    const section = await screen.findByRole("region", { name: "Comes with Dots" });
    await user.click(within(section).getByRole("button", { name: "Install Projects" }));
    const dialog = await screen.findByRole("alertdialog");
    const box = within(dialog).getByRole("checkbox", { name: /Start on the data kept/ });
    expect(box).toBeChecked();
    expect(dialog).toHaveTextContent("it then runs 0002_labels.sql on it");
    await user.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled")).toEqual([
        { name: "projects", keptId: onKept.kept!.id },
      ]),
    );
    await user.click(within(section).getByRole("button", { name: "Install Projects" }));
    const again = await screen.findByRole("alertdialog");
    await user.click(within(again).getByRole("checkbox", { name: /Start on the data kept/ }));
    await user.click(within(again).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled").at(-1)).toEqual({
        name: "projects",
        keptId: null,
      }),
    );
  });

  it("installs fresh when no kept data fits", async () => {
    backend.on("apps_list", () => []);
    backend.on("apps_kept_list", () => []);
    backend.on("apps_bundled_offers", () => [fresh]);
    backend.on("app_install_bundled", () => projects.item);
    const { user } = renderApp("/apps");
    await user.click(await screen.findByRole("button", { name: "Install Projects" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).queryByRole("checkbox")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Install" }));
    await waitFor(() =>
      expect(backend.callsTo("app_install_bundled")).toEqual([{ name: "projects", keptId: null }]),
    );
  });

  it("asks once above its screen to let it start dots", async () => {
    let row = bundled();
    backend.on("apps_list", () => [row]);
    backend.on("app_answer_start_prompt", () => {
      row = bundled(true);
      return row.item;
    });
    const { user } = renderApp("/apps/projects");
    const prompt = await screen.findByRole("region", { name: "Let Projects start dots?" });
    await user.click(within(prompt).getByRole("button", { name: "Not now" }));
    await waitFor(() =>
      expect(backend.callsTo("app_answer_start_prompt")).toEqual([{ name: "projects", on: false }]),
    );
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Let Projects start dots?" })).toBeNull(),
    );
  });

  it("switches starting dots on from the prompt", async () => {
    backend.on("apps_list", () => [bundled()]);
    backend.on("app_answer_start_prompt", () => projects.item);
    const { user } = renderApp("/apps/projects");
    const prompt = await screen.findByRole("region", { name: "Let Projects start dots?" });
    await user.click(within(prompt).getByRole("button", { name: "Switch on" }));
    await waitFor(() =>
      expect(backend.callsTo("app_answer_start_prompt")).toEqual([{ name: "projects", on: true }]),
    );
  });

  it("opens a run the page asks for in the dot's conversation", async () => {
    backend.on("apps_list", () => [bundled(true)]);
    backend.on("app_run_link", () => ({ dot_id: "d1", root_run_id: "r1" }));
    const { router } = renderApp("/apps/projects");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Projects");
    expect(screen.queryByRole("region", { name: "Let Projects start dots?" })).toBeNull();
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    port.postMessage({ type: "open_run", id: 1, run_id: "r9" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/d1/r1"));
    expect(backend.callsTo("app_run_link")).toEqual([{ name: "projects", runId: "r9" }]);
  });

  it("does not open a run once the person has left the app's screen", async () => {
    backend.on("apps_list", () => [bundled(true)]);
    backend.on("apps_kept_list", () => []);
    const link = later<{ dot_id: string; root_run_id: string }>();
    backend.on("app_run_link", () => link.answer);
    const { router } = renderApp("/apps/projects");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Projects");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    port.postMessage({ type: "open_run", id: 1, run_id: "r9" });
    await waitFor(() => expect(backend.callsTo("app_run_link")).toHaveLength(1));
    await act(() => router.navigate("/apps"));
    await screen.findByRole("heading", { name: "Apps", level: 1 });
    await act(async () => link.give({ dot_id: "d1", root_run_id: "r1" }));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(router.state.location.pathname).toBe("/apps");
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

  it("does not tell the frame of another app's data changes", async () => {
    backend.on("apps_list", () => [expenses()]);
    renderApp("/apps/expenses");
    const frame = await screen.findByTitle<HTMLIFrameElement>("Expenses");
    const post = vi.spyOn(frame.contentWindow!, "postMessage");
    fireEvent.load(frame);
    const port = (post.mock.calls[0] as unknown as [unknown, string, MessagePort[]])[2][0]!;
    const got: unknown[] = [];
    port.onmessage = (e: MessageEvent) => got.push(e.data);
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "projects" }));
    act(() => backend.emit("runtime", { type: "app_data_changed", app: "expenses" }));
    await waitFor(() => expect(got).toContainEqual({ type: "changed" }));
    expect(got.filter((m) => (m as { type: string }).type === "changed")).toHaveLength(1);
  });

  it("shows why an app whose code cannot be read does not open", async () => {
    backend.on("apps_list", () => [broken]);
    renderApp(`/apps/${broken.item.name}`);
    expect(
      await screen.findByText(/cannot open: invalid input: app.json: EOF while parsing an object/),
    ).toBeVisible();
    expect(document.querySelector("iframe")).toBeNull();
  });

  it("says so for an app it does not have", async () => {
    backend.on("apps_list", () => []);
    renderApp("/apps/nothing");
    expect(await screen.findByText("There is no app nothing.")).toBeVisible();
  });
});
