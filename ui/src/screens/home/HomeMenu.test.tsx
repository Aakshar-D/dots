import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
import { dot, home, overview, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

/// Opens the menu under the dot's name in the header.
async function openMenu(user: ReturnType<typeof renderApp>["user"]) {
  await user.click(await screen.findByRole("button", { name: dot.name }));
  return screen.findByRole("menu");
}

/// A backend holding the dot, for a walk between the home and the dot's editor.
function oneDot() {
  const held = { dot: dot as Dot };
  home([]);
  backend.on("dot_overview", () => [{ ...overview, dot: held.dot }]);
  backend.on("get_dot", () => held.dot);
  backend.on("update_dot", (args) => {
    held.dot = { ...held.dot, ...(args.spec as object) };
    return held.dot;
  });
  return held;
}

describe("the dot's menu", () => {
  it("opens from the dot's name with Edit, Memory…, Run now and Enabled", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("button", { name: dot.name });
    // The name is the header's heading, and the menu's only way in: there is no ⚙.
    expect(screen.getByRole("heading", { level: 1, name: dot.name })).toContainElement(name);
    expect(name).toHaveAttribute("title", "Dot settings");
    expect(screen.queryByRole("button", { name: "Dot settings" })).toBeNull();
    await user.click(name);
    const menu = await screen.findByRole("menu");
    const items = [...menu.querySelectorAll('[role="menuitem"], [role="menuitemcheckbox"]')];
    expect(items.map((item) => item.textContent)).toEqual([
      "Edit",
      "Memory…",
      "Run now",
      "Enabled",
    ]);
  });

  it("opens from the keyboard", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    const name = await screen.findByRole("button", { name: dot.name });
    act(() => name.focus());
    await user.keyboard("{Enter}");
    expect(
      within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Edit" }),
    ).toBeVisible();
  });

  it("opens the dot's editor", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    const { user, router } = renderApp(`/home/${dot.id}`);
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Edit" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/dots/${dot.id}`));
  });

  it("runs the dot now, and the run shows up in the conversation", async () => {
    const runs = home([]);
    backend.on("run_now", () => {
      const run = scheduled("manual", "12:00", {
        trigger: "manual",
        status: "queued",
        summary: null,
      });
      runs.push(run);
      return run;
    });
    const { user, router } = renderApp(`/home/${dot.id}`);
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Run now" }));
    await waitFor(() => expect(backend.callsTo("run_now")).toEqual([{ dotId: dot.id }]));
    const at = localTime("2026-10-09T12:00:00.000Z");
    expect(await screen.findByRole("article", { name: `Run now · ${at}` })).toBeVisible();
    expect(router.state.location.pathname).toBe(`/home/${dot.id}`);
  });

  it("shows why a run could not start", async () => {
    home([]);
    backend.on("run_now", () => {
      throw { kind: "conflict", message: "the dot is disabled" };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitem", { name: "Run now" }));
    expect(await screen.findByText("the dot is disabled")).toBeVisible();
  });

  it("turns the dot off and shows the stored state", async () => {
    const held = oneDot();
    backend.on("set_dot_enabled", (args) => {
      held.dot = { ...held.dot, enabled: args.enabled as boolean };
      return held.dot;
    });
    const { user } = renderApp(`/home/${dot.id}`);
    let menu = await openMenu(user);
    expect(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" })).toBeChecked();
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" }));
    await waitFor(() =>
      expect(backend.callsTo("set_dot_enabled")).toEqual([{ id: dot.id, enabled: false }]),
    );
    const header = screen.getByRole("heading", { level: 1, name: dot.name }).closest("header")!;
    expect(await within(header).findByRole("img", { name: "off" })).toBeVisible();
    menu = await openMenu(user);
    expect(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" })).not.toBeChecked();
  });

  it("hands the dot it switched off to the editor, which then saves it switched off", async () => {
    const held = oneDot();
    backend.on("set_dot_enabled", (args) => {
      held.dot = { ...held.dot, enabled: args.enabled as boolean };
      return held.dot;
    });
    // The editor loads the dot while it is enabled.
    const { user, router } = renderApp(`/dots/${dot.id}`);
    expect(await screen.findByLabelText("Enabled")).toBeChecked();

    await act(() => router.navigate(`/home/${dot.id}`));
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" }));
    await waitFor(() => expect(backend.callsTo("set_dot_enabled")).toHaveLength(1));
    const reopened = await openMenu(user);
    await user.click(within(reopened).getByRole("menuitem", { name: "Edit" }));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      id: dot.id,
      spec: { enabled: false },
    });
  });

  it("makes the editor load the dot again after a switch whose answer was lost", async () => {
    const held = oneDot();
    // The dot is switched off, but the answer is not one the UI can read.
    backend.on("set_dot_enabled", (args) => {
      held.dot = { ...held.dot, enabled: args.enabled as boolean };
      return { id: held.dot.id };
    });
    const { user, router } = renderApp(`/dots/${dot.id}`);
    expect(await screen.findByLabelText("Enabled")).toBeChecked();

    await act(() => router.navigate(`/home/${dot.id}`));
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" }));
    expect(await screen.findByText(/^set_dot_enabled: unexpected/)).toBeVisible();

    await act(() => router.navigate(`/dots/${dot.id}`));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
  });

  it("fills an editor opened during a switch from the switch's answer", async () => {
    const held = oneDot();
    const answer = later<Dot>();
    backend.on("set_dot_enabled", () => answer.answer);
    const { user, router } = renderApp(`/home/${dot.id}`);
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" }));
    await waitFor(() => expect(backend.callsTo("set_dot_enabled")).toHaveLength(1));

    // The command is on its way to the core, which still holds the enabled dot: a copy loaded
    // now would be the old one, so the editor waits for the command.
    await act(() => router.navigate(`/dots/${dot.id}`));
    await screen.findByRole("heading", { name: "Dot" });
    await act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));
    expect(backend.callsTo("get_dot")).toEqual([]);
    expect(screen.queryByLabelText("Enabled")).toBeNull();

    held.dot = { ...held.dot, enabled: false };
    await act(async () => answer.give(held.dot));
    expect(await screen.findByLabelText("Enabled")).not.toBeChecked();
  });

  it("loads the dot for an editor opened during a switch that then fails", async () => {
    oneDot();
    const answer = later<Dot>();
    backend.on("set_dot_enabled", () => answer.answer);
    const { user, router } = renderApp(`/home/${dot.id}`);
    const menu = await openMenu(user);
    await user.click(within(menu).getByRole("menuitemcheckbox", { name: "Enabled" }));
    await waitFor(() => expect(backend.callsTo("set_dot_enabled")).toHaveLength(1));
    await act(() => router.navigate(`/dots/${dot.id}`));
    await screen.findByRole("heading", { name: "Dot" });

    await act(async () => answer.fail({ kind: "internal", message: "database is locked" }));
    expect(await screen.findByLabelText("Enabled")).toBeChecked();
    expect(screen.getByText("database is locked")).toBeVisible();
  });
});
