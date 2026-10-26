import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { EnvEntry, GrantStatus, OrgEntry, OrgSummary, Uses } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { Variables } from "@/screens/dot-editor/Variables";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

type Grants = Record<string, string[]>;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };
const both: Uses = { ...none, mcp_servers: ["github"], plugins: ["pdf-tools"] };
const token = {
  name: "GITHUB_TOKEN",
  kind: "secret" as const,
  value: null,
  created_at: "2026-10-21T12:00:00.000Z",
  updated_at: "2026-10-21T12:00:00.000Z",
  used_by: [],
  value_missing: false,
};

function setup(opts: {
  uses: Uses;
  before: Uses | null;
  variables?: Grants;
  library?: typeof sample.library;
  env?: EnvEntry[];
  /// The dot's org (null or left out: Personal), the joined orgs and the org's entries.
  orgId?: string | null;
  orgs?: OrgSummary[];
  entries?: OrgEntry[];
  /// A stored dot's id, and the status of its grants.
  dotId?: string | null;
  statuses?: GrantStatus[];
}) {
  backend.on("library_list", () => opts.library ?? sample.library);
  backend.on("env_list", () => opts.env ?? [token]);
  backend.on("org_list", () => opts.orgs ?? []);
  backend.on("org_entries", () => opts.entries ?? []);
  backend.on("dot_variables", () => opts.statuses ?? []);
  let latest = opts.variables ?? {};
  function Host() {
    const [variables, setVariables] = useState<Grants>(opts.variables ?? {});
    latest = variables;
    return (
      <Variables
        uses={opts.uses}
        variables={variables}
        before={opts.before}
        orgId={opts.orgId ?? null}
        dotId={opts.dotId ?? null}
        onChange={setVariables}
      />
    );
  }
  const { unmount } = render(
    <QueryClientProvider client={createQueryClient()}>
      <Host />
    </QueryClientProvider>,
  );
  return { user: userEvent.setup(), variables: () => latest, unmount };
}

describe("the editor's Variables section", () => {
  it("says a newly ticked server is granted its variables when the dot is saved", async () => {
    setup({ uses: both, before: null });
    const github = await screen.findByRole("group", { name: "github" });
    expect(within(github).getByText("GITHUB_TOKEN · granted when you save")).toBeVisible();
    // A plugin's server is the plugin's: ticking the plugin uses it.
    const render = screen.getByRole("group", { name: "plugin-pdf-tools-render" });
    expect(within(render).getByText("EXAMPLE_TOKEN · granted when you save")).toBeVisible();
  });

  it("asks before it grants what a server already used asks for now", async () => {
    const { user, variables } = setup({
      uses: both,
      before: both,
      variables: { "mcp:github": ["GITHUB_TOKEN"] },
    });
    const github = await screen.findByRole("group", { name: "github" });
    expect(within(github).getByText("GITHUB_TOKEN")).toBeVisible();
    const render = screen.getByRole("group", { name: "plugin-pdf-tools-render" });
    expect(within(render).getByText(/now also wants/)).toHaveTextContent(
      "plugin-pdf-tools-render now also wants EXAMPLE_TOKEN",
    );
    await user.click(
      within(render).getByRole("button", {
        name: "Grant EXAMPLE_TOKEN to plugin-pdf-tools-render",
      }),
    );
    expect(variables()).toEqual({
      "mcp:github": ["GITHUB_TOKEN"],
      "mcp:plugin-pdf-tools-render": ["EXAMPLE_TOKEN"],
    });
    expect(within(render).getByText(/not in Personal/)).toBeVisible();
  });

  it("lists only the valid variable names a server reads", async () => {
    const long = `A${"B".repeat(128)}`;
    const library = sample.library.map((e) =>
      e.kind === "mcp_server" && e.name === "github" && e.config.transport === "stdio"
        ? { ...e, config: { ...e.config, env: { ...e.config.env, LONG: `\${${long}}` } } }
        : e,
    );
    const variables = { "mcp:github": ["GITHUB_TOKEN"] };
    setup({ uses: { ...none, mcp_servers: ["github"] }, before: both, variables, library });
    const github = await screen.findByRole("group", { name: "github" });
    expect(within(github).getByText("GITHUB_TOKEN")).toBeVisible();
    expect(within(github).queryByText(/now also wants/)).toBeNull();
    expect(within(github).queryByRole("button")).toBeNull();
  });

  it("grants a variable to the shell with a warning, and takes it back", async () => {
    const { user, variables } = setup({ uses: none, before: none });
    const shell = await screen.findByRole("group", { name: "Shell" });
    expect(within(shell).getByText(/could send them anywhere its tools reach/)).toBeVisible();
    await user.type(within(shell).getByLabelText("Variable for the shell"), "1BAD");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(within(shell).getByText(/Use a letter or _/)).toBeVisible();
    expect(variables()).toEqual({});
    await user.clear(within(shell).getByLabelText("Variable for the shell"));
    await user.type(within(shell).getByLabelText("Variable for the shell"), "GITHUB_TOKEN");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(variables()).toEqual({ shell: ["GITHUB_TOKEN"] });
    await user.click(
      within(shell).getByRole("button", { name: "Stop granting GITHUB_TOKEN to the shell" }),
    );
    expect(variables()).toEqual({ shell: [] });
  });
});

