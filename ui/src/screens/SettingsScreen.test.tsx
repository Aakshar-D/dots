import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview, Settings, Status } from "@/api/types";
import { settingsErrors } from "@/screens/SettingsScreen";
import { backend, later } from "@/test/backend";
import { quietStatus, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

type User = ReturnType<typeof renderApp>["user"];

/// Opens a Settings tab from the list beside the content.
async function openTab(user: User, name: string) {
  const tabs = await screen.findByRole("navigation", { name: "Settings sections" });
  await user.click(within(tabs).getByRole("link", { name }));
}

const stored = sample.settings; // one local endpoint, defaults otherwise
const [endpoint] = stored.local_endpoints;

/// A backend that stores what is saved, like the core.
function saving(restartRequired = false) {
  let current: Settings = stored;
  backend.on("get_settings", () => current);
  backend.on("update_settings", (args) => {
    current = args.settings as Settings;
    return { settings: current, restart_required: restartRequired };
  });
}

describe("the Settings screen", () => {
  it("shows what is stored and what is in use", async () => {
    const { user } = renderApp("/settings");
    expect(await screen.findByLabelText("Start Dots when I sign in")).toBeChecked();
    await waitFor(() =>
      expect(screen.getByLabelText("Data folder")).toHaveValue(quietStatus.data_dir),
    );
    await openTab(user, "Runs & server");
    expect(await screen.findByLabelText("Port")).toHaveValue(stored.port);
    expect(screen.getByLabelText("Runs at once")).toHaveValue(stored.max_concurrent_runs);
    await openTab(user, "Claude");
    expect(await screen.findByLabelText("Program path")).toHaveValue("");
    expect(screen.getByText(quietStatus.claude.program!)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Mark as tested" })).toBeNull();
    await openTab(user, "Local models");
    expect(await screen.findByRole("listitem", { name: endpoint })).toBeVisible();
  });

  it("saves the whole settings and says when a restart is needed", async () => {
    saving(true);
    backend.on("status", () =>
      backend.callsTo("update_settings").length > 0
        ? { ...quietStatus, restart_required: true }
        : quietStatus,
    );
    const { user } = renderApp("/settings/runs");
    const port = await screen.findByLabelText("Port");
    await user.clear(port);
    await user.type(port, "40123");
    await openTab(user, "General");
    await user.click(await screen.findByLabelText("Start Dots when I sign in"));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Saved. Restart Dots to apply.")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, port: 40123, autostart: false } },
    ]);
    // The frame's banner offers the restart.
    expect(await screen.findByRole("button", { name: "Restart Dots" })).toBeVisible();
  });

  it("saves the global instructions, up to 32 KB", async () => {
    saving();
    const { user } = renderApp("/settings/instructions");
    const text = await screen.findByLabelText("Global instructions");
    expect(text).toHaveValue(stored.global_instructions);
    await user.clear(text);
    await user.type(text, "Sign off as the team.");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, global_instructions: "Sign off as the team." } },
    ]);
    const limit = { port: 1, max_concurrent_runs: 1 };
    expect(settingsErrors({ ...limit, global_instructions: "x".repeat(32768) })).toEqual({});
    // Bytes, as the core counts: "é" is two.
    expect(settingsErrors({ ...limit, global_instructions: "é".repeat(16385) })).toEqual({
      global: "The global instructions are at most 32 KB.",
    });
  });

  it("saves the default model new dots start with", async () => {
    saving();
    const { user } = renderApp("/settings/claude");
    const model = await screen.findByLabelText("Default model");
    await within(model).findByRole("option", { name: "Opus" });
    expect(model).toHaveValue(stored.default_model);
    await user.selectOptions(model, "Opus");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, default_model: "opus" } },
    ]);
    // Any id through Custom…, but not none.
    await user.selectOptions(model, "Custom…");
    const box = screen.getByRole("textbox", { name: "Custom model ID" });
    await user.clear(box);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByText("Name the model new dots start with.")).toBeVisible();
    expect(backend.callsTo("update_settings")).toHaveLength(1);
    await user.type(box, "claude-opus-4-1");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")[1]).toEqual({
        settings: { ...stored, default_model: "claude-opus-4-1" },
      }),
    );
  });

  it("checks every result with Haiku by default, and saves a change to either", async () => {
    saving();
    const { user } = renderApp("/settings/claude");
    const box = await screen.findByLabelText("Check every result");
    expect(box).toBeChecked();
    const model = screen.getByLabelText("Checker model");
    await within(model).findByRole("option", { name: "Sonnet" });
    expect(model).toHaveValue("haiku");
    await user.click(box);
    await user.selectOptions(model, "Sonnet");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, check_results: false, checker_model: "sonnet" } },
    ]);
    expect(settingsErrors({ port: 1, max_concurrent_runs: 1, checker_model: " " })).toEqual({
      checker: "Name the model that checks results.",
    });
  });

  it("keeps numbers inside the core's bounds", async () => {
    expect(settingsErrors({ port: 47321, max_concurrent_runs: 16 })).toEqual({});
    expect(Object.keys(settingsErrors({ port: 0, max_concurrent_runs: 17 }))).toEqual([
      "port",
      "max",
    ]);
    expect(settingsErrors({ port: 65536, max_concurrent_runs: 1 })).toHaveProperty("port");
    expect(settingsErrors({ port: Number.NaN, max_concurrent_runs: 1.5 })).toMatchObject({
      port: expect.any(String),
      max: expect.any(String),
    });

    saving();
    const { user } = renderApp("/settings/runs");
    const max = await screen.findByLabelText("Runs at once");
    await user.clear(max);
    await user.type(max, "99");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(screen.getByText("Runs at once must be a whole number, 1 to 16.")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  it("sends a new Claude program, and an emptied path as none", async () => {
    saving(true);
    const { user } = renderApp("/settings/claude");
    const path = await screen.findByLabelText("Program path");
    await user.type(path, "  C:/tools/claude.exe ");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_settings")).toHaveLength(1));
    expect(backend.callsTo("update_settings")[0]).toEqual({
      settings: { ...stored, claude_path: "C:/tools/claude.exe" },
    });

    await user.clear(path);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_settings")).toHaveLength(2));
    expect(backend.callsTo("update_settings")[1]).toEqual({ settings: stored });
  });

  it("says that nothing was saved when the native confirmation is declined", async () => {
    backend.on("update_settings", () => {
      throw { kind: "cancelled", message: "the Claude program was not changed" };
    });
    const { user } = renderApp("/settings/claude");
    await user.type(await screen.findByLabelText("Program path"), "C:/evil.exe");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText("Not saved: you declined the change of the Claude program."),
    ).toBeVisible();
    // The user's own "no" is not an error.
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText("Saved")).toBeNull();
  });

  it("stops saying 'Not saved' once the path is edited after a decline", async () => {
    backend.on("update_settings", () => {
      throw { kind: "cancelled", message: "the Claude program was not changed" };
    });
    const { user } = renderApp("/settings/claude");
    const path = await screen.findByLabelText("Program path");
    await user.type(path, "C:/evil.exe");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText(/^Not saved:/)).toBeVisible();

    // Another field does not answer what was declined; a new path does.
    await openTab(user, "General");
    await user.click(await screen.findByLabelText("Start Dots when I sign in"));
    await openTab(user, "Claude");
    expect(await screen.findByText(/^Not saved:/)).toBeVisible();
    await user.type(screen.getByLabelText("Program path"), "{Backspace}");
    expect(screen.queryByText(/^Not saved:/)).toBeNull();
    expect(screen.getByText(/^Leave empty to find claude on your PATH/)).toBeVisible();
  });

  it("shows the core's refusal when a save fails", async () => {
    backend.on("update_settings", () => {
      throw { kind: "invalid", message: "endpoint_url must start with http://" };
    });
    const { user } = renderApp("/settings/claude");
    await user.click(await screen.findByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("must start with http://");
    // A refusal is about what was sent: it stays until the next save, whatever is typed.
    await user.type(screen.getByLabelText("Program path"), "x");
    expect(screen.getByRole("alert")).toHaveTextContent("must start with http://");
  });

  it("adds and removes local endpoints", async () => {
    saving();
    const { user } = renderApp("/settings/models");
    const add = await screen.findByLabelText("Add an endpoint");
    await user.type(add, "https://lan-box:11434{Enter}");
    expect(screen.getByText("An endpoint starts with http:// and names a host.")).toBeVisible();

    await user.clear(add);
    await user.type(add, "http://lan-box:11434");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(screen.getByRole("listitem", { name: "http://lan-box:11434" })).toBeVisible();
    expect(add).toHaveValue("");

    await user.click(screen.getByRole("button", { name: `Remove ${endpoint}` }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...stored, local_endpoints: ["http://lan-box:11434"] } },
      ]),
    );
  });

  it("tests an endpoint with a model and shows the answer or the failure", async () => {
    backend.on("test_endpoint", () => "The model answered with a well-formed tool call.");
    const { user } = renderApp("/settings/models");
    const row = within(await screen.findByRole("listitem", { name: endpoint }));
    expect(row.getByRole("button", { name: "Test endpoint" })).toBeDisabled();
    await user.type(row.getByLabelText(`Model for ${endpoint}`), "gemma");
    await user.click(row.getByRole("button", { name: "Test endpoint" }));
    expect(await row.findByRole("status")).toHaveTextContent("well-formed tool call");
    expect(backend.callsTo("test_endpoint")).toEqual([{ endpoint, model: "gemma" }]);

    backend.on("test_endpoint", () => {
      throw { kind: "internal", message: "the endpoint is unreachable" };
    });
    await user.click(row.getByRole("button", { name: "Test endpoint" }));
    expect(await row.findByRole("alert")).toHaveTextContent("the endpoint is unreachable");
  });

  it("pauses schedules at once, without Save", async () => {
    backend.on("set_schedules_paused", (args) => ({
      ...quietStatus,
      schedules_paused: args.paused as boolean,
    }));
    const { user } = renderApp("/settings");
    const pause = await screen.findByLabelText("Pause schedules and watchers");
    await waitFor(() => expect(pause).toBeEnabled());
    await user.click(pause);
    await waitFor(() => expect(screen.getByLabelText("Pause schedules and watchers")).toBeChecked());
    expect(backend.callsTo("set_schedules_paused")).toEqual([{ paused: true }]);
    expect(backend.callsTo("update_settings")).toEqual([]);
    expect(await screen.findByRole("button", { name: "Resume schedules" })).toBeVisible();
  });

  it("keeps a pause when an older status was on its way", async () => {
    let paused = false;
    backend.on("status", () => ({ ...quietStatus, schedules_paused: paused }));
    backend.on("set_schedules_paused", (args) => {
      paused = args.paused as boolean;
      return { ...quietStatus, schedules_paused: paused };
    });
    const { user, listening, queryClient } = renderApp("/settings");
    const pause = await screen.findByLabelText("Pause schedules and watchers");
    await listening();

    // The status is asked for again, and the core reads it for that answer before the pause.
    const read = later<Status>();
    const asked = backend.callsTo("status").length;
    backend.on("status", () =>
      backend.callsTo("status").length === asked + 1
        ? read.answer
        : { ...quietStatus, schedules_paused: paused },
    );
    act(() => backend.emit("runtime-lagged", null));
    await waitFor(() => expect(backend.callsTo("status")).toHaveLength(asked + 1));
    await user.click(pause);
    await waitFor(() => expect(pause).toBeChecked());
    await act(async () => read.give(quietStatus));
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));

    expect(pause).toBeChecked();
    expect(backend.callsTo("status")).toHaveLength(asked + 2);
  });

  it("makes the dots column load again after a pause that came during its first load", async () => {
    let paused = false;
    backend.on("status", () => ({ ...quietStatus, schedules_paused: paused }));
    backend.on("set_schedules_paused", (args) => {
      paused = args.paused as boolean;
      return { ...quietStatus, schedules_paused: paused };
    });
    const { user, listening } = renderApp("/settings");
    await screen.findByLabelText("Pause schedules and watchers");
    await listening();

    // The dots column is read, and left for Settings before its answer is here.
    const before: DotOverview = sample.overview;
    const after: DotOverview = { ...before, dot: { ...before.dot, name: "read-after-the-pause" } };
    const read = later<DotOverview[]>();
    backend.on("dot_overview", () =>
      backend.callsTo("dot_overview").length === 1 ? read.answer : [after],
    );
    await user.click(screen.getByRole("link", { name: "Home" }));
    await waitFor(() => expect(backend.callsTo("dot_overview")).toHaveLength(1));
    await user.click(screen.getByRole("link", { name: "Settings" }));
    await user.click(await screen.findByLabelText("Pause schedules and watchers"));
    await waitFor(() => expect(screen.getByLabelText("Pause schedules and watchers")).toBeChecked());
    await act(async () => read.give([before]));

    // The answer was read before the pause, so the list is asked for again when it is shown.
    await user.click(screen.getByRole("link", { name: "Home" }));
    expect(await screen.findByRole("listitem", { name: "read-after-the-pause" })).toBeVisible();
    expect(backend.callsTo("dot_overview")).toHaveLength(2);
  });

  it("offers to mark an untested CLI version as tested", async () => {
    backend.on("status", () => ({
      ...quietStatus,
      claude: { ...quietStatus.claude, version: "2.1.300", mismatch: true },
    }));
    backend.on("mark_claude_tested", () => quietStatus);
    const { user } = renderApp("/settings/claude");
    const main = within(await screen.findByRole("main"));
    await user.click(await main.findByRole("button", { name: "Mark as tested" }));
    await waitFor(() => expect(main.queryByRole("button", { name: "Mark as tested" })).toBeNull());
    expect(backend.callsTo("mark_claude_tested")).toHaveLength(1);
  });
});

