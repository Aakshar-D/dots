import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import { GuardrailsView, RoleRow, type MembersView, type WorkspaceView } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs; // Acme Engineering, active, this PC an Admin
const list: MembersView = { ...sample.members, draft: null };
const [adminView, memberView] = GuardrailsView.array().parse(contract.GuardrailsView) as [
  GuardrailsView,
  GuardrailsView,
];
const defaults = adminView.defaults;
const COMPAT = adminView.compat_note;
const ws = (id: string, name: string): WorkspaceView => ({
  scope: "acme-eng",
  id,
  name,
  use_rules: null,
  my_role: null,
  can_manage: true,
  refusals: allowed,
});

/// The account with Default and Platform; `views` answers `workspace_guardrails` per workspace.
function account(views: Record<string, GuardrailsView>, members: MembersView = list) {
  backend.on("org_list", () => [acme]);
  backend.on("member_list", () => members);
  backend.on("workspace_list", () => [ws("default", "Default"), ws("platform", "Platform")]);
  backend.on("workspace_guardrails", (args) => {
    const view = views[String(args.workspaceId)];
    if (!view) throw { kind: "not_found", message: "no such workspace" };
    return view;
  });
}

/// Opens Settings, then the Guardrails tab of Acme Engineering.
async function guardrailsTab() {
  const view = renderApp("/settings/account");
  const section = await screen.findByRole("group", { name: "Accounts" });
  const row = await within(section).findByRole("listitem", { name: "Acme Engineering" });
  await view.user.click(within(row).getByRole("tab", { name: "Guardrails" }));
  const panel = await within(row).findByRole("tabpanel");
  return { ...view, panel };
}

const unguarded: GuardrailsView = {
  ...adminView,
  workspace_id: "default",
  workspace_name: "Default",
  guardrails: null,
};

describe("Settings, Accounts, the account's tabs", () => {
  it("wrap rather than widen the page in a narrow window", async () => {
    account({ default: unguarded });
    renderApp("/settings/account");
    const section = await screen.findByRole("group", { name: "Accounts" });
    const row = await within(section).findByRole("listitem", { name: "Acme Engineering" });
    const tabs = within(row).getByRole("tablist");
    expect(within(tabs).getAllByRole("tab")).toHaveLength(6);
    // The list's fixed height would clip a second row.
    expect(tabs).toHaveClass("flex-wrap", "group-data-horizontal/tabs:h-auto");
    expect(tabs).not.toHaveClass("group-data-horizontal/tabs:h-8");
  });
});

