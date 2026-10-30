import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { MembersView, OrgSummary, WorkspaceView } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs;
const list: MembersView = { ...sample.members, draft: null };
const ws = (scope: string, id: string, name: string, can_manage = true): WorkspaceView => ({
  scope,
  id,
  name,
  use_rules: null,
  my_role: null,
  can_manage,
  refusals: allowed,
});

/// Answers `org_list`, `member_list` and `workspace_list` like the core for one account.
function account(opts: { org?: OrgSummary; view?: MembersView; manage?: boolean } = {}) {
  const manage = opts.manage ?? true;
  backend.on("org_list", () => [opts.org ?? acme]);
  backend.on("member_list", () => opts.view ?? list);
  backend.on("workspace_list", (args) =>
    args.scope === "personal"
      ? [ws("personal", "default", "Default"), ws("personal", "side", "Side")]
      : [ws("acme-eng", "default", "Default", manage), ws("acme-eng", "ops", "Ops", manage)],
  );
}

async function settings() {
  const view = renderApp("/settings");
  const section = await screen.findByRole("group", { name: "Accounts" });
  return { ...view, section };
}

async function workspacesTab() {
  const view = await settings();
  const row = await within(view.section).findByRole("listitem", { name: "Acme Engineering" });
  await view.user.click(within(row).getByRole("tab", { name: "Workspaces" }));
  const panel = await within(row).findByRole("tabpanel");
  await within(panel).findByRole("list", { name: "Workspaces in Acme Engineering" });
  return { ...view, panel };
}

