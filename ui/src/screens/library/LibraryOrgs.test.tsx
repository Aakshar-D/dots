import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { EnvEntry, OrgEntry, OrgSummary } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import type { ServerEntry } from "./parts";

const [github, owned] = sample.library as [ServerEntry, ServerEntry];
const [acme] = sample.orgs; // Acme Engineering, connected
const revoked = sample.orgs.find((o) => o.status === "revoked")!;
const beta: OrgSummary = { ...revoked, id: "beta", name: "Beta Labs" };
const gamma: OrgSummary = { ...acme, id: "gamma", name: "Gamma" };
// GITHUB_TOKEN (nightly-tests grants it), LOG_LEVEL (not granted), NPM_TOKEN (value gone).
const [token, , npm] = sample.env as [EnvEntry, EnvEntry, EnvEntry];
// API_BASE_URL (plain, locked, acme-release grants it) and GITHUB_TOKEN (secret, not locked).
const [baseUrl, orgToken] = sample.orgEntries as [OrgEntry, OrgEntry];
// What a secret's value would look like if one ever reached the screen. Made up.
const LEAK = "ui-test-leaked-secret-6a1f0c";
const cancelled = { kind: "cancelled", message: "declined: nothing was sent or saved" };

/// Opens the Env tab with Personal's `env`, the joined `orgs` and each org's entries.
async function envTab(
  opts: { env?: EnvEntry[]; orgs?: OrgSummary[]; entries?: Record<string, OrgEntry[]> } = {},
) {
  backend.on("env_list", () => opts.env ?? sample.env);
  backend.on("org_list", () => opts.orgs ?? [acme]);
  backend.on("org_entries", (args) => opts.entries?.[args.orgId as string] ?? sample.orgEntries);
  const rendered = renderApp("/library");
  await rendered.user.click(await screen.findByRole("tab", { name: "Env" }));
  return rendered;
}

const section = (name: string) => screen.findByRole("region", { name });
const row = async (scope: string, name: string) =>
  within(await section(scope)).findByRole("listitem", { name });