const [acme] = sample.orgs as [OrgSummary]; // Acme Engineering, connected, personal: fallback
// GITHUB_TOKEN from the org, LOG_LEVEL from Personal, API_KEY not in the org, PROD_DB_URL
// denied, GITHUB_TOKEN refused to the shell, and LOG_LEVEL of a dot whose org was removed.
const [fromOrg, fromPersonal, notInOrg, denied, forbidden, removed] =
  sample.grantStatuses as GrantStatus[];
const github: Uses = { ...none, mcp_servers: ["github"] };
const inAcme = { orgId: "acme-eng", dotId: "D2", uses: github, before: github };

/// The names the shell's picker offers.
function offered(): string[] {
  const input = within(screen.getByRole("group", { name: "Shell" })).getByLabelText(
    "Variable for the shell",
  );
  const list = document.getElementById(input.getAttribute("list")!)!;
  return [...list.querySelectorAll("option")].map((o) => o.value);
}

describe("the Variables section of a dot in an org", () => {
  it("shows each grant's status in the org: its source, or the core's reason", async () => {
    setup({
      ...inAcme,
      orgs: [acme],
      variables: {
        "mcp:github": ["GITHUB_TOKEN", "API_KEY", "PROD_DB_URL"],
        shell: ["LOG_LEVEL", "GITHUB_TOKEN"],
      },
      statuses: [fromOrg!, fromPersonal!, notInOrg!, denied!, forbidden!],
    });
    const server = await screen.findByRole("group", { name: "github" });
    await waitFor(() =>
      expect(within(server).getByText("GITHUB_TOKEN")).toHaveTextContent(
        "GITHUB_TOKEN · from Acme Engineering",
      ),
    );
    const missing = within(server).getByText(notInOrg!.text!);
    expect(missing).toHaveClass("text-needs");
    expect(within(server).getByText(denied!.text!)).toHaveClass("text-needs");
    const shell = screen.getByRole("group", { name: "Shell" });
    expect(within(shell).getByText("LOG_LEVEL")).toHaveTextContent("LOG_LEVEL · from Personal");
    expect(within(shell).getByText(forbidden!.text!)).toHaveClass("text-needs");
    expect(backend.callsTo("dot_variables")).toEqual([{ dotId: "D2" }]);
  });

  it("shows the grants of a dot whose org was removed in the error tone", async () => {
    setup({ ...inAcme, orgs: [], variables: { shell: ["LOG_LEVEL"] }, statuses: [removed!] });
    const shell = await screen.findByRole("group", { name: "Shell" });
    const text = await within(shell).findByText(removed!.text!);
    expect(text).toHaveTextContent("dot 'acme-release': its org 'acme-eng' was removed");
    expect(text).toHaveClass("text-destructive");
    // Nothing of the org is offered: it is not joined.
    expect(offered()).toEqual([]);
    expect(backend.callsTo("org_entries")).toEqual([]);
  });

  it("says a grant not saved yet is checked when the dot is saved", async () => {
    const { user } = setup({ ...inAcme, orgs: [acme], entries: sample.orgEntries });
    const shell = await screen.findByRole("group", { name: "Shell" });
    await user.type(within(shell).getByLabelText("Variable for the shell"), "API_BASE_URL");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(within(shell).getByText("API_BASE_URL")).toHaveTextContent(
      "API_BASE_URL · checked when you save",
    );
  });

  it("hides the org's secrets from the shell while shell_secrets is false", async () => {
    const keeps = { ...acme, personal_mode: "deny" as const, shell_secrets: false };
    const { user, variables } = setup({ ...inAcme, orgs: [keeps], entries: sample.orgEntries });
    const shell = await screen.findByRole("group", { name: "Shell" });
    await waitFor(() => expect(offered()).toEqual(["API_BASE_URL"]));
    const hint =
      "Acme Engineering does not allow secrets in the shell, so its secrets are not offered here.";
    expect(within(shell).getByText(hint)).toBeVisible();
    // Typed in anyway, it is not granted.
    await user.type(within(shell).getByLabelText("Variable for the shell"), "github_token");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(
      within(shell).getByText(
        "github_token is a secret, and Acme Engineering does not allow secrets in the shell.",
      ),
    ).toBeVisible();
    expect(variables()).toEqual({});
  });

  it("offers the org's secrets to the shell when shell_secrets is true", async () => {
    const lets = { ...acme, personal_mode: "deny" as const, shell_secrets: true };
    setup({ ...inAcme, orgs: [lets], entries: sample.orgEntries });
    await screen.findByRole("group", { name: "Shell" });
    await waitFor(() => expect(offered()).toEqual(["API_BASE_URL", "GITHUB_TOKEN"]));
    expect(screen.queryByText(/does not allow secrets in the shell/)).toBeNull();
  });

  it("adds Personal's names as the org's personal mode allows", async () => {
    // Personal: GITHUB_TOKEN and NPM_TOKEN (secrets), LOG_LEVEL (plain). The org:
    // API_BASE_URL (plain) and GITHUB_TOKEN (secret).
    const mode = (personal_mode: OrgSummary["personal_mode"], shell_secrets = true) => ({
      ...acme,
      personal_mode,
      shell_secrets,
    });
    const cases: [OrgSummary, string[]][] = [
      [mode("deny"), ["API_BASE_URL", "GITHUB_TOKEN"]],
      // No accepted policy reads as deny.
      [mode(null), ["API_BASE_URL", "GITHUB_TOKEN"]],
      [mode("fallback"), ["API_BASE_URL", "GITHUB_TOKEN", "LOG_LEVEL", "NPM_TOKEN"]],
      [mode("override"), ["GITHUB_TOKEN", "LOG_LEVEL", "NPM_TOKEN", "API_BASE_URL"]],
      // A Personal secret is kept from the shell too.
      [mode("fallback", false), ["API_BASE_URL", "LOG_LEVEL"]],
      [mode("override", false), ["LOG_LEVEL", "API_BASE_URL"]],
    ];
    for (const [org, names] of cases) {
      const { unmount } = setup({
        ...inAcme,
        orgs: [org],
        entries: sample.orgEntries,
        env: sample.env,
      });
      await screen.findByRole("group", { name: "Shell" });
      await waitFor(() => expect(offered(), String(org.personal_mode)).toEqual(names));
      unmount();
    }
  });

  it("leaves out the names the org's use rules deny, from either side", async () => {
    // Under fallback the scope has API_BASE_URL and GITHUB_TOKEN (the org), then LOG_LEVEL and
    // NPM_TOKEN (Personal). The globs match without regard to case, and deny wins.
    const rules = (use_rules: OrgSummary["use_rules"]) => ({
      ...acme,
      personal_mode: "fallback" as const,
      shell_secrets: true,
      use_rules,
    });
    const cases: [OrgSummary, string[]][] = [
      [
        rules({ allow: ["*"], deny: [] }),
        ["API_BASE_URL", "GITHUB_TOKEN", "LOG_LEVEL", "NPM_TOKEN"],
      ],
      [rules({ allow: ["*"], deny: ["github_*", "npm*"] }), ["API_BASE_URL", "LOG_LEVEL"]],
      [rules({ allow: ["api_*", "*_TOKEN"], deny: ["GITHUB_*"] }), ["API_BASE_URL", "NPM_TOKEN"]],
      // No accepted policy: nothing may be granted.
      [rules(null), []],
    ];
    for (const [org, names] of cases) {
      const { unmount } = setup({
        ...inAcme,
        orgs: [org],
        entries: sample.orgEntries,
        env: sample.env,
      });
      await screen.findByRole("group", { name: "Shell" });
      await waitFor(() => expect(offered(), JSON.stringify(org.use_rules)).toEqual(names));
      unmount();
    }
    // A denied name typed in by hand is still granted; its status then says why it is not set.
    const { user, variables } = setup({
      ...inAcme,
      orgs: [rules({ allow: ["*"], deny: ["NPM_*"] })],
      entries: sample.orgEntries,
      env: sample.env,
    });
    const shell = await screen.findByRole("group", { name: "Shell" });
    await waitFor(() => expect(offered()).toEqual(["API_BASE_URL", "GITHUB_TOKEN", "LOG_LEVEL"]));
    await user.type(within(shell).getByLabelText("Variable for the shell"), "NPM_TOKEN");
    await user.click(within(shell).getByRole("button", { name: "Grant to the shell" }));
    expect(variables()).toEqual({ shell: ["NPM_TOKEN"] });
  });
});

