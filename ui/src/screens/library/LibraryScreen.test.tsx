import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { LibraryEntry } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

import { nameFromFolder, validName } from "./ContextTab";

const [github, owned, skill, plugin, company] = sample.library as [
  LibraryEntry,
  LibraryEntry,
  LibraryEntry,
  LibraryEntry,
  LibraryEntry,
];
const cancelled = { kind: "cancelled", message: "declined: nothing was run or saved" };

const tool = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  description: `${name} tool`,
  inputSchema: { type: "object" },
  ...extra,
});

/// A server whose list changed: `seen` has one tool more, one less and one changed.
function changedServer(): LibraryEntry {
  const catalog = {
    ...github.catalog!,
    status: "changed" as const,
    accepted_tools: [tool("keep"), tool("gone"), tool("edit")],
    seen_fingerprint: "fp-seen",
    seen_tools: [
      tool("keep"),
      tool("edit", { description: "other words" }),
      tool("fresh", { annotations: { readOnlyHint: true } }),
    ],
  };
  return { ...github, catalog };
}

function listing(entries: LibraryEntry[]) {
  backend.on("library_list", () => entries);
}

describe("the Library screen", () => {
  it("has a place in the sidebar between Dots and Inbox", async () => {
    renderApp("/library");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const labels = within(nav)
      .getAllByRole("link")
      .map((l) => l.textContent?.replace(/\d+$/, ""));
    expect(labels.slice(0, 3)).toEqual(["Dots", "Library", "Inbox"]);
    expect(await screen.findByRole("heading", { name: "Library" })).toBeVisible();
  });

  it("lists the servers with their status, their dots and the variables they miss", async () => {
    listing([github, owned, plugin]);
    renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    expect(within(row).getByText("Tools changed")).toBeVisible();
    expect(within(row).getByText("stdio")).toBeVisible();
    expect(within(row).getByText("Missing variable: GITHUB_TOKEN")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
    const fromPlugin = screen.getByRole("listitem", { name: "plugin-pdf-tools-render" });
    expect(within(fromPlugin).getByText("Accepted")).toBeVisible();
    expect(within(fromPlugin).getByText("http")).toBeVisible();
    expect(within(fromPlugin).getByText("Comes with plugin pdf-tools")).toBeVisible();
    // A plugin's server changes with its plugin only.
    expect(within(fromPlugin).queryByRole("button", { name: "Edit" })).toBeNull();
    expect(within(fromPlugin).queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("shows an unreachable server's error and an unchecked server", async () => {
    const down = {
      ...github,
      missing_variables: [],
      catalog: {
        ...github.catalog!,
        status: "unreachable" as const,
        error: "could not start: nope",
      },
    };
    const fresh = { ...github, id: "x", name: "fresh", missing_variables: [], catalog: null };
    listing([down, fresh]);
    renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    expect(within(row).getByText("Unreachable")).toBeVisible();
    expect(within(row).getByText("could not start: nope")).toBeVisible();
    expect(
      within(screen.getByRole("listitem", { name: "fresh" })).getByText("Unchecked"),
    ).toBeVisible();
  });

  it("checks a server now", async () => {
    listing([github]);
    backend.on("library_check", () => github.catalog);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Check now" }));
    await waitFor(() => expect(backend.callsTo("library_check")).toEqual([{ id: github.id }]));
  });

  it("reloads the list when the core says the Library changed", async () => {
    listing([]);
    renderApp("/library");
    expect(await screen.findByText("No MCP servers yet.")).toBeVisible();
    listing([github]);
    act(() => backend.emit("runtime", { type: "library_updated", item_id: github.id }));
    expect(await screen.findByRole("listitem", { name: "github" })).toBeVisible();
  });

  it("shows a failed load", async () => {
    backend.on("library_list", () => {
      throw { kind: "internal", message: "no database" };
    });
    renderApp("/library");
    expect(await screen.findByRole("alert")).toHaveTextContent("no database");
  });
});

describe("adding a server", () => {
  async function open() {
    const view = renderApp("/library");
    await view.user.click(await screen.findByRole("button", { name: "Add server" }));
    return { ...view, dialog: await screen.findByRole("dialog", { name: "Add MCP server" }) };
  }

  it("tests the connection, shows the tools, and saves accepting what was shown", async () => {
    backend.on("library_probe", () => ({
      fingerprint: "fp-1",
      tools: [tool("read_it", { annotations: { readOnlyHint: true } }), tool("write_it")],
    }));
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.type(within(dialog).getByLabelText("Arguments"), "-y{Enter}some-server");
    await user.type(within(dialog).getByLabelText("Environment"), "MODE=readonly");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));

    const config = {
      kind: "mcp_server",
      config: {
        transport: "stdio",
        command: "npx",
        args: ["-y", "some-server"],
        cwd: null,
        env: { MODE: "readonly" },
      },
    };
    expect(await within(dialog).findByText("read_it")).toBeVisible();
    expect(within(dialog).getByText("write_it tool")).toBeVisible();
    expect(within(dialog).getAllByText("read-only")).toHaveLength(1);
    expect(backend.callsTo("library_probe")).toEqual([{ config: config.config }]);

    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([{ name: "files", config, accept: "fp-1" }]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("forgets the test when the config changes, so a save accepts nothing it did not show", async () => {
    backend.on("library_probe", () => ({ fingerprint: "fp-1", tools: [tool("a")] }));
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText("a tool")).toBeVisible();
    await user.type(within(dialog).getByLabelText("Command"), "x");
    expect(within(dialog).queryByText("a tool")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("library_add")[0]).toMatchObject({ accept: null }));
  });

  it("saves a server that could not be reached, unchecked", async () => {
    backend.on("library_probe", () => {
      throw { kind: "internal", message: "could not start: SF_TOKEN is not set" };
    });
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user, dialog } = await open();
    await user.selectOptions(within(dialog).getByLabelText("Transport"), "http");
    await user.type(within(dialog).getByLabelText("Name"), "sf");
    await user.type(within(dialog).getByLabelText("URL"), "https://example.com/mcp");
    await user.type(within(dialog).getByLabelText("Headers"), "Authorization: Bearer ${{T}");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText("could not start: SF_TOKEN is not set")).toBeVisible();
    expect(within(dialog).getByText(/saved without its tools/i)).toBeVisible();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "sf",
          config: {
            kind: "mcp_server",
            config: {
              transport: "http",
              url: "https://example.com/mcp",
              headers: { Authorization: "Bearer ${T}" },
            },
          },
          accept: null,
        },
      ]),
    );
  });

  it("names the variables that Dots' environment does not set, and warns about a pasted secret", async () => {
    backend.on("environment_variables", (args) =>
      (args.names as string[]).map((name) => ({ name, set: name !== "NOT_SET" })),
    );
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Command"), "run");
    await user.type(
      within(dialog).getByLabelText("Environment"),
      "A=${{NOT_SET}{Enter}B=${{IS_SET}{Enter}API_TOKEN=abc123",
    );
    expect(await within(dialog).findByText("Not set in Dots' environment: NOT_SET")).toBeVisible();
    expect(within(dialog).getByText(/API_TOKEN looks like a secret.*\$\{VAR\}/)).toBeVisible();
    expect(within(dialog).getByText(/reads the variable from Dots' environment/)).toBeVisible();
  });

  it("checks the form before it asks the core", async () => {
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "Bad Name");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      within(dialog).getByText(
        'Use 1 to 40 of a-z, 0-9, _ and -, with no "__" and no "_" at the start or end. "dots" is reserved.',
      ),
    ).toBeVisible();
    expect(within(dialog).getByText("Enter the command to run.")).toBeVisible();
    await user.type(within(dialog).getByLabelText("Environment"), "no-equals");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("Line 1 needs the form KEY=VALUE.")).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("takes a refusal in the native dialog calmly, and a core error where it happened", async () => {
    backend.on("library_probe", () => {
      throw cancelled;
    });
    backend.on("library_add", () => {
      throw { kind: "conflict", message: 'the mcp server "files" already exists' };
    });
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    await user.type(within(dialog).getByLabelText("Command"), "npx");
    await user.click(within(dialog).getByRole("button", { name: "Test connection" }));
    expect(await within(dialog).findByText(/You declined. Nothing was tested./)).toBeVisible();
    expect(screen.queryByText(/declined: nothing/)).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
    expect(screen.getByRole("dialog", { name: "Add MCP server" })).toBeVisible();
  });

  it("edits a server: the name stays, the config goes to the core, a refusal keeps the form", async () => {
    listing([github]);
    backend.on("library_update", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit github" });
    expect(within(dialog).getByLabelText("Name")).toBeDisabled();
    expect(within(dialog).getByLabelText("Command")).toHaveValue("npx");
    expect(within(dialog).getByLabelText("Arguments")).toHaveValue(
      "-y\n@modelcontextprotocol/server-github",
    );
    expect(within(dialog).getByLabelText("Environment")).toHaveValue(
      "GITHUB_TOKEN=${GITHUB_TOKEN}",
    );
    await user.type(within(dialog).getByLabelText("Working folder"), "C:\\work");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: github.id,
          config: {
            kind: "mcp_server",
            config: { ...github.config, cwd: "C:\\work" },
          },
        },
      ]),
    );
    expect(await within(dialog).findByText(/You declined. Nothing was saved./)).toBeVisible();
    expect(screen.getByRole("dialog", { name: "Edit github" })).toBeVisible();
  });
});