describe("the Env tab's orgs", () => {
  it("shows Personal, then each org with its status and read-only rows it manages", async () => {
    await envTab();
    const org = await section("Acme Engineering");
    expect(within(org).getByRole("heading", { name: "Acme Engineering" })).toBeVisible();
    expect(within(org).getByText(acme.status_text)).toBeVisible();
    const names = within(screen.getByRole("tabpanel"))
      .getAllByRole("region")
      .map((r) => r.getAttribute("aria-label"));
    expect(names).toEqual(["Personal", "Acme Engineering"]);

    const plain = await row("Acme Engineering", "API_BASE_URL");
    expect(within(plain).getByText("plain")).toBeVisible();
    expect(within(plain).getByText(baseUrl.value!)).toBeVisible();
    expect(within(plain).getByText("Locked")).toBeVisible();
    expect(within(plain).getByText("Managed by Acme Engineering")).toBeVisible();
    expect(within(plain).queryAllByRole("button")).toEqual([]);

    const secret = await row("Acme Engineering", orgToken.name);
    expect(within(secret).getByText("secret")).toBeVisible();
    // Not locked: a Personal variable of that name takes its place.
    expect(within(secret).queryByText("Locked")).toBeNull();
    expect(within(secret).getByText("Managed by Acme Engineering")).toBeVisible();
    expect(within(secret).queryAllByRole("button")).toEqual([]);
    expect(backend.callsTo("org_entries")).toEqual([{ orgId: "acme-eng" }]);
  });

  it("never shows a secret's value, in Personal or in an org", async () => {
    await envTab({
      env: [{ ...token, value: LEAK }],
      entries: { "acme-eng": [{ ...orgToken, value: LEAK }] },
    });
    expect(await row("Acme Engineering", orgToken.name)).toBeVisible();
    expect(await row("Personal", token.name)).toBeVisible();
    expect(document.body.textContent).not.toContain(LEAK);
  });

  it("shows the status of an org that gives no values, and no rows for it", async () => {
    await envTab({ orgs: [acme, beta], entries: { beta: [] } });
    const org = await section("Beta Labs");
    const status = await within(org).findByText(beta.status_text);
    expect(status).toHaveClass("text-destructive");
    await waitFor(() => expect(backend.callsTo("org_entries")).toHaveLength(2));
    expect(within(org).queryAllByRole("listitem")).toEqual([]);
  });

  it("names the dots that grant each variable, in its own scope", async () => {
    await envTab();
    expect(
      within(await row("Personal", "GITHUB_TOKEN")).getByText("Used by nightly-tests"),
    ).toBeVisible();
    expect(within(await row("Personal", "LOG_LEVEL")).getByText("Not used")).toBeVisible();
    expect(
      within(await row("Acme Engineering", "API_BASE_URL")).getByText("Used by acme-release"),
    ).toBeVisible();
    const secret = await row("Acme Engineering", "GITHUB_TOKEN");
    expect(within(secret).getByText("Not used")).toBeVisible();
  });

  it("marks a secret whose value is gone, and sets it again with the edit dialog", async () => {
    let env = sample.env;
    backend.on("env_set", () => {
      env = env.map((e) => (e.name === npm.name ? { ...e, value_missing: false } : e));
      return { ...npm, value_missing: false };
    });
    const { user } = await envTab();
    backend.on("env_list", () => env);
    const gone = await row("Personal", "NPM_TOKEN");
    expect(within(gone).getByText("value missing")).toHaveClass("text-needs");
    expect(within(await row("Personal", "GITHUB_TOKEN")).queryByText("value missing")).toBeNull();

    await user.click(within(gone).getByRole("button", { name: "Set value" }));
    const dialog = await screen.findByRole("dialog", { name: "Set NPM_TOKEN" });
    expect(within(dialog).getByLabelText("Value")).toHaveAttribute("type", "password");
    await user.type(within(dialog).getByLabelText("Value"), "ui-test-npm-value-2d4b");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.callsTo("env_set")).toEqual([
      { name: "NPM_TOKEN", value: "ui-test-npm-value-2d4b" },
    ]);
    await waitFor(async () =>
      expect(within(await row("Personal", "NPM_TOKEN")).queryByText("value missing")).toBeNull(),
    );
  });

  it("reads an org's entries again when the org is updated", async () => {
    const { listening } = await envTab();
    await row("Acme Engineering", "API_BASE_URL");
    await listening();
    const before = backend.callsTo("org_entries").length;
    backend.on("org_entries", () => [{ ...baseUrl, value: "https://api2.acme.example" }]);
    act(() => backend.emit("runtime", { type: "org_updated", org_id: "acme-eng" }));
    expect(await screen.findByText("https://api2.acme.example")).toBeVisible();
    expect(backend.callsTo("org_entries").length).toBeGreaterThan(before);
  });
});

/// github with a changed list in Personal and in Acme Engineering (used by acme-release), never
/// checked in Beta Labs and used by none of its dots, and never checked in Gamma, used there.
/// `reset`: Beta Labs' list was stored once and reset to unchecked since (a server change).
function scoped(reset = false): ServerEntry {
  const personal = github.catalogs[0]!;
  const changed = { ...personal.catalog, seen_fingerprint: "fp-acme" };
  const unchecked = {
    ...personal.catalog,
    status: "unchecked" as const,
    accepted_fingerprint: null,
    accepted_tools: null,
    seen_fingerprint: null,
    seen_tools: null,
    checked_at: null,
  };
  return {
    ...github,
    catalogs: [
      personal,
      {
        scope: "acme-eng",
        catalog: changed,
        stored: true,
        used_by: [{ id: "D2", name: "acme-release" }],
      },
      { scope: "beta", catalog: unchecked, stored: reset, used_by: [] },
      {
        scope: "gamma",
        catalog: unchecked,
        stored: false,
        used_by: [{ id: "D3", name: "gamma-sync" }],
      },
    ],
  };
}

