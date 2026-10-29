import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { WorkspaceView } from "@/api/types";
import { backend } from "@/test/backend";
import { blankSpec, sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const ws = (id: string, name: string): WorkspaceView => ({
  scope: "personal",
  id,
  name,
  use_rules: null,
  my_role: null,
  can_manage: true,
});
const basics = { name: "nightly", instructions: "Run the tests.", workdir: "C:/dev/app" };

describe("the workspace of a dot", () => {
  it("new dot uses the current workspace", async () => {
    backend.on("scope_get", () => ({ account: "personal", workspace: "ops" }));
    backend.on("workspace_list", () => [ws("default", "Default"), ws("ops", "Ops")]);
    backend.on("create_dot", (args) => ({ ...sample.dot, ...(args.spec as object), id: "dot-new" }));
    const { user } = renderApp("/dots/new");
    const select = await screen.findByLabelText("Workspace");
    await waitFor(() => expect(select).toHaveValue("ops"));
    await user.type(screen.getByLabelText("Name"), basics.name);
    await user.type(screen.getByLabelText("Instructions"), basics.instructions);
    await user.type(screen.getByLabelText("Folder"), basics.workdir);
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    await waitFor(() => expect(backend.callsTo("create_dot")).toHaveLength(1));
    expect(backend.callsTo("create_dot")).toEqual([
      { spec: { ...blankSpec, ...basics }, orgId: null, workspaceId: "ops" },
    ]);
  });

  it("moving a dot calls dot_move and shows refusals", async () => {
    const dot = { ...sample.dot, workspace_id: "default" };
    backend.on("workspace_list", () => [ws("default", "Default"), ws("ops", "Ops")]);
    backend.on("get_dot", () => dot);
    backend.on("dot_move", (args) => ({ ...dot, workspace_id: String(args.workspaceId) }));
    const { user } = renderApp(`/dots/${dot.id}`);
    const select = await screen.findByLabelText("Workspace");
    await waitFor(() => expect(select).toHaveValue("default"));
    await user.selectOptions(select, "ops");
    await waitFor(() =>
      expect(backend.callsTo("dot_move")).toEqual([{ dotId: dot.id, workspaceId: "ops" }]),
    );
    await waitFor(() => expect(select).toHaveValue("ops"));

    backend.on("dot_move", () => {
      throw { kind: "conflict", message: "you are a viewer in 'default'" };
    });
    await user.selectOptions(select, "default");
    expect(await screen.findByText("you are a viewer in 'default'")).toBeVisible();
    expect(select).toHaveValue("ops");
  });
});
