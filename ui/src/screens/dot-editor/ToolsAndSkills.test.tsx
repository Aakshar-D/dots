import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import type { EngineKind, LibraryEntry, Uses } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

const [github, pluginServer, pdf, pdfTools, company] = sample.library;
const none: Uses = { mcp_servers: [], skills: [], plugins: [], context: [] };

function setup(opts: { uses?: Uses; engine?: EngineKind; library?: LibraryEntry[] } = {}) {
  backend.on("library_list", () => opts.library ?? sample.library);
  let latest = opts.uses ?? none;
  function Host() {
    const [uses, setUses] = useState(opts.uses ?? none);
    latest = uses;
    return <ToolsAndSkills uses={uses} engine={opts.engine ?? "claude"} onChange={setUses} />;
  }
  render(
    <QueryClientProvider client={createQueryClient()}>
      <MemoryRouter>
        <Host />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return { user: userEvent.setup(), uses: () => latest };
}

describe("the editor's Tools and skills section", () => {
  it("lists the Library in four checklists, without the servers a plugin brings", async () => {
    setup();
    const servers = await screen.findByRole("group", { name: "MCP servers" });
    expect(within(servers).getAllByRole("checkbox").map((c) => c.getAttribute("aria-label"))).toEqual([
      github!.name,
    ]);
    expect(within(screen.getByRole("group", { name: "Skills" })).getByLabelText("pdf")).not.toBeChecked();
    expect(within(screen.getByRole("group", { name: "Plugins" })).getByLabelText("pdf-tools")).not.toBeChecked();
  });

  it("lists the context collections, one for every dot ticked and fixed", async () => {
    const own = { ...company!, id: "01J0CTX2", name: "clients" } as LibraryEntry;
    if (own.kind !== "context") throw new Error("the sample is a context collection");
    own.config = { ...own.config, every_dot: false, description: "Who they are" };
    const { user, uses } = setup({ library: [company!, own] });
    const group = await screen.findByRole("group", { name: "Context" });
    const every = within(group).getByLabelText("company");
    expect(every).toBeChecked();
    expect(every).toBeDisabled();
    expect(within(group).getByText(/42 documents · every dot/)).toBeVisible();
    const box = within(group).getByLabelText("clients");
    expect(box).not.toBeChecked();
    await user.click(box);
    expect(uses().context).toEqual(["clients"]);
  });

  it("says \"at least\" when a collection's count is only a lower bound", async () => {
    if (company!.kind !== "context" || !company!.context) throw new Error("not a collection");
    const partial = { ...company!, context: { ...company!.context, complete: false } } as LibraryEntry;
    setup({ library: [partial] });
    const group = await screen.findByRole("group", { name: "Context" });
    expect(within(group).getByText(/at least 42 documents · every dot/)).toBeVisible();
  });

  it("ticks and unticks, and hands back the names", async () => {
    const { user, uses } = setup({ uses: { ...none, skills: ["pdf"] } });
    expect(await screen.findByLabelText("pdf")).toBeChecked();
    await user.click(screen.getByLabelText(github!.name));
    await user.click(screen.getByLabelText("pdf-tools"));
    await user.click(screen.getByLabelText("pdf"));
    expect(uses()).toEqual({
      mcp_servers: ["github"],
      skills: [],
      plugins: ["pdf-tools"],
      context: [],
    });
  });

  it("keeps a name the Library no longer has, so that it can be unticked", async () => {
    const { user, uses } = setup({ uses: { ...none, skills: ["gone"] } });
    const box = await screen.findByLabelText("gone");
    expect(box).toBeChecked();
    expect(screen.getByText("Not in the Library any more.")).toBeVisible();
    await user.click(box);
    expect(uses().skills).toEqual([]);
  });

  it("says where the Library is empty", async () => {
    setup({ library: [] });
    expect(await screen.findAllByText(/Nothing here yet/)).toHaveLength(4);
    expect(screen.getAllByRole("link", { name: "Library" })[0]).toHaveAttribute("href", "/library");
  });

  it("lists the variables the chosen servers and plugins need and the environment lacks", async () => {
    const needy = { ...pluginServer!, missing_variables: ["PDF_KEY"] } as LibraryEntry;
    const { user } = setup({ library: [github!, needy, pdf!, pdfTools!] });
    await screen.findByLabelText("github");
    expect(screen.queryByText(/Not set in Dots' environment/)).toBeNull();

    // github needs a variable only once it is ticked
    await user.click(screen.getByLabelText("github"));
    expect(screen.getByRole("status")).toHaveTextContent("Not set in Dots' environment: " + github!.missing_variables.join(", "));
    // a plugin's own server counts when its plugin is ticked
    await user.click(screen.getByLabelText("pdf-tools"));
    expect(screen.getByRole("status")).toHaveTextContent("PDF_KEY");
  });

  it("warns about a server whose tools wait for a review", async () => {
    setup();
    const row = (await screen.findByLabelText(github!.name)).closest("li")!;
    expect(row).toHaveTextContent("Tools changed");
  });

  it("tells a local dot which parts of a ticked plugin do not apply", async () => {
    const { user } = setup({ engine: "local" });
    await screen.findByLabelText("pdf-tools");
    expect(screen.queryByText(/do not apply/)).toBeNull();
    await user.click(screen.getByLabelText("pdf-tools"));
    expect(screen.getByText(/1 command, 1 agent and its hooks do not apply/)).toBeVisible();
  });

  it("says nothing of the kind to a Claude dot", async () => {
    setup({ engine: "claude", uses: { ...none, plugins: ["pdf-tools"] } });
    await screen.findByLabelText("pdf-tools");
    expect(screen.queryByText(/do not apply/)).toBeNull();
  });
});