describe("Settings, Accounts, Workspaces", () => {
  it("admin adds a workspace through a signed edit", async () => {
    account();
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await workspacesTab();
    const items = within(panel).getByRole("list", { name: "Workspaces in Acme Engineering" });
    expect(within(items).getByRole("listitem", { name: "Default" })).toHaveTextContent("Default");
    expect(within(items).getByRole("listitem", { name: "Ops" })).toHaveTextContent(
      "use: no extra rules",
    );
    const before = backend.callsTo("workspace_list").length;
    await user.click(within(panel).getByRole("button", { name: "Add workspace" }));
    const dialog = await screen.findByRole("dialog");
    // A new workspace's id comes from its name.
    expect(dialog).toHaveAccessibleDescription("Its id is made from the name.");
    await user.type(within(dialog).getByLabelText("Name"), "Payments");
    await user.click(within(dialog).getByRole("button", { name: "Add" }));
    await waitFor(() => expect(backend.callsTo("member_edit")).toHaveLength(1));
    expect(backend.callsTo("member_edit")[0]).toEqual({
      orgId: "acme-eng",
      baseSeq: 3,
      edit: { kind: "add_workspace", name: "Payments" },
    });
    // The workspaces are asked for again.
    await waitFor(() => expect(backend.callsTo("workspace_list").length).toBeGreaterThan(before));
  });

  it("rename sends rename_workspace", async () => {
    account();
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await workspacesTab();
    await user.click(within(panel).getByRole("button", { name: "Rename Ops" }));
    const dialog = await screen.findByRole("dialog");
    // Renaming keeps the id, so nothing says it is made from the name.
    expect(within(dialog).queryByText("Its id is made from the name.")).toBeNull();
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Platform ops");
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));
    await waitFor(() => expect(backend.callsTo("member_edit")).toHaveLength(1));
    expect(backend.callsTo("member_edit")[0].edit).toEqual({
      kind: "rename_workspace",
      id: "ops",
      name: "Platform ops",
    });
  });

  it("delete asks to confirm and Default can't be deleted", async () => {
    account();
    backend.on("member_edit", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await workspacesTab();
    expect(within(panel).queryByRole("button", { name: "Delete Default" })).toBeNull();
    await user.click(within(panel).getByRole("button", { name: "Delete Ops" }));
    const ask = await screen.findByRole("alertdialog");
    expect(ask).toHaveTextContent("Its dots on each PC move to Default.");
    expect(backend.callsTo("member_edit")).toHaveLength(0);
    await user.click(within(ask).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(backend.callsTo("member_edit")).toHaveLength(1));
    expect(backend.callsTo("member_edit")[0].edit).toEqual({ kind: "delete_workspace", id: "ops" });
  });

  it("member sees workspaces read-only", async () => {
    account({
      org: { ...acme, role: "member" },
      view: { ...list, can_manage: false, can_sign: false, sign_blocker: "refused" },
      manage: false,
    });
    const { panel } = await workspacesTab();
    const why = "Acme Engineering: a Member can't manage members. Ask an Admin or Owner.";
    for (const name of ["Add workspace", "Rename Ops", "Delete Ops", "Rename Default"]) {
      const button = within(panel).getByRole("button", { name });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", why);
    }
  });

  it("a waiting draft holds workspace edits with the draft words", async () => {
    const waiting =
      "Acme Engineering: a change is waiting to be published. Retry or discard it first.";
    account({
      view: {
        ...list,
        draft: {
          base_seq: 3,
          seq: 4,
          summary: "Add the Payments workspace",
          signed: "x",
          error: "",
          created_at: "2026-10-30T10:00:00Z",
        },
      },
    });
    const { panel } = await workspacesTab();
    expect(within(panel).getByText(/Add the Payments workspace/)).toBeVisible();
    const add = within(panel).getByRole("button", { name: "Add workspace" });
    expect(add).toBeDisabled();
    expect(add).toHaveAttribute("title", waiting);
  });

  it("personal workspaces add rename delete", async () => {
    account();
    backend.on("personal_workspace_add", (args) => ws("personal", "new", String(args.name)));
    backend.on("personal_workspace_rename", () => null);
    backend.on("personal_workspace_delete", () => null);
    const { user, section } = await settings();
    const personal = await within(section).findByRole("region", { name: "Personal" });
    await within(personal).findByRole("list", { name: "Workspaces in Personal" });
    await user.click(within(personal).getByRole("button", { name: "Add workspace" }));
    let dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Games");
    await user.click(within(dialog).getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(backend.callsTo("personal_workspace_add")).toEqual([{ name: "Games" }]),
    );

    await user.click(within(personal).getByRole("button", { name: "Rename Side" }));
    dialog = await screen.findByRole("dialog");
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Hobby");
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));
    await waitFor(() =>
      expect(backend.callsTo("personal_workspace_rename")).toEqual([{ id: "side", name: "Hobby" }]),
    );

    expect(within(personal).queryByRole("button", { name: "Delete Default" })).toBeNull();
    await user.click(within(personal).getByRole("button", { name: "Delete Side" }));
    const ask = await screen.findByRole("alertdialog");
    expect(ask).toHaveTextContent("Its dots on each PC move to Default.");
    await user.click(within(ask).getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(backend.callsTo("personal_workspace_delete")).toEqual([{ id: "side" }]),
    );
  });

  it("deleting the Personal workspace in view loads the scope and the dots again", async () => {
    account();
    backend.on("scope_get", () => ({ account: "personal", workspace: "side" }));
    backend.on("personal_workspace_delete", () => null);
    const { user, section } = await settings();
    const personal = await within(section).findByRole("region", { name: "Personal" });
    await within(personal).findByRole("list", { name: "Workspaces in Personal" });
    await waitFor(() => expect(backend.callsTo("scope_get").length).toBeGreaterThan(0));
    const scopes = backend.callsTo("scope_get").length;
    await user.click(within(personal).getByRole("button", { name: "Delete Side" }));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(backend.callsTo("scope_get").length).toBeGreaterThan(scopes));
  });

  it.each(["Rename", "Delete"])(
    "%s of a Personal workspace loads the API keys again",
    async (action) => {
      // A key is listed with its workspace's name, and a deleted workspace's keys are gone.
      account();
      backend.on("personal_workspace_rename", () => null);
      backend.on("personal_workspace_delete", () => null);
      const { user, section } = await settings();
      const personal = await within(section).findByRole("region", { name: "Personal" });
      await within(personal).findByRole("list", { name: "Workspaces in Personal" });
      await waitFor(() => expect(backend.callsTo("api_key_list").length).toBeGreaterThan(0));
      const before = backend.callsTo("api_key_list").length;
      await user.click(within(personal).getByRole("button", { name: `${action} Side` }));
      if (action === "Rename") {
        const dialog = await screen.findByRole("dialog");
        await user.type(within(dialog).getByLabelText("Name"), "2");
        await user.click(within(dialog).getByRole("button", { name: "Rename" }));
      } else {
        await user.click(
          within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }),
        );
      }
      await waitFor(() => expect(backend.callsTo("api_key_list").length).toBeGreaterThan(before));
    },
  );

  it("renaming the Personal workspace in view loads the scope again", async () => {
    account();
    backend.on("scope_get", () => ({ account: "personal", workspace: "side" }));
    backend.on("personal_workspace_rename", () => null);
    const { user, section } = await settings();
    const personal = await within(section).findByRole("region", { name: "Personal" });
    await within(personal).findByRole("list", { name: "Workspaces in Personal" });
    await waitFor(() => expect(backend.callsTo("scope_get").length).toBeGreaterThan(0));
    const scopes = backend.callsTo("scope_get").length;
    await user.click(within(personal).getByRole("button", { name: "Rename Side" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), " 2");
    await user.click(within(dialog).getByRole("button", { name: "Rename" }));
    await waitFor(() => expect(backend.callsTo("scope_get").length).toBeGreaterThan(scopes));
  });
});