describe("the review of a changed tool list", () => {
  it("shows what was added, removed and changed, and accepts the list that was shown", async () => {
    listing([changedServer()]);
    backend.on("library_accept", () => ({ ...github.catalog!, status: "accepted" }));
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    expect(within(dialog).getByText("Added")).toBeVisible();
    expect(within(dialog).getByText("fresh")).toBeVisible();
    expect(within(dialog).getByText("read-only")).toBeVisible();
    expect(within(dialog).getByText("Removed")).toBeVisible();
    expect(within(dialog).getByText("gone")).toBeVisible();
    expect(within(dialog).getByText("Changed")).toBeVisible();
    expect(within(dialog).getByText("edit")).toBeVisible();
    expect(within(dialog).getByText("description changed")).toBeVisible();
    expect(within(dialog).queryByText("keep")).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    await waitFor(() =>
      expect(backend.callsTo("library_accept")).toEqual([
        { id: github.id, fingerprint: "fp-seen" },
      ]),
    );
  });

  it("says so when the list moved since it was shown", async () => {
    listing([changedServer()]);
    backend.on("library_accept", () => {
      throw { kind: "conflict", message: "the tool list changed again" };
    });
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    await user.click(within(dialog).getByRole("button", { name: "Accept" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("changed again");
  });

  it("opens from the Inbox's link and for a server that was never accepted", async () => {
    const changed = changedServer();
    listing([{ ...changed, catalog: { ...changed.catalog!, accepted_tools: null } }]);
    renderApp(`/library?review=${github.id}`);
    const dialog = await screen.findByRole("dialog", { name: "Tools changed: github" });
    expect(within(dialog).getByText("fresh")).toBeVisible();
    expect(within(dialog).getByText("keep")).toBeVisible();
  });
});

describe("removing an item", () => {
  it("asks first and names the dots, then removes it from them", async () => {
    listing([github]);
    backend.on("library_remove", () => null);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("nightly-tests");
    expect(backend.callsTo("library_remove")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));
    await waitFor(() =>
      expect(backend.callsTo("library_remove")).toEqual([{ id: github.id, confirm: true }]),
    );
  });

  it("does not claim dots use an item no dot uses", async () => {
    listing([{ ...github, used_by: [] }]);
    backend.on("library_remove", () => null);
    const { user } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("No dot uses it.");
    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));
    await waitFor(() =>
      expect(backend.callsTo("library_remove")).toEqual([{ id: github.id, confirm: false }]),
    );
  });
});

