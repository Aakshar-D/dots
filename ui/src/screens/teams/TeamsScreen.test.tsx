import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Team } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [crew, platform] = sample.teams as [Team, Team];

describe("the Teams screen", () => {
  it("sits on the rail after Library, before Connectors", async () => {
    renderApp("/teams");
    const nav = await screen.findByRole("navigation", { name: "Main" });
    const labels = within(nav)
      .getAllByRole("link")
      .map((link) => link.getAttribute("aria-label"));
    expect(labels).toEqual([
      "Home",
      "Inbox",
      "Intents",
      "Apps",
      "Library",
      "Teams",
      "Connectors",
      "Usage",
      "Settings",
    ]);
    expect(within(nav).getByRole("link", { name: "Teams" })).toHaveAttribute("href", "/teams");
    expect(within(nav).getByRole("link", { name: "Teams" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(await screen.findByText(/No teams yet/)).toBeVisible();
  });

  it("lists each team with its description, members and projects, opening its page", async () => {
    backend.on("teams_list", () => [crew, platform]);
    renderApp("/teams");
    const row = await screen.findByRole("listitem", { name: "crew" });
    expect(row).toHaveTextContent("Fixes and ships Dots.");
    expect(row).toHaveTextContent("2 members · 2 projects");
    expect(within(row).getByRole("link", { name: "crew" })).toHaveAttribute(
      "href",
      "/teams/01J0TEAM",
    );
    expect(screen.getByRole("listitem", { name: "platform" })).toHaveTextContent(
      "0 members · 0 projects",
    );
    expect(screen.queryByText(/No teams yet/)).toBeNull();
  });

  it("makes a team in the account and workspace the person works in, then opens it", async () => {
    const ops: Team = {
      ...crew,
      id: "01J0TEAM9",
      name: "ops",
      description: "Runs the servers.",
      members: [],
      projects: [],
    };
    backend.on("team_create", () => ops);
    backend.on("team_get", () => ops);
    backend.on("workflows_list", () => []);
    const { user, router } = renderApp("/teams");
    await user.click(await screen.findByRole("button", { name: "New team" }));
    const dialog = await screen.findByRole("dialog", { name: "New team" });
    const create = within(dialog).getByRole("button", { name: "Create team" });
    expect(create).toBeDisabled();
    await user.type(within(dialog).getByLabelText("Name"), " ops ");
    await user.type(within(dialog).getByLabelText("Description"), "Runs the servers.");
    await user.click(create);
    expect(await screen.findByRole("heading", { name: "ops" })).toBeVisible();
    expect(router.state.location.pathname).toBe("/teams/01J0TEAM9");
    expect(backend.callsTo("team_create")).toEqual([
      {
        spec: {
          name: "ops",
          description: "Runs the servers.",
          members: [],
          org_id: null,
          workspace_id: "default",
        },
      },
    ]);
  });

  it("makes a team in an org's workspace when the person works there", async () => {
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "platform" }));
    backend.on("team_create", () => platform);
    backend.on("team_get", () => platform);
    backend.on("workflows_list", () => []);
    const { user } = renderApp("/teams");
    await user.click(await screen.findByRole("button", { name: "New team" }));
    const dialog = await screen.findByRole("dialog", { name: "New team" });
    await user.type(within(dialog).getByLabelText("Name"), "platform");
    await user.click(within(dialog).getByRole("button", { name: "Create team" }));
    await screen.findByRole("heading", { name: "platform" });
    expect(backend.callsTo("team_create")[0]).toMatchObject({
      spec: { org_id: "acme-eng", workspace_id: "platform" },
    });
  });

  it("keeps the dialog open with the core's refusal", async () => {
    backend.on("team_create", () => {
      throw { kind: "conflict", message: 'a team named "crew" exists' };
    });
    const { user } = renderApp("/teams");
    await user.click(await screen.findByRole("button", { name: "New team" }));
    const dialog = await screen.findByRole("dialog", { name: "New team" });
    await user.type(within(dialog).getByLabelText("Name"), "crew");
    await user.click(within(dialog).getByRole("button", { name: "Create team" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      'a team named "crew" exists',
    );
    expect(within(dialog).getByRole("button", { name: "Create team" })).toBeEnabled();
  });
});