describe("Settings, Accounts, Guardrails", () => {
  it("shows a workspace's guardrails to an Admin, with the git actions in the core's words", async () => {
    account({ default: unguarded, platform: adminView });
    const { user, panel } = await guardrailsTab();
    expect(await within(panel).findByText("Default has no guardrails.", { exact: false })).toBeVisible();
    await user.selectOptions(within(panel).getByLabelText("Workspace"), "platform");
    const section = await within(panel).findByRole("region", { name: "Guardrails of Platform" });
    expect(backend.callsTo("workspace_guardrails").at(-1)).toEqual({
      orgId: "acme-eng",
      workspaceId: "platform",
    });
    expect(within(section).getByLabelText("Protected paths")).toHaveValue(
      defaults.protected_paths.join("\n"),
    );
    expect(within(section).getByLabelText("Protected commands")).toHaveValue(
      defaults.protected_commands.join("\n"),
    );
    for (const { label } of adminView.git_actions) {
      const box = within(section).getByRole("checkbox", {
        name: label.charAt(0).toUpperCase() + label.slice(1),
      });
      expect(box).toBeChecked();
    }
    expect(within(section).getByLabelText("Member max")).toHaveValue("sandboxed");
    // Nothing changed yet, so there is nothing to sign.
    expect(within(section).getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("Use defaults fills an empty workspace, and Save signs it against the list's seq after the older-Dots note", async () => {
    account({ default: unguarded, platform: adminView });
    backend.on("workspace_guardrails_set", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    expect(within(section).queryByLabelText("Protected paths")).toBeNull();
    await user.click(within(section).getByRole("button", { name: "Use defaults" }));
    expect(within(section).getByLabelText("Protected paths")).toHaveValue(
      defaults.protected_paths.join("\n"),
    );
    // The note is on the form before anything is signed.
    expect(within(section).getByText(COMPAT)).toBeVisible();
    await user.click(within(section).getByRole("button", { name: "Save" }));
    const ask = await screen.findByRole("alertdialog");
    expect(ask).toHaveAccessibleDescription(COMPAT);
    await user.click(within(ask).getByRole("button", { name: "Sign and save" }));
    await waitFor(() => expect(backend.callsTo("workspace_guardrails_set")).toHaveLength(1));
    expect(backend.callsTo("workspace_guardrails_set")[0]).toEqual({
      orgId: "acme-eng",
      baseSeq: adminView.seq,
      workspaceId: "default",
      guardrails: defaults,
    });
  });

  it("sends what was edited: lines trimmed, unticked git actions left out, the new ceiling", async () => {
    account({ default: { ...adminView, workspace_id: "default", workspace_name: "Default" } });
    backend.on("workspace_guardrails_set", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    const paths = within(section).getByLabelText("Protected paths");
    await user.clear(paths);
    await user.type(paths, "  db/**  {Enter}{Enter}secrets/**");
    await user.click(within(section).getByRole("checkbox", { name: "Force pushing" }));
    const commandsBox = within(section).getByLabelText("Protected commands");
    await user.clear(commandsBox);
    await user.type(commandsBox, "make deploy");
    await user.selectOptions(within(section).getByLabelText("Member max"), "read-only");
    await user.click(within(section).getByRole("button", { name: "Save" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Sign and save" }),
    );
    await waitFor(() => expect(backend.callsTo("workspace_guardrails_set")).toHaveLength(1));
    expect(backend.callsTo("workspace_guardrails_set")[0]?.guardrails).toEqual({
      protected_paths: ["db/**", "secrets/**"],
      protected_git: defaults.protected_git.filter((a) => a !== "force_push"),
      protected_commands: ["make deploy"],
      member_max_preset: "read-only",
    });
  });

  it("Turn off sends null after saying what it does", async () => {
    account({ default: { ...adminView, workspace_id: "default", workspace_name: "Default" } });
    backend.on("workspace_guardrails_set", () => ({ outcome: "published", summary: acme }));
    const { user, panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    await user.click(within(section).getByRole("button", { name: "Turn off" }));
    expect(within(section).getByText(/Off: nothing is protected in Default/)).toBeVisible();
    expect(within(section).queryByText(COMPAT)).toBeNull();
    await user.click(within(section).getByRole("button", { name: "Save" }));
    const ask = await screen.findByRole("alertdialog");
    expect(ask).toHaveAccessibleName("Turn off the guardrails of Default?");
    await user.click(within(ask).getByRole("button", { name: "Sign and save" }));
    await waitFor(() =>
      expect(backend.callsTo("workspace_guardrails_set")[0]).toMatchObject({ guardrails: null }),
    );
  });

  it("a stale list is refused with the core's words, and the guardrails are asked for again", async () => {
    account({ default: unguarded });
    const changed =
      "Acme Engineering: the member list changed (now #8) while you were editing. Check it and " +
      "make the change again.";
    backend.on("workspace_guardrails_set", () => {
      throw { kind: "conflict", message: changed };
    });
    const { user, panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    const before = backend.callsTo("workspace_guardrails").length;
    await user.click(within(section).getByRole("button", { name: "Use defaults" }));
    await user.click(within(section).getByRole("button", { name: "Save" }));
    await user.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Sign and save" }),
    );
    expect(await within(panel).findByRole("alert")).toHaveTextContent(changed);
    await waitFor(() =>
      expect(backend.callsTo("workspace_guardrails").length).toBeGreaterThan(before),
    );
    await waitFor(() =>
      expect(backend.callsTo("member_list").length).toBeGreaterThan(1),
    );
  });

  it("shows a Member the guardrails read-only, with the core's reason", async () => {
    const guarded: GuardrailsView = { ...memberView, guardrails: defaults, applies_as: "member" };
    account({ default: guarded });
    const { panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    expect(within(section).getByText(memberView.edit_refusal!)).toBeVisible();
    expect(
      within(section).getByText(/Your dots here are guarded as a Member's/),
    ).toBeVisible();
    expect(within(section).queryByRole("button", { name: "Use defaults" })).toBeNull();
    expect(within(section).queryByRole("button", { name: "Save" })).toBeNull();
    expect(within(section).queryByRole("textbox")).toBeNull();
    const git = within(section).getByRole("list", { name: "Protected git actions" });
    expect(within(git).getByText("Pushing to main or master")).toBeVisible();
    const paths = within(section).getByRole("list", { name: "Protected paths" });
    expect(within(paths).getAllByRole("listitem")).toHaveLength(defaults.protected_paths.length);
    expect(section).toHaveTextContent("Member max: Sandboxed");
  });

  it("shows an Admin who can't sign why, read-only", async () => {
    const blocker =
      "Acme Engineering: your Dots ID isn't one of the account's signing admins yet.";
    account({ default: { ...unguarded, sign_blocker: blocker } });
    const { panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    expect(within(section).getByText(blocker)).toBeVisible();
    expect(within(section).queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("waits for a draft list: the controls are off with the reason", async () => {
    account({ default: unguarded }, sample.members);
    const { panel } = await guardrailsTab();
    const section = await within(panel).findByRole("region", { name: "Guardrails of Default" });
    expect(within(section).getByRole("region", { name: "Unpublished member list" })).toBeVisible();
    const defaultsButton = within(section).getByRole("button", { name: "Use defaults" });
    expect(defaultsButton).toBeDisabled();
    expect(defaultsButton).toHaveAttribute(
      "title",
      "Acme Engineering: a change is waiting to be published. Retry or discard it first.",
    );
  });
});

describe("Settings, Accounts, Members, what each role can do", () => {
  it("shows the core's permission table, with its conditions", async () => {
    const rows = RoleRow.array().parse(contract.RoleRow);
    backend.on("org_list", () => [acme]);
    backend.on("member_list", () => list);
    backend.on("role_matrix", () => rows);
    const view = renderApp("/settings/account");
    const section = await screen.findByRole("group", { name: "Accounts" });
    const row = await within(section).findByRole("listitem", { name: "Acme Engineering" });
    await view.user.click(within(row).getByRole("tab", { name: "Members" }));
    const panel = await within(row).findByRole("tabpanel");
    await view.user.click(await within(panel).findByText("What each role can do"));
    const table = await within(panel).findByRole("table", { name: "What each role can do" });
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((h) => h.textContent),
    ).toEqual(["Can", "Owner", "Admin", "Member", "Viewer"]);
    const guard = within(table).getByRole("row", { name: /change guardrails/ });
    expect(
      within(guard)
        .getAllByRole("cell")
        // What a screen reader says; the mark and dash are hidden from it.
        .map((c) => c.querySelector(".sr-only")?.textContent),
    ).toEqual(["Yes", "Yes", "No", "No"]);
    const approvals = within(table).getByRole("rowheader", { name: /answer approvals/ });
    expect(approvals).toHaveTextContent("A Member can't approve a call the workspace's guardrails protect");
    expect(within(table).getAllByRole("row")).toHaveLength(rows.length + 1);
  });
});
