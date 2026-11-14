import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, Policy, PolicyCeiling } from "@/api/types";
import { specOf } from "@/lib/dot-rules";
import { backend } from "@/test/backend";
import { noCeiling, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const refusal =
  "Acme Engineering: a Member's sol in the Platform workspace may use up to Sandboxed, and this policy is Trusted. Ask an Admin or Owner.";
const heldText =
  "Acme Engineering: this sol's tool policy is past what a Member may use in the Platform workspace (it is Trusted). Change the policy, or ask an Admin.";
const protectedHere = {
  protected_paths: ["**/migrations/**", ".github/**"],
  protected_git: ["push_main" as const, "force_push" as const],
  protected_commands: ["rm -rf"],
  member_max_preset: "sandboxed" as const,
};

/// The core's answer for a Member in Platform, whose ceiling is Sandboxed: Trusted is refused,
/// and so is a policy that is Trusted.
function memberCeiling(policy: Policy | null): PolicyCeiling {
  return {
    applies_as: "member",
    workspace_name: "Platform",
    max_preset: "sandboxed",
    presets: noCeiling.presets.map((p) => ({
      preset: p.preset,
      refusal: p.preset === "trusted" ? refusal : null,
    })),
    policy_refusal: policy?.preset === "trusted" ? refusal : null,
    held: null,
    protected: protectedHere,
  };
}

const trusted: Policy = {
  ...sample.dot.policy,
  preset: "trusted",
  rules: [{ tool: "*", action: "allow" }],
  default: "allow",
};

function stored(dot: Dot) {
  backend.on("org_list", () => [sample.orgs[0]]);
  backend.on("get_dot", () => dot);
  backend.on("update_dot", (args) => ({ ...dot, ...(args.spec as object) }));
  backend.on("policy_ceiling", (args) => memberCeiling(args.policy as Policy));
}

const inPlatform: Dot = { ...sample.dot, org_id: sample.orgs[0]!.id, workspace_id: "platform" };

describe("the sol editor under a workspace's guardrails", () => {
  it("says how far a Member's sol may go, turns Trusted off with the reason, and lists what is protected", async () => {
    stored(inPlatform);
    backend.on("dot_ceiling", () => memberCeiling(inPlatform.policy));
    const { user } = renderApp(`/dots/${inPlatform.id}`);
    expect(
      await screen.findByText(/Members' sols in Platform can go up to Sandboxed/),
    ).toBeVisible();
    // Asked for the dot's own account, workspace and policy.
    expect(backend.callsTo("policy_ceiling")[0]).toEqual({
      orgId: sample.orgs[0]!.id,
      workspaceId: "platform",
      policy: inPlatform.policy,
    });
    const preset = screen.getByLabelText("Preset");
    const trustedOption = within(preset).getByRole("option", { name: /^Trusted/ });
    expect(trustedOption).toBeDisabled();
    expect(trustedOption).toHaveAttribute("title", refusal);
    expect(preset).toHaveAccessibleDescription(`Trusted is off: ${refusal}`);
    expect(within(preset).getByRole("option", { name: /^Read-only/ })).toBeEnabled();
    // The policy itself is within the ceiling: no refusal, and Save works.
    expect(screen.queryByRole("alert")).toBeNull();
    const summary = screen.getByText(/Protected in Platform: 2 paths, 2 git actions, 1 command/);
    await user.click(summary);
    expect(screen.getByText("**/migrations/**, .github/**")).toBeVisible();
    expect(screen.getByText("pushing to main or master, force pushing")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
  });

  it("holds Save for a change that keeps a policy past the ceiling, shows why its runs wait, and lets it be lowered", async () => {
    const over: Dot = { ...inPlatform, policy: trusted };
    stored(over);
    backend.on("dot_ceiling", () => ({ ...memberCeiling(trusted), held: heldText }));
    backend.on("policy_preset", () => sample.dot.policy);
    const { user } = renderApp(`/dots/${over.id}`);
    expect(await screen.findByText(heldText)).toBeVisible();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(refusal);
    // A change to the policy that stays past the ceiling can't be saved.
    const askSendDelete = screen.getByRole("switch", {
      name: "Always ask before sending or deleting",
    });
    await user.click(askSendDelete);
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "Save" })).toHaveAttribute("title", refusal);
    await user.click(askSendDelete);
    // Lowered to Sandboxed, the core takes it.
    await user.selectOptions(screen.getByLabelText("Preset"), "sandboxed");
    await waitFor(() => expect(screen.getByRole("button", { name: "Save" })).toBeEnabled());
    expect(screen.queryByRole("alert")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    expect(backend.callsTo("update_dot")[0]).toEqual({
      id: over.id,
      spec: { ...specOf(over), policy: sample.dot.policy },
    });
  });

  it("still saves a held sol whose policy stays as it is, so it can be renamed or switched off", async () => {
    const over: Dot = { ...inPlatform, policy: trusted };
    stored(over);
    backend.on("dot_ceiling", () => ({ ...memberCeiling(trusted), held: heldText }));
    const { user } = renderApp(`/dots/${over.id}`);
    expect(await screen.findByText(heldText)).toBeVisible();
    // The refusal still says why the policy is past the ceiling, but Save goes: the core takes
    // a save that keeps the policy (guardrails spec section 4.3).
    expect(await screen.findByRole("alert")).toHaveTextContent(refusal);
    const name = screen.getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Renamed");
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    expect(backend.callsTo("update_dot")[0]).toEqual({
      id: over.id,
      spec: { ...specOf(over), name: "Renamed" },
    });
  });

  it("tells an Admin their protected calls ask, with every preset open", async () => {
    stored(inPlatform);
    backend.on("policy_ceiling", () => ({
      ...noCeiling,
      applies_as: "admin",
      workspace_name: "Platform",
      protected: protectedHere,
    }));
    renderApp(`/dots/${inPlatform.id}`);
    expect(
      await screen.findByText("Platform has guardrails: your sols always ask before a protected call."),
    ).toBeVisible();
    const preset = screen.getByLabelText("Preset");
    expect(within(preset).getByRole("option", { name: /^Trusted/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("shows nothing of guardrails where none apply, and asks for a new sol's account", async () => {
    renderApp("/dots/new");
    await screen.findByLabelText("Preset");
    await waitFor(() => expect(backend.callsTo("policy_ceiling").length).toBeGreaterThan(0));
    expect(backend.callsTo("policy_ceiling")[0]).toMatchObject({
      orgId: null,
      workspaceId: "default",
    });
    expect(screen.queryByText(/can go up to/)).toBeNull();
    expect(screen.queryByText(/^Protected in/)).toBeNull();
  });
});
