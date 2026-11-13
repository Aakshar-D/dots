import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ApprovalGuardrail, HeldRun, Inbox } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [approval] = sample.inbox.approvals;
const second = { ...approval, id: "apr-2", tool: "Write", input: { file_path: "notes.md" } };
const empty: Inbox = {
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  instructions: [],
  app_reviews: [],
  team_scope: [],
  workflows: [],
  not_delivered: [],
  back: [],
  snoozed: [],
  done: [],
  hidden_messages: [],
};

const memberHit: ApprovalGuardrail = {
  what: "pushing to main or master",
  workspace_name: "Platform",
  needs_admin: true,
  text: "This needs an Admin: pushing to main or master is protected in the Platform workspace.",
};
const adminHit: ApprovalGuardrail = {
  what: "the file migrations/0001.sql",
  workspace_name: "Platform",
  needs_admin: false,
  text: "The file migrations/0001.sql is protected in the Platform workspace, so it always asks.",
};

const card = (name = "Approval for Bash") => screen.findByRole("article", { name });

describe("an approval the guardrails protect", () => {
  it("needs an Admin on a Member's sol: Approve is off with the reason, Deny works", async () => {
    backend.on("inbox", () => ({ ...empty, approvals: [approval] }));
    backend.on("approval_guardrail", () => memberHit);
    backend.on("decide_approval", () => ({ ...sample.approval, status: "denied" }));
    const { user } = renderApp("/inbox");
    const shown = await card();
    const note = await within(shown).findByLabelText("Guardrails");
    expect(note).toHaveTextContent(`Needs an Admin${memberHit.text}`);
    expect(backend.callsTo("approval_guardrail")[0]).toEqual({ id: approval.id });
    const approve = within(shown).getByRole("button", { name: "Approve" });
    expect(approve).toBeDisabled();
    expect(approve).toHaveAttribute("title", memberHit.text);
    // The core offers no rule for it, so there is no always allow either.
    expect(within(shown).queryByRole("button", { name: "Approve and always allow" })).toBeNull();
    await user.click(within(shown).getByRole("button", { name: "Deny" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: approval.id, approved: false, note: null, allowRule: null },
      ]),
    );
  });

  it("keeps Always allow off too, should a rule be offered", async () => {
    backend.on("inbox", () => ({ ...empty, approvals: [approval] }));
    backend.on("approval_guardrail", () => memberHit);
    backend.on("approval_rule_candidates", () => sample.rules);
    renderApp("/inbox");
    const shown = await card();
    await within(shown).findByLabelText("Guardrails");
    expect(
      await within(shown).findByRole("button", { name: "Approve and always allow" }),
    ).toBeDisabled();
  });

  it("only says it always asks on an Admin's sol, and Approve works", async () => {
    backend.on("inbox", () => ({ ...empty, approvals: [approval] }));
    backend.on("approval_guardrail", () => adminHit);
    backend.on("decide_approval", () => ({ ...sample.approval, status: "approved" }));
    const { user } = renderApp("/inbox");
    const shown = await card();
    const note = await within(shown).findByLabelText("Guardrails");
    expect(note).toHaveTextContent(adminHit.text);
    expect(within(note).queryByText("Needs an Admin")).toBeNull();
    await user.click(within(shown).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: approval.id, approved: true, note: null, allowRule: null },
      ]),
    );
  });

  it("turns Approve all off when one call of the run needs an Admin; Deny all stays", async () => {
    backend.on("inbox", () => ({ ...empty, approvals: [approval, second] }));
    backend.on("approval_guardrail", ({ id }) => (id === second.id ? memberHit : null));
    renderApp("/inbox");
    const group = await screen.findByRole("group", {
      name: `Approvals asked by ${approval.dot_name}`,
    });
    const all = within(group).getByRole("button", { name: "Approve all (2)" });
    await waitFor(() => expect(all).toBeDisabled());
    expect(all).toHaveAttribute("title", memberHit.text);
    expect(within(group).getAllByText("Needs an Admin").length).toBeGreaterThan(0);
    expect(within(group).getByRole("button", { name: "Deny all (2)" })).toBeEnabled();
    // The card of the other call can still be approved alone.
    const bash = within(group).getByRole("article", { name: "Approval for Bash" });
    expect(within(bash).getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("leaves Approve all on when no call is protected", async () => {
    backend.on("inbox", () => ({ ...empty, approvals: [approval, second] }));
    renderApp("/inbox");
    const group = await screen.findByRole("group", {
      name: `Approvals asked by ${approval.dot_name}`,
    });
    await waitFor(() => expect(backend.callsTo("approval_guardrail").length).toBeGreaterThan(1));
    expect(within(group).getByRole("button", { name: "Approve all (2)" })).toBeEnabled();
  });
});

describe("a sol past its workspace's policy ceiling", () => {
  it("waits to run in the Inbox with the core's hold text, and no Run anyway", async () => {
    const [scheduled] = sample.inbox.held as [HeldRun];
    const text =
      "Acme Engineering: this sol's tool policy is past what a Member may use in the Platform workspace (it is Trusted). Change the policy, or ask an Admin.";
    const held: HeldRun = {
      ...scheduled,
      run_id: "01J0RUN8",
      starts_after: null,
      reason: { kind: "role", account: "Acme Engineering", text },
    };
    backend.on("inbox", () => ({ ...empty, held: [held] }));
    backend.on("limits_get", () => sample.limits[0]);
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Waiting to run" });
    const [item] = await within(section).findAllByRole("listitem");
    expect(item).toHaveTextContent(text);
    expect(within(item!).queryByRole("button", { name: "Run anyway" })).toBeNull();
  });
});
