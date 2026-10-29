import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, OrgSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { blankSpec, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs as [OrgSummary]; // Acme Engineering, connected
const gamma: OrgSummary = { ...acme, id: "gamma", name: "Gamma" };
// A dot of Acme Engineering, and the contract's Personal dot.
const release: Dot = { ...sample.dot, id: "D2", name: "acme-release", org_id: "acme-eng" };
const personal = sample.dot;
const REMOVED = "Its account 'acme-eng' was removed. Join it again to run this dot.";
const basics = { name: "nightly", instructions: "Run the tests.", workdir: "C:/dev/app" };

/// Opens the new-dot form with `orgs` joined and fills in what a dot needs.
async function newDot(orgs: OrgSummary[]) {
  backend.on("org_list", () => orgs);
  backend.on("create_dot", (args) => ({
    ...sample.dot,
    ...(args.spec as object),
    id: "dot-new",
    org_id: (args.orgId as string | null) ?? null,
  }));
  const rendered = renderApp("/dots/new");
  const { user } = rendered;
  await user.type(await screen.findByLabelText("Name"), basics.name);
  await user.type(screen.getByLabelText("Instructions"), basics.instructions);
  await user.type(screen.getByLabelText("Folder"), basics.workdir);
  return rendered;
}

/// The editor of `dot`, with `orgs` joined.
function editor(dot: Dot, orgs: OrgSummary[]) {
  const held = { dot };
  backend.on("org_list", () => orgs);
  backend.on("get_dot", () => held.dot);
  backend.on("update_dot", (args) => {
    held.dot = { ...held.dot, ...(args.spec as object) };
    return held.dot;
  });
  backend.on("set_dot_enabled", (args) => {
    held.dot = { ...held.dot, enabled: args.enabled as boolean };
    return held.dot;
  });
  backend.on("delete_dot", () => null);
  return { held, ...renderApp(`/dots/${dot.id}`) };
}

describe("the org of a new dot", () => {
  it("offers Personal and each joined org, and creates the dot in the one chosen", async () => {
    const { user, router } = await newDot([acme, gamma]);
    const picker = screen.getByLabelText("Account");
    expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Personal",
      "Acme Engineering",
      "Gamma",
    ]);
    expect(picker).toHaveValue("");
    await user.selectOptions(picker, "Acme Engineering");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics }, orgId: "acme-eng", workspaceId: null },
    ]);
  });

  it("makes a Personal dot with no org", async () => {
    const { user, router } = await newDot([acme]);
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics }, orgId: null, workspaceId: null },
    ]);
  });

  it("starts in the current account", async () => {
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    const { user, router } = await newDot([acme, gamma]);
    await waitFor(() => expect(screen.getByLabelText("Account")).toHaveValue("acme-eng"));
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics }, orgId: "acme-eng", workspaceId: null },
    ]);
  });

  it("offers no picker when no org is joined", async () => {
    await newDot([]);
    await waitFor(() => expect(backend.callsTo("org_list").length).toBeGreaterThan(0));
    expect(screen.queryByLabelText("Account")).toBeNull();
  });

  it("shows the org as text once the dot is made, with no picker", async () => {
    const { user } = await newDot([acme]);
    await user.selectOptions(screen.getByLabelText("Account"), "Acme Engineering");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    const org = await screen.findByRole("group", { name: "Account" });
    await waitFor(() => expect(org).toHaveTextContent("Account: Acme Engineering"));
    expect(screen.queryByRole("combobox", { name: "Account" })).toBeNull();
  });
});