describe("the quiet limit setting", () => {
  it("shows the stored seconds as minutes and saves minutes as seconds", async () => {
    saving();
    const { user } = renderApp("/settings/runs");
    const field = await screen.findByLabelText("Quiet limit for every dot");
    expect(stored.quiet_limit_secs).toBe(600);
    expect(field).toHaveValue(10);
    await user.clear(field);
    await user.type(field, "20");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, quiet_limit_secs: 1200 } },
    ]);
  });

  it.each(["0", "1441"])("marks %s minutes and does not save", async (minutes) => {
    saving();
    const { user } = renderApp("/settings/runs");
    const field = await screen.findByLabelText("Quiet limit for every dot");
    await user.clear(field);
    await user.type(field, minutes);
    expect(screen.getByText(/1 to 1440/)).toBeVisible();
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    await user.click(save);
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  it("checks the limit in seconds", () => {
    const base = { port: 47321, max_concurrent_runs: 4 };
    expect(settingsErrors({ ...base, quiet_limit_secs: 60 })).toEqual({});
    expect(settingsErrors({ ...base, quiet_limit_secs: 86400 })).toEqual({});
    expect(settingsErrors({ ...base, quiet_limit_secs: 0 })).toHaveProperty("quiet");
    expect(settingsErrors({ ...base, quiet_limit_secs: 86460 })).toHaveProperty("quiet");
    expect(settingsErrors({ ...base, quiet_limit_secs: 90 })).toHaveProperty("quiet");
  });
});