describe("skills", () => {
  it("lists them with their description, folder, users and problem", async () => {
    listing([github, skill]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    const row = await screen.findByRole("listitem", { name: "pdf" });
    expect(within(row).getByText("Fill and read PDF forms.")).toBeVisible();
    expect(within(row).getByText("C:/skills/pdf")).toBeVisible();
    expect(within(row).getByText("SKILL.md not found")).toBeVisible();
    expect(screen.queryByRole("listitem", { name: "github" })).toBeNull();
  });

  it("scans a folder and adds the skills that were ticked", async () => {
    backend.on("dialog_open", () => "C:/skills");
    backend.on("library_scan_skills", () => sample.scannedSkills);
    backend.on("library_add", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    await user.click(screen.getByRole("button", { name: "Add skill" }));
    const dialog = await screen.findByRole("dialog", { name: "Add skills" });
    expect(await within(dialog).findByLabelText("pdf")).toBeChecked();
    // A name the Library cannot take is shown with the reason and cannot be ticked.
    expect(within(dialog).getByLabelText("bad name")).toBeDisabled();
    expect(within(dialog).getByText(/use A-Z, a-z, 0-9/)).toBeVisible();
    expect(backend.callsTo("dialog_open")[0]).toMatchObject({ directory: true });
    expect(backend.callsTo("library_scan_skills")).toEqual([{ path: "C:/skills" }]);
    await user.click(within(dialog).getByRole("button", { name: "Add 1 skill" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        { name: "pdf", config: { kind: "skill", config: { path: "C:/skills/pdf" } }, accept: null },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says when a folder holds no skill, and keeps the dialog when an add fails", async () => {
    backend.on("dialog_open", () => "C:/empty");
    backend.on("library_scan_skills", () => []);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Skills" }));
    await user.click(screen.getByRole("button", { name: "Add skill" }));
    const dialog = await screen.findByRole("dialog", { name: "Add skills" });
    expect(await within(dialog).findByText("No SKILL.md found in that folder.")).toBeVisible();

    backend.on("library_scan_skills", () => sample.scannedSkills);
    backend.on("library_add", () => {
      throw { kind: "conflict", message: 'the skill "pdf" already exists' };
    });
    await user.click(within(dialog).getByRole("button", { name: "Choose another folder" }));
    await user.click(await within(dialog).findByRole("button", { name: "Add 1 skill" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
  });
});

describe("plugins", () => {
  it("lists a plugin with its parts, folder and users", async () => {
    listing([plugin, owned]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    expect(within(row).getByText("C:/plugins/pdf-tools")).toBeVisible();
    expect(within(row).getByText("1 skill")).toBeVisible();
    expect(within(row).getByText("1 command")).toBeVisible();
    expect(within(row).getByText("1 agent")).toBeVisible();
    expect(within(row).getByText("1 MCP server")).toBeVisible();
    expect(within(row).getByText("hooks")).toBeVisible();
    expect(within(row).getByText("Used by nightly-tests")).toBeVisible();
    expect(within(row).getByText("agents/broken.md: could not be read")).toBeVisible();
    expect(within(row).getByLabelText("Run this plugin's hooks")).not.toBeChecked();
  });

  it("switches the hooks on through the core, which asks, without a second question here", async () => {
    listing([plugin]);
    backend.on("library_update", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    await user.click(within(row).getByLabelText("Run this plugin's hooks"));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: plugin.id,
          config: { kind: "plugin", config: { ...plugin.config, run_hooks: true } },
        },
      ]),
    );
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("keeps the hooks off and says so calmly when the person declines", async () => {
    listing([plugin]);
    backend.on("library_update", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    const row = await screen.findByRole("listitem", { name: "pdf-tools" });
    await user.click(within(row).getByLabelText("Run this plugin's hooks"));
    expect(await within(row).findByText(/You declined: the hooks stay off/)).toBeVisible();
    expect(within(row).getByLabelText("Run this plugin's hooks")).not.toBeChecked();
    expect(screen.queryByText(/declined: nothing/)).toBeNull();
  });

  it("adds an installed Claude Code plugin", async () => {
    listing([]);
    backend.on("installed_claude_plugins", () => [
      ...sample.installedPlugins,
      { ...sample.installedPlugins[0]!, id: "gone@market", name: "gone", exists: false },
    ]);
    backend.on("library_add", () => sample.libraryItems[1]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    await user.click(screen.getByRole("button", { name: "Add plugin" }));
    const dialog = await screen.findByRole("dialog", { name: "Add plugin" });
    const installed = await within(dialog).findByRole("listitem", { name: "pdf-tools@market" });
    const gone = within(dialog).getByRole("listitem", { name: "gone@market" });
    expect(gone).toHaveTextContent("folder not found");
    expect(within(gone).getByRole("button", { name: "Add" })).toBeDisabled();
    await user.click(within(installed).getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "pdf-tools",
          config: {
            kind: "plugin",
            config: {
              path: sample.installedPlugins[0]!.install_path,
              installed_id: "pdf-tools@market",
              run_hooks: false,
            },
          },
          accept: null,
        },
      ]),
    );
  });

  it("adds a plugin from a folder under the folder's name, which can be changed", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:\\plugins\\pdf-tools");
    backend.on("library_add", () => {
      throw cancelled;
    });
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Plugins" }));
    await user.click(screen.getByRole("button", { name: "Add plugin" }));
    const dialog = await screen.findByRole("dialog", { name: "Add plugin" });
    await user.click(within(dialog).getByRole("button", { name: "Choose a folder" }));
    const name = await within(dialog).findByLabelText("Name");
    expect(name).toHaveValue("pdf-tools");
    await user.clear(name);
    await user.type(name, "pdfs");
    await user.click(within(dialog).getByRole("button", { name: "Add from folder" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "pdfs",
          config: {
            kind: "plugin",
            config: { path: "C:\\plugins\\pdf-tools", installed_id: null, run_hooks: false },
          },
          accept: null,
        },
      ]),
    );
    // The core asks about the servers the plugin starts; a refusal is not an error.
    expect(await within(dialog).findByText(/You declined: the plugin was not added/)).toBeVisible();
  });
});

describe("the server form's checks", () => {
  async function open() {
    const view = renderApp("/library");
    await view.user.click(await screen.findByRole("button", { name: "Add server" }));
    return { ...view, dialog: await screen.findByRole("dialog", { name: "Add MCP server" }) };
  }
  const NAME =
    'Use 1 to 40 of a-z, 0-9, _ and -, with no "__" and no "_" at the start or end. "dots" is reserved.';

  it.each(["_x", "x_", "a__b"])("refuses the name %s", async (name) => {
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), name);
    await user.type(within(dialog).getByLabelText("Command"), "run");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText(NAME)).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it.each(["http://host\\path", "http:///x", "http://host/\u202ex", "http://ho\u0001st/"])(
    "refuses the url %j",
    async (url) => {
      const { user, dialog } = await open();
      await user.selectOptions(within(dialog).getByLabelText("Transport"), "http");
      await user.type(within(dialog).getByLabelText("Name"), "web");
      await user.click(within(dialog).getByLabelText("URL"));
      await user.paste(url);
      await user.click(within(dialog).getByRole("button", { name: "Save" }));
      const message = within(dialog).getByText(/Enter a URL that starts with/);
      expect(message).toBeVisible();
      expect(message.textContent).not.toContain("\\");
      expect(backend.callsTo("library_add")).toEqual([]);
    },
  );

  it.each([
    ["Command", "ru\u0007n"],
    ["Arguments", "-a\u200e"],
    ["Working folder", "C:\\w\u0085"],
    ["Environment", "K\u202eY=1"],
  ])("refuses control or direction characters in %s", async (label, text) => {
    const { user, dialog } = await open();
    await user.type(within(dialog).getByLabelText("Name"), "files");
    if (label !== "Command") await user.type(within(dialog).getByLabelText("Command"), "run");
    await user.click(within(dialog).getByLabelText(label));
    await user.paste(text);
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("Remove the control or direction characters.")).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("says what an http server's variables do, and what a stdio server's do", async () => {
    const { user, dialog } = await open();
    expect(within(dialog).getByText(/when the server starts/)).toBeVisible();
    await user.selectOptions(within(dialog).getByLabelText("Transport"), "http");
    expect(within(dialog).getByText(/sends its value to the server.*asks first/)).toBeVisible();
    expect(within(dialog).queryByText(/when the server starts/)).toBeNull();
  });

  it("keeps an open form when a refetch of the list fails", async () => {
    listing([github]);
    const { user, queryClient } = renderApp("/library");
    const row = await screen.findByRole("listitem", { name: "github" });
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit github" });
    await user.type(within(dialog).getByLabelText("Working folder"), "C:\\work");
    backend.on("library_list", () => {
      throw { kind: "internal", message: "no database" };
    });
    await act(() => queryClient.invalidateQueries());
    expect(await screen.findByText("no database")).toBeVisible();
    expect(screen.getByRole("dialog", { name: "Edit github" })).toBeVisible();
    expect(within(dialog).getByLabelText("Working folder")).toHaveValue("C:\\work");
  });
});

describe("the Library's Context tab", () => {
  it("lists a collection with its description, documents, folder and every-dot badge", async () => {
    listing([company]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    expect(within(row).getByText("How we work: style, people, runbooks")).toBeVisible();
    expect(within(row).getByText("42 documents")).toBeVisible();
    expect(within(row).getByText("every dot")).toBeVisible();
    expect(within(row).getByText("Every dot reads it")).toBeVisible();
    expect(within(row).getByText("C:/notes/company")).toBeVisible();
  });

  it("adds a collection from a folder under a name made from it", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:\\Notes\\Company Wiki");
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    expect(within(dialog).getByText(/Only add folders whose writers you trust/)).toBeVisible();
    expect(within(dialog).getByLabelText("Name")).toHaveValue("company-wiki");
    await user.type(within(dialog).getByLabelText("Description"), "How we work");
    await user.click(within(dialog).getByLabelText("Every dot reads it"));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_add")).toEqual([
        {
          name: "company-wiki",
          config: {
            kind: "context",
            config: { path: "C:\\Notes\\Company Wiki", description: "How we work", every_dot: true },
          },
          accept: null,
        },
      ]),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("checks the name and the description before it asks the core", async () => {
    listing([]);
    backend.on("dialog_open", () => "C:/kb");
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    const name = within(dialog).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Bad Name");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      within(dialog).getByText("Use 1 to 40 of a-z, 0-9, _ and -, starting with a letter or digit."),
    ).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("edits a collection: the name stays, the config goes to the core", async () => {
    listing([company]);
    backend.on("library_update", () => company);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Edit" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit company" });
    expect(within(dialog).getByLabelText("Name")).toBeDisabled();
    await user.click(within(dialog).getByLabelText("Every dot reads it"));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("library_update")).toEqual([
        {
          id: company.id,
          config: {
            kind: "context",
            config: {
              path: "C:/notes/company",
              description: "How we work: style, people, runbooks",
              every_dot: false,
            },
          },
        },
      ]),
    );
  });

  it("shows a collection's first documents and says how many there are", async () => {
    listing([company]);
    backend.on("context_documents", () => sample.contextDocuments);
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Documents" }));
    const dialog = await screen.findByRole("dialog", { name: "Documents of company" });
    expect(await within(dialog).findByText("Ana Lima")).toBeVisible();
    expect(within(dialog).getByText("Owns billing")).toBeVisible();
    expect(within(dialog).getByText("The first 1 of 42 documents.")).toBeVisible();
    expect(backend.callsTo("context_documents")).toEqual([{ id: company.id }]);
  });
});

describe("the Context tab's other paths", () => {
  const context = async (entries: LibraryEntry[]) => {
    listing(entries);
    const view = renderApp("/library");
    await view.user.click(await screen.findByRole("tab", { name: "Context" }));
    return view;
  };

  it("does nothing when the folder dialog is cancelled", async () => {
    backend.on("dialog_open", () => null);
    const { user } = await context([]);
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    await waitFor(() => expect(backend.callsTo("dialog_open")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("shows a refused add in the dialog and lets Save be pressed again", async () => {
    backend.on("dialog_open", () => "C:/kb");
    backend.on("library_add", () => {
      throw { kind: "conflict", message: 'the context collection "kb" already exists' };
    });
    const { user } = await context([]);
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("already exists");
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("refuses a description over 300 characters before it asks the core", async () => {
    backend.on("dialog_open", () => "C:/kb");
    const { user } = await context([]);
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    await user.click(within(dialog).getByLabelText("Description"));
    await user.paste("x".repeat(301));
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("At most 300 characters.")).toBeVisible();
    expect(backend.callsTo("library_add")).toEqual([]);
  });

  it("counts a description as it is saved, without its edge spaces", async () => {
    backend.on("dialog_open", () => "C:/kb");
    backend.on("library_add", () => sample.libraryItems[0]);
    const { user } = await context([]);
    await user.click(screen.getByRole("button", { name: "Add collection" }));
    const dialog = await screen.findByRole("dialog", { name: "Add context collection" });
    await user.click(within(dialog).getByLabelText("Description"));
    await user.paste(`${"x".repeat(300)}   `);
    await user.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("library_add")).toHaveLength(1));
  });

  it("says why the documents cannot be read, and when there are none", async () => {
    backend.on("context_documents", () => {
      throw { kind: "internal", message: "the folder is gone" };
    });
    const { user } = await context([company]);
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Documents" }));
    const dialog = await screen.findByRole("dialog", { name: "Documents of company" });
    expect(await within(dialog).findByText(/the folder is gone/)).toBeVisible();
    await user.keyboard("{Escape}");
    backend.on("context_documents", () => ({ documents: [], total: 0, truncated: false, complete: true }));
    await user.click(within(row).getByRole("button", { name: "Documents" }));
    const again = await screen.findByRole("dialog", { name: "Documents of company" });
    expect(await within(again).findByText("No documents in that folder.")).toBeVisible();
  });

  it("shows why a collection cannot be read", async () => {
    await context([{ ...company, problem: "the folder C:/notes/company is missing" }]);
    const row = await screen.findByRole("listitem", { name: "company" });
    expect(within(row).getByText("the folder C:/notes/company is missing")).toBeVisible();
  });

  it("removes a collection", async () => {
    backend.on("library_remove", () => null);
    const { user } = await context([{ ...company, used_by: [] }]);
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove collection" }));
    await waitFor(() =>
      expect(backend.callsTo("library_remove")).toEqual([{ id: company.id, confirm: false }]),
    );
  });

  it("says every dot reads an every-dot collection, never that no dot uses it", async () => {
    const everyDot = { ...company, used_by: [] };
    expect(everyDot.kind === "context" && everyDot.config.every_dot).toBe(true);
    const { user } = await context([everyDot]);
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("Every dot reads it.");
    expect(dialog).not.toHaveTextContent("No dot uses it.");
  });

  it("names the dots that tick an every-dot collection too", async () => {
    const dot = github.used_by[0]!;
    const { user } = await context([{ ...company, used_by: [dot] }]);
    const row = await screen.findByRole("listitem", { name: "company" });
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("Every dot reads it.");
    expect(dialog).toHaveTextContent(dot.name);
    expect(dialog).not.toHaveTextContent("No dot uses it.");
  });
});

describe("a document count that is only a lower bound", () => {
  it("says at least in the row and in the Documents dialog", async () => {
    const bounded = { ...company, context: { documents: 100000, complete: false } };
    listing([bounded]);
    backend.on("context_documents", () => ({
      ...sample.contextDocuments,
      total: 100000,
      truncated: true,
      complete: false,
    }));
    const { user } = renderApp("/library");
    await user.click(await screen.findByRole("tab", { name: "Context" }));
    const row = await screen.findByRole("listitem", { name: "company" });
    expect(within(row).getByText("at least 100000 documents")).toBeVisible();
    await user.click(within(row).getByRole("button", { name: "Documents" }));
    const dialog = await screen.findByRole("dialog", { name: "Documents of company" });
    expect(
      await within(dialog).findByText("The first 1 of at least 100000 documents."),
    ).toBeVisible();
  });
});

describe("nameFromFolder", () => {
  it.each([
    ["C:\\Users\\me\\Notes", "notes"],
    ["C:\\Users\\me\\My Notes\\", "my-notes"],
    ["\\srv\\share", "share"],
    ["C:\\", "c"],
    ["Notes (old)", "notes-old"],
    ["文档", ""],
  ])("makes %j into %j", (folder, name) => {
    expect(nameFromFolder(folder)).toBe(name);
  });

  it("cuts a long folder name to a valid name of at most 40 characters", () => {
    const name = nameFromFolder(`C:/${"ab-".repeat(20)}`);
    expect(name.length).toBeLessThanOrEqual(40);
    expect(validName(name)).toBe(true);
    expect(name).not.toMatch(/[-_]$/);
  });
});
