import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview, OrgSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const acme: OrgSummary = { ...sample.orgs[0], id: "acme-eng", name: "Acme Engineering" };
const design: OrgSummary = { ...sample.orgs[6], id: "design", name: "Design" };
const row = (org_id: string | null, needs_you: number): DotOverview => ({
  ...sample.overview,
  dot: { ...sample.dot, id: `dot-${org_id ?? "p"}`, org_id },
  needs_you,
});

function setup(account = "personal") {
  backend.on("identity_get", () => ({ ...sample.identity, name: "Ada Lovelace" }));
  backend.on("org_list", () => [acme, design]);
  // The core keeps the account picked, and answers a switch late, as real IPC can.
  backend.on("scope_get", () => ({ account, workspace: "default" }));
  backend.on("scope_set", async (args) => {
    await new Promise((r) => setTimeout(r, 200));
    account = String(args.account);
    return { account, workspace: "default" };
  });
  backend.on("dot_overview", () => [row(null, 0), row("acme-eng", 2)]);
  return renderApp("/home");
}

describe("the account switcher", () => {
  it("shows the profile initials and the current account", async () => {
    setup();
    const button = await screen.findByRole("button", { name: "Account: Personal" });
    await waitFor(() => expect(button).toHaveTextContent("AL"));
    expect(button).toHaveTextContent("P");
  });

  it("lists Personal and the joined accounts", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    const menu = screen.getByRole("menu");
    const names = within(menu).getAllByRole("menuitemradio").map((i) => i.textContent);
    expect(names[0]).toContain("Personal");
    expect(names[1]).toContain("Acme Engineering");
    expect(names[2]).toContain("Design");
    expect(within(menu).getByRole("menuitem", { name: "Join an account" })).toBeVisible();
    expect(within(menu).getByRole("menuitem", { name: "Manage accounts" })).toBeVisible();
  });

  it("switches to the account picked", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Acme Engineering/ }));
    expect(screen.queryByRole("menu")).toBeNull();
    await screen.findByRole("button", { name: "Account: Acme Engineering" }, { timeout: 3000 });
    await new Promise((r) => setTimeout(r, 300));
    expect(backend.callsTo("scope_set")).toEqual([{ account: "acme-eng", workspace: "default" }]);
    expect(screen.getByRole("button", { name: "Account: Acme Engineering" })).toBeVisible();
  });

  it("counts what waits in an account that is not the current one", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    const acmeItem = await screen.findByRole("menuitemradio", { name: /Acme Engineering/ });
    await waitFor(() => expect(acmeItem).toHaveTextContent("2"));
  });

  it("says when an account is not plainly connected", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    const item = await screen.findByRole("menuitemradio", { name: /Design/ });
    expect(item).toHaveTextContent(/waiting for the admin/i);
  });

  it("closes on Escape and on a click outside", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    expect(screen.getByRole("menu")).toBeVisible();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).toBeNull();
    await user.click(screen.getByRole("button", { name: /^Account:/ }));
    await user.click(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("marks the current account and returns focus to the button", async () => {
    const { user } = setup("acme-eng");
    const button = await screen.findByRole("button", { name: /^Account:/ });
    await user.click(button);
    expect(await screen.findByRole("menuitemradio", { name: /Acme Engineering/ })).toBeChecked();
    expect(screen.getByRole("menuitemradio", { name: /Personal/ })).not.toBeChecked();
    await user.keyboard("{Escape}");
    expect(button).toHaveFocus();
    await user.click(button);
    await user.click(await screen.findByRole("menuitemradio", { name: /Design/ }));
    expect(button).toHaveFocus();
  });

  it("opens Settings at the accounts", async () => {
    const { user, router } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    await user.click(screen.getByRole("menuitem", { name: "Manage accounts" }));
    expect(router.state.location.pathname).toBe("/settings");
    expect(router.state.location.hash).toBe("#accounts");
  });

  it("labels each account with the person's role in it", async () => {
    const { user } = setup();
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    const items = await screen.findAllByRole("menuitemradio");
    expect(items[0]).toHaveTextContent("Owner");
    expect(items[1]).toHaveTextContent("Admin");
    expect(items[2]).toHaveTextContent("Member");
  });
});

describe("the workspaces under an account", () => {
  const ws = (id: string, name: string) => ({
    scope: "personal", id, name, use_rules: null, my_role: null, can_manage: true,
    refusals: allowed,
  });

  it("switcher lists the account's workspaces and picks one", async () => {
    const { user } = setup();
    backend.on("workspace_list", () => [ws("default", "Default"), ws("ops", "Ops")]);
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    const menu = screen.getByRole("menu");
    const current = await within(menu).findByRole("menuitemradio", { name: "Default" });
    expect(current).toHaveAttribute("aria-checked", "true");
    await user.click(within(menu).getByRole("menuitemradio", { name: "Ops" }));
    await waitFor(() =>
      expect(backend.callsTo("scope_set")).toEqual([{ account: "personal", workspace: "ops" }]),
    );
  });

  it("single Default workspace shows no list", async () => {
    const { user } = setup();
    backend.on("workspace_list", () => [ws("default", "Default")]);
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    await waitFor(() => expect(backend.callsTo("workspace_list").length).toBeGreaterThan(0));
    expect(screen.queryByRole("group", { name: "Workspaces" })).toBeNull();
    expect(screen.queryByRole("menuitemradio", { name: "Default" })).toBeNull();
  });

  it("renders the account alone when no workspace matches the scope", async () => {
    const { user } = setup();
    backend.on("workspace_list", () => []);
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    expect(await screen.findByRole("menuitemradio", { name: /Personal/ })).toBeVisible();
  });
});

