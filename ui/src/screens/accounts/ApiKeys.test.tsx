import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { ApiKeyView, WorkspaceView } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [acme] = sample.orgs;
const SECRET = "dots_sk_" + "a".repeat(43);
const ws = (id: string, name: string, create_api_keys: string | null = null): WorkspaceView => ({
  scope: "acme-eng",
  id,
  name,
  use_rules: null,
  my_role: null,
  can_manage: true,
  refusals: { ...allowed, create_api_keys },
});
const key = (over: Partial<ApiKeyView> = {}): ApiKeyView => ({
  id: "k1",
  scope: "acme-eng",
  workspace_id: "default",
  workspace_name: "Default",
  name: "CI",
  prefix: "dots_sk_AbCd",
  created_at: "2026-10-01T09:00:00Z",
  last_used_at: null,
  revoked_at: null,
  refusal: null,
  ...over,
});

function account(opts: { keys?: ApiKeyView[]; spaces?: WorkspaceView[]; url?: string } = {}) {
  let keys = opts.keys ?? [key()];
  backend.on("org_list", () => [acme]);
  backend.on("member_list", () => ({ ...sample.members, draft: null }));
  backend.on("workspace_list", () =>
    opts.spaces ?? [ws("default", "Default"), ws("ops", "Ops", "Viewers can't make keys here.")],
  );
  backend.on("api_key_list", () => keys);
  backend.on("api_key_create", (args) => {
    const made = key({ id: "k2", name: String(args.name), workspace_id: String(args.workspaceId) });
    keys = [made, ...keys];
    return { key: made, secret: SECRET };
  });
  backend.on("api_key_revoke", () => {
    keys = keys.map((k) => ({ ...k, revoked_at: "2026-10-02T09:00:00Z" }));
    return keys[0];
  });
  backend.on("api_base_url", () => {
    if (opts.url) return opts.url;
    throw { kind: "conflict", message: "The local server is not running." };
  });
}

async function keysTab() {
  const view = renderApp("/settings");
  const section = await screen.findByRole("group", { name: "Accounts" });
  const row = await within(section).findByRole("listitem", { name: "Acme Engineering" });
  await view.user.click(within(row).getByRole("tab", { name: "API keys" }));
  const panel = await within(row).findByRole("tabpanel");
  return { ...view, panel };
}

describe("Settings, Accounts, API keys", () => {
  it("lists_keys_with_prefix_and_last_used", async () => {
    account({
      keys: [
        key(),
        key({ id: "k0", name: "Old", prefix: "dots_sk_ZzZz", last_used_at: "2026-10-03T10:00:00Z", revoked_at: "2026-10-04T10:00:00Z" }),
      ],
    });
    const { panel } = await keysTab();
    const ci = await within(panel).findByRole("listitem", { name: "CI" });
    expect(ci).toHaveTextContent("dots_sk_AbCd…");
    expect(ci).toHaveTextContent("Default");
    expect(ci).toHaveTextContent("Never");
    expect(within(ci).queryByText("Revoked")).toBeNull();
    const old = within(panel).getByRole("listitem", { name: "Old" });
    expect(old).not.toHaveTextContent("Never");
    expect(within(old).getByText("Revoked")).toBeInTheDocument();
  });

  it("create_shows_the_secret_once", async () => {
    account({ url: "http://127.0.0.1:4455" });
    const { user, panel, queryClient } = await keysTab();
    await within(panel).findByRole("listitem", { name: "CI" });
    await user.click(within(panel).getByRole("button", { name: "Create key" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Deploy");
    await user.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(await within(dialog).findByDisplayValue(SECRET)).toHaveAttribute("readonly");
    expect(dialog).toHaveTextContent("Copy it now. Dots won't show it again.");
    expect(dialog).toHaveTextContent("http://127.0.0.1:4455");
    expect(dialog).toHaveTextContent(
      `curl -H "Authorization: Bearer ${SECRET}" http://127.0.0.1:4455/dots`,
    );
    expect(backend.callsTo("api_key_create")[0]).toEqual({
      scope: "acme-eng",
      workspaceId: "default",
      name: "Deploy",
    });
    await user.click(within(dialog).getByRole("button", { name: "Done" }));
    // Neither cache keeps the key once the dialog is closed.
    const held = JSON.stringify([
      queryClient.getMutationCache().getAll().map((m) => [m.state.data, m.state.variables]),
      queryClient.getQueryCache().getAll().map((q) => q.state.data),
    ]);
    expect(held).not.toContain(SECRET);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    // The list has the new key, by its prefix alone, and the secret is nowhere.
    await within(panel).findByRole("listitem", { name: "Deploy" });
    expect(document.body.textContent).not.toContain(SECRET);
    await user.click(within(panel).getByRole("button", { name: "Create key" }));
    const again = await screen.findByRole("dialog");
    expect(within(again).queryByDisplayValue(SECRET)).toBeNull();
    expect(again.textContent).not.toContain(SECRET);
  });

  it("revoke_asks_to_confirm", async () => {
    account();
    const { user, panel } = await keysTab();
    await user.click(await within(panel).findByRole("button", { name: "Revoke CI" }));
    const confirm = await screen.findByRole("alertdialog");
    expect(confirm).toHaveTextContent("Apps using this key stop working at once.");
    expect(backend.callsTo("api_key_revoke")).toHaveLength(0);
    await user.click(within(confirm).getByRole("button", { name: "Revoke" }));
    await waitFor(() => expect(backend.callsTo("api_key_revoke")).toEqual([{ id: "k1" }]));
    expect(await within(panel).findByText("Revoked")).toBeInTheDocument();
  });

  it("viewer_controls_disabled_with_reason", async () => {
    account({ keys: [key({ refusal: "Only an Admin can revoke this key." })] });
    const { user, panel } = await keysTab();
    const revoke = await within(panel).findByRole("button", { name: "Revoke CI" });
    expect(revoke).toBeDisabled();
    expect(revoke).toHaveAttribute("title", "Only an Admin can revoke this key.");
    await user.click(within(panel).getByRole("button", { name: "Create key" }));
    const dialog = await screen.findByRole("dialog");
    const ops = within(dialog).getByRole("option", { name: "Ops" });
    expect(ops).toBeDisabled();
    expect(ops).toHaveAttribute("title", "Viewers can't make keys here.");
    expect(within(dialog).getByRole("option", { name: "Default" })).toBeEnabled();
  });

  it("create_is_off_with_the_core_text_when_every_workspace_is_refused", async () => {
    account({ spaces: [ws("default", "Default", "Viewers can't make keys."), ws("ops", "Ops", "Viewers can't make keys.")] });
    const { user, panel } = await keysTab();
    await within(panel).findByRole("listitem", { name: "CI" });
    await user.click(within(panel).getByRole("button", { name: "Create key" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Deploy");
    const create = within(dialog).getByRole("button", { name: "Create" });
    expect(create).toBeDisabled();
    expect(create).toHaveAttribute("title", "Viewers can't make keys.");
    expect(dialog).toHaveTextContent("Viewers can't make keys.");
  });

  it("server_off_shows_the_reason", async () => {
    account();
    const { user, panel } = await keysTab();
    await within(panel).findByRole("listitem", { name: "CI" });
    await user.click(within(panel).getByRole("button", { name: "Create key" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Deploy");
    await user.click(within(dialog).getByRole("button", { name: "Create" }));
    await within(dialog).findByDisplayValue(SECRET);
    expect(dialog).toHaveTextContent("The local server is not running.");
  });
});