describe("MCP servers in each scope", () => {
  it("lists a server once, with its state in each org that checked it or uses it", async () => {
    backend.on("library_list", () => [scoped()]);
    backend.on("org_list", () => [acme, beta, gamma]);
    renderApp("/library");
    const server = await screen.findByRole("listitem", { name: "github" });
    expect(screen.getAllByRole("listitem", { name: "github" })).toHaveLength(1);
    const inAcme = await within(server).findByRole("listitem", {
      name: "github in Acme Engineering",
    });
    expect(within(inAcme).getByText("Tools changed")).toBeVisible();
    expect(within(inAcme).getByText("Used by acme-release")).toBeVisible();
    expect(within(inAcme).getByRole("button", { name: "Review" })).toBeVisible();
    const inGamma = within(server).getByRole("listitem", { name: "github in Gamma" });
    expect(within(inGamma).getByText("Unchecked")).toBeVisible();
    expect(within(inGamma).queryByRole("button", { name: "Review" })).toBeNull();
    expect(within(server).queryByRole("listitem", { name: "github in Beta Labs" })).toBeNull();
  });

  it("keeps an org whose stored list was reset, though no dot of it uses the server", async () => {
    backend.on("library_list", () => [scoped(true)]);
    backend.on("org_list", () => [acme, beta, gamma]);
    renderApp("/library");
    const server = await screen.findByRole("listitem", { name: "github" });
    const inBeta = await within(server).findByRole("listitem", { name: "github in Beta Labs" });
    expect(within(inBeta).getByText("Unchecked")).toBeVisible();
    expect(within(inBeta).queryByText(/Used by/)).toBeNull();
  });

  it("keeps the Personal row as it was: its review accepts with no scope", async () => {
    backend.on("library_list", () => [scoped()]);
    backend.on("org_list", () => [acme]);
    backend.on("library_accept", () => github.catalog);
    const { user } = renderApp("/library");
    const server = await screen.findByRole("listitem", { name: "github" });
    await within(server).findByRole("listitem", { name: "github in Acme Engineering" });
    // The server's own Review, not the one in its Acme Engineering line.
    const review = within(server)
      .getAllByRole("button", { name: "Review" })
      .find((button) => button.closest("li") === server)!;
    await user.click(review);
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    await waitFor(() => expect(backend.callsTo("library_accept")).toHaveLength(1));
    const [args] = backend.callsTo("library_accept");
    expect(args).toEqual({ id: github.id, fingerprint: "fp-seen" });
    expect(args!.scope).toBeUndefined();
  });

  it("reviews an org's list under the org's name and accepts it in that org", async () => {
    backend.on("library_list", () => [scoped()]);
    backend.on("org_list", () => [acme]);
    backend.on("library_accept", () => github.catalog);
    const { user } = renderApp("/library");
    const inAcme = await screen.findByRole("listitem", { name: "github in Acme Engineering" });
    await user.click(within(inAcme).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Tools changed: github in Acme Engineering",
    });
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    await waitFor(() =>
      expect(backend.callsTo("library_accept")).toEqual([
        { id: github.id, fingerprint: "fp-acme", scope: "acme-eng" },
      ]),
    );
  });

  it("names the org in the review the Inbox's link opens", async () => {
    backend.on("library_list", () => [scoped()]);
    backend.on("org_list", () => [acme]);
    renderApp(`/library?review=${github.id}&scope=acme-eng`);
    expect(
      await screen.findByRole("dialog", { name: "Tools changed: github in Acme Engineering" }),
    ).toBeVisible();
  });
});

/// An http server of the person's own whose URL reads a variable, so its origin is pinned:
/// confirmed in Personal, not yet in Acme Engineering.
const pinned: ServerEntry = {
  ...owned,
  id: "01J0PIN",
  name: "tickets",
  owner_id: null,
  config: { transport: "http", url: "https://${TICKETS_HOST}/mcp", headers: {} },
  confirmed_origins: { personal: ["https://tickets.example.com"] },
};

async function openForm(entry: ServerEntry, button = "Edit") {
  backend.on("library_list", () => [entry]);
  backend.on("org_list", () => [acme]);
  const view = renderApp("/library");
  const item = await screen.findByRole("listitem", { name: entry.name });
  await view.user.click(within(item).getByRole("button", { name: button }));
  const dialog = await screen.findByRole("dialog", { name: new RegExp(entry.name) });
  return { ...view, dialog };
}