describe("the Variables section of a Personal dot", () => {
  it("offers every Personal name, secrets too, and asks for no org", async () => {
    setup({ uses: none, before: none, env: sample.env, dotId: "D1" });
    await screen.findByRole("group", { name: "Shell" });
    await waitFor(() => expect(offered()).toEqual(["GITHUB_TOKEN", "LOG_LEVEL", "NPM_TOKEN"]));
    expect(screen.queryByText(/does not allow secrets/)).toBeNull();
    expect(backend.callsTo("org_list")).toEqual([]);
    expect(backend.callsTo("org_entries")).toEqual([]);
  });

  it("shows only what Personal lacks, as before, from the core's status", async () => {
    const lacks: GrantStatus = {
      consumer: "shell",
      name: "NPM_TOKEN",
      status: "not_defined",
      text: "NPM_TOKEN: not in Personal (dot 'nightly-tests', shell)",
      source: null,
      secret: false,
    };
    const resolves: GrantStatus = {
      ...lacks,
      name: "LOG_LEVEL",
      status: "ok",
      text: null,
      source: "personal",
    };
    setup({
      uses: none,
      before: none,
      env: sample.env,
      dotId: "D1",
      variables: { shell: ["LOG_LEVEL", "NPM_TOKEN"] },
      statuses: [resolves, lacks],
    });
    const shell = await screen.findByRole("group", { name: "Shell" });
    // NPM_TOKEN is listed in Personal, but its value is gone: the core's status says so.
    await waitFor(() =>
      expect(within(shell).getByText("NPM_TOKEN")).toHaveTextContent("NPM_TOKEN · not in Personal"),
    );
    expect(within(shell).getByText("LOG_LEVEL")).toHaveTextContent(/^LOG_LEVEL$/);
    expect(within(shell).queryByText(/from Personal/)).toBeNull();
  });
});