describe("the auto-archive setting", () => {
  it("saves the days after which an idle topic is archived, 0 for never", async () => {
    saving();
    const { user } = renderApp("/settings/runs");
    const field = await screen.findByLabelText("Archive topics idle for (days)");
    expect(field).toHaveValue(stored.auto_archive_days);
    await user.clear(field);
    await user.type(field, "0");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, auto_archive_days: 0 } },
    ]);
  });

  it("checks the days", () => {
    const base = { port: 47321, max_concurrent_runs: 4 };
    expect(settingsErrors({ ...base, auto_archive_days: 0 })).toEqual({});
    expect(settingsErrors({ ...base, auto_archive_days: 3650 })).toEqual({});
    expect(settingsErrors({ ...base, auto_archive_days: 3651 })).toHaveProperty("archive");
    expect(settingsErrors({ ...base, auto_archive_days: 1.5 })).toHaveProperty("archive");
  });
});

describe("the Settings tabs", () => {
  const tabs = () => screen.findByRole("navigation", { name: "Settings sections" });

  it("opens General at /settings", async () => {
    const { router } = renderApp("/settings");
    expect(await screen.findByLabelText("Start Dots when I sign in")).toBeVisible();
    expect(router.state.location.pathname).toBe("/settings/general");
    expect(within(await tabs()).getByRole("link", { name: "General" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.queryByLabelText("Port")).toBeNull();
  });

  it("lists the tabs in order", async () => {
    renderApp("/settings");
    const names = within(await tabs())
      .getAllByRole("link")
      .map((link) => link.textContent);
    expect(names).toEqual([
      "General",
      "Account",
      "Instructions",
      "Claude",
      "Runs & server",
      "Local models",
      "Browser",
      "Updates",
      "Help",
    ]);
  });

  it("shows only the chosen tab and puts it in the address", async () => {
    const { user, router } = renderApp("/settings");
    await screen.findByLabelText("Start Dots when I sign in");
    await openTab(user, "Runs & server");
    expect(await screen.findByLabelText("Port")).toBeVisible();
    expect(router.state.location.pathname).toBe("/settings/runs");
    expect(within(await tabs()).getByRole("link", { name: "Runs & server" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(within(await tabs()).getByRole("link", { name: "General" })).not.toHaveAttribute(
      "aria-current",
    );
    expect(screen.queryByLabelText("Start Dots when I sign in")).toBeNull();
    expect(screen.queryByLabelText("Global instructions")).toBeNull();

    // Back goes to the tab before.
    await act(() => router.navigate(-1));
    expect(await screen.findByLabelText("Start Dots when I sign in")).toBeVisible();
    expect(screen.queryByLabelText("Port")).toBeNull();
  });

  it("sends an unknown tab back to the first", async () => {
    const { router } = renderApp("/settings/nonsense");
    expect(await screen.findByLabelText("Start Dots when I sign in")).toBeVisible();
    expect(router.state.location.pathname).toBe("/settings/general");
  });

  it("keeps an unsaved edit across tabs, says so, and saves it", async () => {
    saving();
    const { user } = renderApp("/settings/instructions");
    const text = await screen.findByLabelText("Global instructions");
    expect(screen.queryByText("Unsaved changes")).toBeNull();
    await user.clear(text);
    await user.type(text, "Be brief.");
    expect(screen.getByText("Unsaved changes")).toBeVisible();

    await openTab(user, "Help");
    expect(await screen.findByRole("button", { name: "Report a problem" })).toBeVisible();
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    await openTab(user, "Instructions");
    expect(await screen.findByLabelText("Global instructions")).toHaveValue("Be brief.");

    await openTab(user, "Updates");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_settings")).toEqual([
      { settings: { ...stored, global_instructions: "Be brief." } },
    ]);
    await waitFor(() => expect(screen.queryByText("Unsaved changes")).toBeNull());
  });

  it("opens the tab with an error when Save finds one there, and marks it", async () => {
    saving();
    const { user, router } = renderApp("/settings/runs");
    const max = await screen.findByLabelText("Runs at once");
    await user.clear(max);
    await user.type(max, "99");
    await openTab(user, "General");
    await screen.findByLabelText("Start Dots when I sign in");
    expect(within(await tabs()).queryByText("has errors")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/settings/runs"));
    expect(await screen.findByText("Runs at once must be a whole number, 1 to 16.")).toBeVisible();
    expect(within(await tabs()).getByRole("link", { name: /Runs & server/ })).toHaveTextContent(
      "has errors",
    );
    expect(within(await tabs()).getByRole("link", { name: "General" })).not.toHaveTextContent(
      "has errors",
    );
    expect(backend.callsTo("update_settings")).toEqual([]);
  });

  it("keeps the rail's Settings link lit on every tab", async () => {
    renderApp("/settings/updates");
    const rail = await screen.findByRole("navigation", { name: "Main" });
    expect(within(rail).getByRole("link", { name: "Settings" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });
});