describe("a dot whose org was removed", () => {
  it("says so in the editor, and keeps Save and Run off with the reason", async () => {
    editor(release, []);
    const org = await screen.findByRole("group", { name: "Account" });
    expect(await within(org).findByText("account removed")).toBeVisible();
    expect(org).toHaveTextContent("Account: acme-eng");
    expect(within(org).getByText(REMOVED)).toHaveClass("text-destructive");
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    expect(save).toHaveAttribute("title", REMOVED);
    const run = screen.getByRole("button", { name: "Run now" });
    expect(run).toBeDisabled();
    expect(run).toHaveAttribute("title", REMOVED);
  });

  it("can still be switched off at once, and deleted", async () => {
    const { user, router } = editor({ ...release, enabled: true }, []);
    await screen.findByRole("group", { name: "Account" });
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeDisabled());
    await user.click(screen.getByLabelText("Enabled"));
    await waitFor(() =>
      expect(backend.callsTo("set_dot_enabled")).toEqual([{ id: "D2", enabled: false }]),
    );
    expect(screen.getByLabelText("Enabled")).not.toBeChecked();
    expect(backend.callsTo("update_dot")).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.click(await screen.findByRole("button", { name: "Delete the dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/home"));
    expect(backend.callsTo("delete_dot")).toEqual([{ id: "D2" }]);
  });

  it("shows the core's refusal when the org went away after the list was read", async () => {
    const { user } = editor(release, [acme]);
    const org = await screen.findByRole("group", { name: "Account" });
    await waitFor(() => expect(org).toHaveTextContent("Account: Acme Engineering"));
    backend.on("update_dot", () => {
      throw { kind: "conflict", message: "dot 'acme-release': its org 'acme-eng' was removed" };
    });
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "dot 'acme-release': its org 'acme-eng' was removed",
    );
  });

  it("is marked on its card at home, and can't be run from there", async () => {
    // The core can only be in Personal once the org is gone; its dots are listed there.
    backend.on("org_list", () => []);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    backend.on("dot_overview", () => [
      { ...sample.overview, dot: release },
      { ...sample.overview, dot: personal },
    ]);
    backend.on("set_dot_enabled", (args) => ({ ...release, enabled: args.enabled as boolean }));
    const { user } = renderApp(`/home/${release.id}`);
    const card = await screen.findByRole("listitem", { name: "acme-release" });
    const mark = await within(card).findByText("account removed");
    expect(mark).toHaveClass("text-destructive");
    expect(mark).toHaveAttribute("title", REMOVED);
    const other = screen.getByRole("listitem", { name: personal.name });
    expect(within(other).queryByText("account removed")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Dot settings" }));
    expect(await screen.findByRole("menuitem", { name: "Run now" })).toHaveAttribute(
      "data-disabled",
    );
    // Turning it off still works.
    await user.click(screen.getByRole("menuitemcheckbox", { name: "Enabled" }));
    await waitFor(() =>
      expect(backend.callsTo("set_dot_enabled")).toEqual([{ id: "D2", enabled: false }]),
    );
  });

  it("can't be sent a message from the chat at home, which says why", async () => {
    backend.on("org_list", () => []);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    backend.on("dot_overview", () => [{ ...sample.overview, dot: release }]);
    renderApp(`/home/${release.id}`);
    const why = await screen.findByText(`account removed: ${REMOVED}`);
    expect(why).toHaveClass("text-destructive");
    const box = screen.getByLabelText("Message");
    expect(box).toBeDisabled();
    expect(box).toHaveAttribute("placeholder", "account removed");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(backend.callsTo("chat")).toEqual([]);
    // Opened from Personal, where it is listed: no switch is asked for.
    expect(backend.callsTo("scope_set")).toEqual([]);
  });

  it("can be sent a message from the chat while its org is joined", async () => {
    backend.on("org_list", () => [acme]);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    backend.on("dot_overview", () => [{ ...sample.overview, dot: release }]);
    const { user } = renderApp(`/home/${release.id}`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(backend.callsTo("org_list").length).toBeGreaterThan(0));
    expect(box).toBeEnabled();
    expect(screen.queryByText(/account removed/)).toBeNull();
    await user.type(box, "Ship it{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(1));
  });

  it("is not marked while its org is joined", async () => {
    backend.on("org_list", () => [acme]);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    backend.on("dot_overview", () => [{ ...sample.overview, dot: release }]);
    renderApp(`/home/${release.id}`);
    await screen.findByRole("listitem", { name: "acme-release" });
    await waitFor(() => expect(backend.callsTo("org_list").length).toBeGreaterThan(0));
    expect(screen.queryByText("account removed")).toBeNull();
  });
});

describe("a Personal dot", () => {
  it("shows no org, asks for none, and saves and runs as before", async () => {
    backend.on("dot_overview", () => [{ ...sample.overview, dot: personal }]);
    editor(personal, [acme]);
    expect(await screen.findByLabelText("Name")).toHaveValue(personal.name);
    expect(screen.queryByRole("group", { name: "Account" })).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Run now" })).toBeEnabled();
    expect(screen.queryByText("account removed")).toBeNull();
    expect(backend.callsTo("org_list")).toEqual([]);
  });
});

describe("a Viewer in the editor", () => {
  const viewer: OrgSummary = { ...acme, membership: "active", role: "viewer" };
  const TITLE = "Acme Engineering: a Viewer can't create or change dots. Ask an Admin or Owner.";

  it("can't save a dot of the account", async () => {
    editor(release, [viewer]);
    const save = await screen.findByRole("button", { name: "Save" });
    await waitFor(() => expect(save).toBeDisabled());
    expect(save).toHaveAttribute("title", TITLE);
  });

  it("can't create a dot in the account chosen, but can in Personal", async () => {
    const { user } = await newDot([viewer]);
    await user.selectOptions(screen.getByLabelText("Account"), "acme-eng");
    const create = screen.getByRole("button", { name: "Create dot" });
    await waitFor(() => expect(create).toBeDisabled());
    expect(create).toHaveAttribute("title", TITLE);
    await user.selectOptions(screen.getByLabelText("Account"), "");
    await waitFor(() => expect(create).toBeEnabled());
  });
});
