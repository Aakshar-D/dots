import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview, OrgSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const row = (name: string, org_id: string | null): DotOverview => ({
  ...sample.overview,
  dot: { ...sample.dot, id: `dot-${name}`, name, org_id, schedule: null, enabled: true },
  active_run: null,
  last_run: null,
});
const dots = [row("mine", null), row("builder", "acme-eng"), row("pixels", "design")];
const acme: OrgSummary = { ...sample.orgs[0], id: "acme-eng", name: "Acme Engineering" };
const design: OrgSummary = { ...sample.orgs[0], id: "design", name: "Design" };
/// A scope the fake core keeps, whose switch answers after `delay` ms, as real IPC can.
function slowScope(start: string, delay = 200) {
  const state = { account: start };
  backend.on("scope_get", () => ({ account: state.account, workspace: "default" }));
  backend.on("scope_set", async (args) => {
    await new Promise((r) => setTimeout(r, delay));
    state.account = String(args.account);
    return { account: state.account, workspace: "default" };
  });
  return state;
}
/// Every route the app passes through, from the first.
function routes(router: { state: { location: { pathname: string } }; subscribe: (fn: (s: { location: { pathname: string } }) => void) => unknown }) {
  const seen = [router.state.location.pathname];
  router.subscribe((s) => {
    if (seen.at(-1) !== s.location.pathname) seen.push(s.location.pathname);
  });
  return seen;
}
const settle = (ms = 300) => new Promise((r) => setTimeout(r, ms));
const column = () => screen.getByRole("navigation", { name: "Dots" });

describe("the home follows the current account", () => {
  it("lists only the dots of the current org", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    renderApp("/home");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole("listitem", {
      name: "builder",
    });
    expect(within(column()).queryByRole("listitem", { name: "mine" })).toBeNull();
    expect(within(column()).queryByRole("listitem", { name: "pixels" })).toBeNull();
  });

  it("lists only the dots without an org in Personal", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => [acme, design]);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    renderApp("/home");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole("listitem", {
      name: "mine",
    });
    expect(within(column()).queryByRole("listitem", { name: "builder" })).toBeNull();
    expect(within(column()).queryByRole("listitem", { name: "pixels" })).toBeNull();
  });

  it("lists every dot while the scope loads", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("scope_get", () => new Promise(() => {}));
    renderApp("/home");
    const nav = await screen.findByRole("navigation", { name: "Dots" });
    for (const name of ["mine", "builder", "pixels"]) {
      expect(await within(nav).findByRole("listitem", { name })).toBeVisible();
    }
  });

  it("switches to the account of a dot opened by link, without redirecting", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => [acme, design]);
    const scope = slowScope("personal");
    const { router } = renderApp("/home/dot-builder");
    const seen = routes(router);
    // Settled: the dot's account is current and its conversation is open.
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole(
      "listitem",
      { name: "builder" },
      { timeout: 3000 },
    );
    await waitFor(() => expect(within(column()).queryByRole("listitem", { name: "mine" })).toBeNull());
    expect(await screen.findByLabelText("Message")).toBeVisible();
    await settle();
    expect(backend.callsTo("scope_set")).toEqual([{ account: "acme-eng", workspace: "default" }]);
    expect(scope.account).toBe("acme-eng");
    expect(router.state.location.pathname).toBe("/home/dot-builder");
    // It never left the linked dot for the home on the way.
    expect(seen).toEqual(["/home/dot-builder"]);
  });

  it("stays in the account picked while a dot is open", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => [acme, design]);
    const scope = slowScope("acme-eng");
    const { user, router } = renderApp("/home/dot-builder");
    await screen.findByLabelText("Message");
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Personal/ }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/dot-mine"));
    await within(column()).findByRole("listitem", { name: "mine" });
    await screen.findByLabelText("Message");
    // Settle, then the scope was set once, to the account picked, and stays there.
    await settle();
    expect(backend.callsTo("scope_set")).toEqual([{ account: "personal", workspace: "default" }]);
    expect(scope.account).toBe("personal");
    expect(router.state.location.pathname).toBe("/home/dot-mine");
    expect(within(column()).queryByRole("listitem", { name: "builder" })).toBeNull();
  });

  it("keeps an account picked while a link still waits for the joined accounts", async () => {
    backend.on("dot_overview", () => dots);
    let joined!: (orgs: OrgSummary[]) => void;
    backend.on("org_list", () => new Promise<OrgSummary[]>((r) => (joined = r)));
    const scope = slowScope("design");
    const { user, router } = renderApp("/home/dot-builder");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole("listitem", {
      name: "pixels",
    });
    await user.click(await screen.findByRole("button", { name: /^Account:/ }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Personal/ }));
    // The joined accounts arrive while the pick is on its way: the link must not switch now.
    joined([acme, design]);
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/dot-mine"), {
      timeout: 3000,
    });
    await screen.findByLabelText("Message");
    await settle();
    expect(backend.callsTo("scope_set")).toEqual([{ account: "personal", workspace: "default" }]);
    expect(scope.account).toBe("personal");
    expect(router.state.location.pathname).toBe("/home/dot-mine");
  });

  it("says so when the joined accounts can't be read for a link", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => {
      throw { kind: "internal", message: "the vault is locked" };
    });
    slowScope("personal");
    renderApp("/home/dot-builder");
    expect(await screen.findByText("the vault is locked")).toBeVisible();
    await settle();
    expect(backend.callsTo("scope_set")).toEqual([]);
  });

  it("goes back to the home when the switch for a link is refused", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => [acme, design]);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    backend.on("scope_set", () => {
      throw { kind: "conflict", message: "no such account" };
    });
    const { router } = renderApp("/home/dot-builder");
    await waitFor(() => expect(router.state.location.pathname).toBe("/home/dot-mine"));
    expect(await screen.findByLabelText("Message")).toBeVisible();
    expect(backend.callsTo("scope_set")).toEqual([{ account: "acme-eng", workspace: "default" }]);
  });

  it("lists the dots of a removed account under Personal, and opens them without a switch", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("org_list", () => [design]);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    const { router } = renderApp("/home/dot-builder");
    const card = await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole(
      "listitem",
      { name: "builder" },
    );
    expect(within(card).getByText("account removed")).toBeVisible();
    expect(within(column()).getByRole("listitem", { name: "mine" })).toBeVisible();
    expect(within(column()).queryByRole("listitem", { name: "pixels" })).toBeNull();
    expect(await screen.findByLabelText("Message")).toBeVisible();
    await new Promise((r) => setTimeout(r, 50));
    expect(backend.callsTo("scope_set")).toEqual([]);
    expect(router.state.location.pathname).toBe("/home/dot-builder");
  });
});
