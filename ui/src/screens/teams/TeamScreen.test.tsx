import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot, Team } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [crew] = sample.teams as [Team, Team];

/// A dot like the sample one, in the account and workspace given.
function dot(id: string, name: string, org_id: string | null = null, workspace_id = "default") {
  return { ...sample.dot, id, name, org_id, workspace_id } satisfies Dot;
}

/// nightly-tests and reviewer (the crew's members), designer (Personal) and ops (an org's).
const DOTS = [
  dot("01J0DOT", "nightly-tests"),
  dot("01J0DOT2", "reviewer"),
  dot("01J0DOT3", "designer"),
  dot("01J0DOT4", "ops", "acme-eng", "platform"),
];

/// The crew's page: its two members and two projects, its four workflows and no run.
function crewPage(dots: Dot[] = DOTS) {
  backend.on("team_get", () => crew);
  backend.on("workflows_list", () => sample.workflows);
  backend.on("list_dots", () => dots);
  backend.on("team_update", () => crew);
  return renderApp(`/teams/${crew.id}`);
}

const members = [
  { dot_id: "01J0DOT", role: "dev agent" },
  { dot_id: "01J0DOT2", role: "" },
];

describe("the team page", () => {
  it("shows the team's name and description, and saves them", async () => {
    const { user } = crewPage();
    expect(await screen.findByRole("heading", { name: "crew", level: 1 })).toBeVisible();
    const name = screen.getByLabelText("Name");
    expect(name).toHaveValue("crew");
    expect(screen.getByLabelText("Description")).toHaveValue("Fixes and ships Dots.");
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();
    await user.clear(name);
    await user.type(name, "core crew ");
    await user.click(save);
    expect(backend.callsTo("team_update")).toEqual([
      {
        id: "01J0TEAM",
        spec: {
          name: "core crew",
          description: "Fixes and ships Dots.",
          members,
          org_id: null,
          workspace_id: "default",
        },
      },
    ]);
  });

  it("saves a member's role when its field is left, and removes one after asking", async () => {
    const { user } = crewPage();
    const section = within(await screen.findByRole("region", { name: "Members" }));
    const role = section.getByRole("textbox", { name: "Role of nightly-tests" });
    expect(role).toHaveValue("dev agent");
    await user.clear(role);
    await user.type(role, "lead");
    // Nothing is saved while the person types.
    expect(backend.callsTo("team_update")).toEqual([]);
    await user.tab();
    expect(backend.callsTo("team_update")).toEqual([
      expect.objectContaining({
        spec: expect.objectContaining({
          members: [
            { dot_id: "01J0DOT", role: "lead" },
            { dot_id: "01J0DOT2", role: "" },
          ],
        }),
      }),
    ]);
    await user.click(section.getByRole("button", { name: "Remove reviewer" }));
    const ask = await screen.findByRole("alertdialog", { name: "Remove reviewer from the team?" });
    expect(ask).toHaveTextContent("A workflow whose steps name it cannot start");
    await user.click(within(ask).getByRole("button", { name: "Remove" }));
    expect(backend.callsTo("team_update")[1]).toMatchObject({
      spec: { members: [{ dot_id: "01J0DOT", role: "dev agent" }] },
    });
  });

  it("adds a dot of the team's account and workspace with its role", async () => {
    const { user } = crewPage();
    const section = within(await screen.findByRole("region", { name: "Members" }));
    const pick = await section.findByLabelText("Dot");
    // Members and the org's dot are not offered.
    expect(within(pick).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Choose a dot",
      "designer",
    ]);
    const add = section.getByRole("button", { name: "Add member" });
    expect(add).toBeDisabled();
    await user.selectOptions(pick, "01J0DOT3");
    await user.type(section.getByLabelText("Role"), " designer ");
    await user.click(add);
    expect(backend.callsTo("team_update")[0]).toMatchObject({
      spec: { members: [...members, { dot_id: "01J0DOT3", role: "designer" }] },
    });
  });

  it("says when every dot that may join is a member", async () => {
    crewPage(DOTS.filter((d) => d.id !== "01J0DOT3"));
    expect(await screen.findByText(/Every dot of the team's account and workspace/)).toBeVisible();
  });

  it("lists the projects with their folder, remote and default branch, and opens one", async () => {
    backend.on("open_project_folder", () => null);
    const { user } = crewPage();
    const section = within(await screen.findByRole("region", { name: "Projects" }));
    const dots = section.getByRole("listitem", { name: "Dots" });
    expect(dots).toHaveTextContent("C:/Users/u/src/dots");
    expect(dots).toHaveTextContent("Remote origin · default branch main");
    expect(section.getByRole("listitem", { name: "Site" })).toHaveTextContent(
      "Remote upstream · default branch found by git",
    );
    await user.click(within(dots).getByRole("button", { name: "Open the folder of Dots" }));
    expect(backend.callsTo("open_project_folder")).toEqual([
      { projectId: "01J0PRJ", workflowRunId: null, branch: null },
    ]);
  });

  it("adds a project from a picked folder, named after it, and shows a refusal", async () => {
    backend.on("dialog_open", () => "C:\\src\\tools");
    backend.on("project_add", () => {
      throw { kind: "invalid", message: "C:\\src\\tools is not a git repository" };
    });
    const { user } = crewPage();
    const section = within(await screen.findByRole("region", { name: "Projects" }));
    await user.click(section.getByRole("button", { name: "Add project" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Add project" }));
    expect(dialog.getByRole("button", { name: "Add project" })).toBeDisabled();
    await user.click(dialog.getByRole("button", { name: "Choose…" }));
    expect(await dialog.findByLabelText("Folder")).toHaveValue("C:\\src\\tools");
    expect(dialog.getByLabelText("Name")).toHaveValue("tools");
    expect(dialog.getByLabelText("Remote")).toHaveValue("origin");
    await user.click(dialog.getByRole("button", { name: "Add project" }));
    expect(await dialog.findByRole("alert")).toHaveTextContent("is not a git repository");
    backend.on("project_add", () => crew.projects[0]);
    await user.click(dialog.getByRole("button", { name: "Add project" }));
    expect(backend.callsTo("project_add")).toEqual([
      {
        teamId: "01J0TEAM",
        spec: {
          name: "tools",
          repo_path: "C:\\src\\tools",
          default_branch: null,
          remote: "origin",
        },
      },
      expect.anything(),
    ]);
    expect(screen.queryByRole("dialog", { name: "Add project" })).toBeNull();
  });

  it("edits a project and deletes one after asking", async () => {
    backend.on("project_update", () => crew.projects[1]);
    backend.on("project_delete", () => null);
    const { user } = crewPage();
    const section = within(await screen.findByRole("region", { name: "Projects" }));
    await user.click(section.getByRole("button", { name: "Edit Site" }));
    const dialog = within(await screen.findByRole("dialog", { name: "Edit Site" }));
    expect(dialog.getByLabelText("Folder")).toHaveValue("C:/Users/u/src/site");
    expect(dialog.getByLabelText("Remote")).toHaveValue("upstream");
    await user.type(dialog.getByLabelText("Default branch"), "trunk");
    await user.click(dialog.getByRole("button", { name: "Save" }));
    expect(backend.callsTo("project_update")).toEqual([
      {
        id: "01J0PRJ2",
        spec: {
          name: "Site",
          repo_path: "C:/Users/u/src/site",
          default_branch: "trunk",
          remote: "upstream",
        },
      },
    ]);
    await user.click(section.getByRole("button", { name: "Delete Dots" }));
    const ask = await screen.findByRole("alertdialog", { name: "Delete Dots?" });
    expect(ask).toHaveTextContent("Its folder and git repo stay as they are");
    await user.click(within(ask).getByRole("button", { name: "Delete project" }));
    expect(backend.callsTo("project_delete")).toEqual([{ id: "01J0PRJ" }]);
  });

  it("deletes the team after saying its runs in flight are stopped, then lists teams", async () => {
    backend.on("team_delete", () => null);
    const { user, router } = crewPage();
    await user.click(await screen.findByRole("button", { name: "Delete team" }));
    const ask = await screen.findByRole("alertdialog", { name: "Delete crew?" });
    // Fix and ship has a run in flight.
    expect(ask).toHaveTextContent("Runs in flight are stopped first: 1 run now.");
    await user.click(within(ask).getByRole("button", { name: "Delete team" }));
    expect(await screen.findByRole("heading", { name: "Teams" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/teams");
    expect(backend.callsTo("team_delete")).toEqual([{ id: "01J0TEAM" }]);
  });

  it("shows why a team cannot be shown", async () => {
    backend.on("team_get", () => {
      throw { kind: "not_found", message: "team nope" };
    });
    backend.on("workflows_list", () => {
      throw { kind: "not_found", message: "team nope" };
    });
    renderApp("/teams/nope");
    expect(await screen.findByRole("alert")).toHaveTextContent("team nope");
  });
});
