import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { AgentImport, ScannedAgent } from "@/api/types";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const agent = (name: string, over: Partial<ScannedAgent> = {}): ScannedAgent => ({
  source: "user",
  plugin: null,
  project: null,
  path: `C:/Users/u/.claude/agents/${name}.md`,
  name,
  description: `The ${name} agent`,
  model: "sonnet",
  tools: null,
  warnings: [],
  already_imported: false,
  error: null,
  ...over,
});

const reviewer = agent("reviewer", {
  tools: ["Read", "Bash", "Frob"],
  warnings: ["Frob is left out: Sage does not know this tool."],
  model: "opus",
});
const tester = agent("tester", {
  source: "project",
  project: "C:/code/app",
  path: "C:/code/app/.claude/agents/tester.md",
  already_imported: true,
});
const pdf = agent("pdf", {
  source: "plugin",
  plugin: "pdf-tools",
  path: "C:/plugins/pdf-tools/agents/pdf.md",
});
const broken = agent("empty", { error: "it has no instructions", description: null });

async function open(agents: ScannedAgent[]) {
  backend.on("scan_claude_agents", () => agents);
  const view = renderApp("/home");
  await view.user.click(await screen.findByRole("button", { name: "Import from Claude Code" }));
  const dialog = await screen.findByRole("dialog", { name: "Import from Claude Code" });
  await within(dialog).findByRole("group", { name: "Your agents" });
  return { ...view, dialog: within(dialog) };
}

describe("importing Claude Code agents", () => {
  it("lists the agents by where they live, with what each would become", async () => {
    const { dialog } = await open([reviewer, broken, tester, pdf]);
    const user = within(dialog.getByRole("group", { name: "Your agents" }));
    const project = within(dialog.getByRole("group", { name: "Project C:/code/app" }));
    const plugin = within(dialog.getByRole("group", { name: "Plugin pdf-tools" }));

    const box = user.getByRole("checkbox", { name: "reviewer" });
    expect(box).toBeChecked();
    expect(user.getByText("The reviewer agent")).toBeVisible();
    expect(user.getByText("Model opus · Tools Read, Bash, Frob")).toBeVisible();
    expect(user.getByText("Frob is left out: Sage does not know this tool.")).toBeVisible();
    // A file that can't be imported says why and can't be ticked.
    expect(user.queryByRole("checkbox", { name: "empty" })).toBeNull();
    expect(user.getByText("it has no instructions")).toBeVisible();
    // An agent that is a dot already starts unticked and says so.
    expect(project.getByRole("checkbox", { name: "tester" })).not.toBeChecked();
    expect(project.getByText("Already a sol")).toBeVisible();
    expect(plugin.getByRole("checkbox", { name: "pdf" })).toBeChecked();
    expect(plugin.getByText("Model sonnet · All tools")).toBeVisible();
  });

  it("imports the ticked agents into the account in view and lists what happened", async () => {
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "platform" }));
    const made: AgentImport = {
      created: [{ ...sample.dot, id: "d-rev", name: "reviewer" }],
      failed: [{ path: pdf.path, reason: "conflict: a sol named \"pdf\" already exists" }],
    };
    backend.on("import_claude_agents", () => made);
    const { dialog, user } = await open([reviewer, tester, pdf]);
    await user.click(dialog.getByRole("checkbox", { name: "tester" }));
    await user.click(dialog.getByRole("checkbox", { name: "tester" }));
    await user.click(dialog.getByRole("button", { name: "Import 2 agents" }));
    await waitFor(() =>
      expect(backend.callsTo("import_claude_agents")).toEqual([
        { paths: [reviewer.path, pdf.path], orgId: "acme-eng", workspaceId: "platform" },
      ]),
    );
    expect(await dialog.findByText("Made reviewer")).toBeVisible();
    expect(dialog.getByText(`${pdf.path}: conflict: a sol named "pdf" already exists`)).toBeVisible();
  });

  it("keeps imported agents unticked and waits for the new scan before importing again", async () => {
    backend.on("import_claude_agents", () => ({
      created: [{ ...sample.dot, id: "d-rev", name: "reviewer" }],
      failed: [],
    }));
    const { dialog, user } = await open([reviewer]);
    // The scan after the import is slow, and still shows the agent as not imported.
    const rescan = later<ScannedAgent[]>();
    backend.on("scan_claude_agents", () => rescan.answer);
    await user.click(dialog.getByRole("button", { name: "Import 1 agent" }));
    expect(await dialog.findByText("Made reviewer")).toBeVisible();
    expect(dialog.getByRole("checkbox", { name: "reviewer" })).not.toBeChecked();
    expect(dialog.getByRole("button", { name: "Import 0 agents" })).toBeDisabled();
    await user.click(dialog.getByRole("checkbox", { name: "reviewer" }));
    expect(dialog.getByRole("button", { name: "Import 1 agent" })).toBeDisabled();
    rescan.give([{ ...reviewer, already_imported: true }]);
    await waitFor(() =>
      expect(dialog.getByRole("button", { name: "Import 1 agent" })).toBeEnabled(),
    );
    expect(backend.callsTo("import_claude_agents")).toHaveLength(1);
  });

  it("says when nothing was found", async () => {
    backend.on("scan_claude_agents", () => []);
    const view = renderApp("/home");
    await view.user.click(await screen.findByRole("button", { name: "Import from Claude Code" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Import from Claude Code" }));
    expect(await dialog.findByText(/No Claude Code agents were found/)).toBeVisible();
    expect(dialog.getByRole("button", { name: "Import 0 agents" })).toBeDisabled();
  });
});
