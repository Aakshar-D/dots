import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { Uses } from "@/api/types";
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
}) {
  backend.on("library_list", () => opts.library ?? sample.library);
  backend.on("env_list", () => [token]);
  let latest = opts.variables ?? {};
  function Host() {
    const [variables, setVariables] = useState<Grants>(opts.variables ?? {});
    latest = variables;
    return (
      <Variables uses={opts.uses} variables={variables} before={opts.before} onChange={setVariables} />
    );
  }
  render(
    <QueryClientProvider client={createQueryClient()}>
      <Host />
    </QueryClientProvider>,
  );
  return { user: userEvent.setup(), variables: () => latest };
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