describe("the server form's Check as and origins", () => {
  it("checks as Personal or as an org", async () => {
    backend.on("library_check", () => ({ ...github.catalog!, status: "accepted" }));
    const { user, dialog } = await openForm(github);
    const picker = within(dialog).getByLabelText("Check as");
    await waitFor(() =>
      expect(within(picker).getAllByRole("option").map((o) => o.textContent)).toEqual([
        "Personal",
        "Acme Engineering",
      ]),
    );
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    expect(await within(dialog).findByRole("status")).toHaveTextContent("Checked as Personal");
    await user.selectOptions(picker, "acme-eng");
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    expect(
      await within(dialog).findByText(/Checked as Acme Engineering/),
    ).toBeVisible();
    const calls = backend.callsTo("library_check");
    expect(calls).toEqual([{ id: github.id }, { id: github.id, scope: "acme-eng" }]);
    expect(calls[0]!.scope).toBeUndefined();
  });

  it("says nothing was checked when the origin is not confirmed, and changes nothing", async () => {
    backend.on("library_check", () => {
      throw cancelled;
    });
    const { user, dialog, listening } = await openForm(pinned);
    await listening();
    const lists = backend.callsTo("library_list").length;
    await user.selectOptions(within(dialog).getByLabelText("Check as"), "acme-eng");
    await user.click(within(dialog).getByRole("button", { name: "Check" }));
    expect(
      await within(dialog).findByText("Not checked: you did not confirm the origin."),
    ).toBeVisible();
    expect(backend.callsTo("library_check")).toEqual([{ id: pinned.id, scope: "acme-eng" }]);
    expect(backend.callsTo("library_list")).toHaveLength(lists);
  });

  it("shows the confirmed origins per scope and confirms one in an org", async () => {
    backend.on("confirm_origin", () => null);
    const { user, dialog } = await openForm(pinned);
    const origins = within(dialog).getByRole("list", { name: "Confirmed origins" });
    expect(
      within(origins).getByRole("listitem", { name: "Personal" }),
    ).toHaveTextContent("https://tickets.example.com");
    expect(
      await within(origins).findByRole("listitem", { name: "Acme Engineering" }),
    ).toHaveTextContent("None yet");
    // Personal has one: only the org is offered.
    expect(within(dialog).queryByRole("button", { name: "Confirm origin in Personal" })).toBeNull();
    await user.click(
      within(dialog).getByRole("button", { name: "Confirm origin in Acme Engineering" }),
    );
    await waitFor(() =>
      expect(backend.callsTo("confirm_origin")).toEqual([
        { serverId: pinned.id, scope: "acme-eng" },
      ]),
    );
  });

  it("offers a plugin's own server the same, with its settings read-only", async () => {
    backend.on("confirm_origin", () => {
      throw cancelled;
    });
    const fromPlugin = { ...pinned, owner_id: owned.owner_id, confirmed_origins: {} };
    const { user, dialog } = await openForm(fromPlugin, "Details");
    expect(within(dialog).getByLabelText("URL")).toBeDisabled();
    expect(within(dialog).queryByRole("button", { name: "Save" })).toBeNull();
    expect(within(dialog).getByLabelText("Check as")).toBeEnabled();
    await user.click(within(dialog).getByRole("button", { name: "Confirm origin in Personal" }));
    expect(
      await within(dialog).findByText("You declined. Nothing was confirmed."),
    ).toBeVisible();
    expect(backend.callsTo("confirm_origin")).toEqual([{ serverId: fromPlugin.id }]);
  });

  it("offers no confirm for a server whose URL is fixed", async () => {
    const { dialog } = await openForm({ ...pinned, config: owned.config, confirmed_origins: {} });
    expect(within(dialog).getByLabelText("Check as")).toBeVisible();
    expect(within(dialog).queryByRole("button", { name: /Confirm origin/ })).toBeNull();
  });
});
