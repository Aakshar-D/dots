import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotAccess, Refusals, WorkspaceView } from "@/api/types";
import { backend } from "@/test/backend";
import { allowed, blankSpec, sample } from "@/test/fixtures";
import { keys } from "@/lib/query";
import { renderApp } from "@/test/render";

const ws = (
  id: string,
  name: string,
  refusals: Refusals = allowed,
  scope = "personal",
): WorkspaceView => ({
  scope,
  id,
  name,
  use_rules: null,
  my_role: null,
  can_manage: true,
  refusals,
});
// The core's words for a Viewer in Acme Engineering's Payments workspace, and for a member
// outside it.
const viewer = (action: string) =>
  `Acme Engineering: a Viewer in the Payments workspace can't ${action}. Ask an Admin or Owner.`;
const OUTSIDE =
  "Acme Engineering: you aren't in the Payments workspace. Ask an Admin or Owner to add you.";
const asViewer: Refusals = {
  edit: viewer("create or change dots"),
  run: viewer("run dots"),
  answer: viewer("answer approvals"),
  create_api_keys: viewer("create API keys"),
};
const outside: Refusals = { edit: OUTSIDE, run: OUTSIDE, answer: OUTSIDE, create_api_keys: OUTSIDE };
const [acme] = sample.orgs;
// Acme's Default, where the person is a Member, and Payments, as `payments` refuses.
const acmeSpaces = (payments: Refusals) => [
  ws("default", "Default", allowed, "acme-eng"),
  ws("payments", "Payments", payments, "acme-eng"),
];

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

describe("what the core lets the person do with a dot", () => {
  const dot = { ...sample.dot, org_id: "acme-eng", workspace_id: "payments" };

  function open(access: DotAccess) {
    backend.on("org_list", () => [acme]);
    backend.on("get_dot", () => dot);
    backend.on("workspace_list", () => acmeSpaces(asViewer));
    backend.on("dot_access", () => access);
    return renderApp(`/dots/${dot.id}`);
  }

  it("a workspace Viewer can't save, run or move the dot, and is told why", async () => {
    open(asViewer);
    const save = await screen.findByRole("button", { name: "Save" });
    await waitFor(() => expect(save).toBeDisabled());
    expect(save).toHaveAttribute("title", viewer("create or change dots"));
    const run = screen.getByRole("button", { name: "Run now" });
    expect(run).toBeDisabled();
    expect(run).toHaveAttribute("title", viewer("run dots"));
    const move = await screen.findByLabelText("Workspace");
    expect(move).toBeDisabled();
    expect(move).toHaveAttribute("title", viewer("create or change dots"));
    expect(backend.callsTo("dot_access")).toContainEqual({ dotId: dot.id });
  });

  it("outside the dot's workspace, each says the person isn't in it", async () => {
    open(outside);
    const save = await screen.findByRole("button", { name: "Save" });
    await waitFor(() => expect(save).toBeDisabled());
    expect(save).toHaveAttribute("title", OUTSIDE);
    const run = screen.getByRole("button", { name: "Run now" });
    expect(run).toBeDisabled();
    expect(run).toHaveAttribute("title", OUTSIDE);
  });

  it("a Member can save, run and move it", async () => {
    open(allowed);
    const save = await screen.findByRole("button", { name: "Save" });
    await waitFor(() => expect(backend.callsTo("dot_access").length).toBeGreaterThan(0));
    expect(save).toBeEnabled();
    expect(screen.getByRole("button", { name: "Run now" })).toBeEnabled();
    expect(await screen.findByLabelText("Workspace")).toBeEnabled();
  });

  it("a move offers a workspace the person can't change dots in switched off, with why", async () => {
    const home = { ...dot, workspace_id: "default" };
    backend.on("org_list", () => [acme]);
    backend.on("get_dot", () => home);
    backend.on("workspace_list", () => acmeSpaces(asViewer));
    renderApp(`/dots/${home.id}`);
    const move = await screen.findByLabelText("Workspace");
    const payments = await within(move).findByRole("option", { name: "Payments" });
    expect(payments).toBeDisabled();
    expect(payments).toHaveAttribute("title", viewer("create or change dots"));
    expect(within(move).getByRole("option", { name: "Default" })).toBeEnabled();
  });
});

describe("the workspace a new dot starts in", () => {
  it("offers one the person can't make dots in switched off, with why", async () => {
    backend.on("org_list", () => [acme]);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    backend.on("workspace_list", () => acmeSpaces(asViewer));
    renderApp("/dots/new");
    const picker = await screen.findByLabelText("Workspace");
    const payments = within(picker).getByRole("option", { name: "Payments" });
    expect(payments).toBeDisabled();
    expect(payments).toHaveAttribute("title", viewer("create or change dots"));
    expect(screen.getByRole("button", { name: "Create dot" })).toBeEnabled();
  });

  it("can't be created in a workspace the person isn't in, which Create dot says", async () => {
    backend.on("org_list", () => [acme]);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "payments" }));
    backend.on("workspace_list", () => acmeSpaces(outside));
    renderApp("/dots/new");
    const create = await screen.findByRole("button", { name: "Create dot" });
    await waitFor(() => expect(create).toBeDisabled());
    expect(create).toHaveAttribute("title", OUTSIDE);
  });
});

describe("an open editor and an org event", () => {
  it("keeps what was typed in a dot opened from the cache, and asks the access again", async () => {
    const dot = { ...sample.dot, org_id: "acme-eng" };
    backend.on("org_list", () => [acme]);
    backend.on("get_dot", () => dot);
    const { user, router, queryClient, listening } = renderApp("/home");
    await listening();
    // As after Create: the dot is in the cache and its editor opens from it, without a fetch.
    queryClient.setQueryData(keys.dot(dot.id), dot);
    await act(() => router.navigate(`/dots/${dot.id}`));
    const name = await screen.findByLabelText("Name");
    await waitFor(() => expect(backend.callsTo("dot_access")).toHaveLength(1));
    await user.clear(name);
    await user.type(name, "typed-name");

    const dotLoads = backend.callsTo("get_dot").length;
    act(() => backend.emit("runtime", { type: "org_updated", org_id: "acme-eng" }));
    await waitFor(() => expect(backend.callsTo("dot_access")).toHaveLength(2));
    await waitFor(() => expect(queryClient.isFetching()).toBe(0));
    // The dot itself was not loaded again, and the form kept what was typed.
    expect(backend.callsTo("get_dot")).toHaveLength(dotLoads);
    expect(await screen.findByLabelText("Name")).toHaveValue("typed-name");
  });
});

describe("a failed question about what the person may do", () => {
  it("is shown by the editor's buttons, which stay on", async () => {
    const dot = { ...sample.dot, workspace_id: "default" };
    backend.on("get_dot", () => dot);
    backend.on("dot_access", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/dots/${dot.id}`);
    expect(await screen.findByText(/database is locked/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Run now" })).toBeEnabled();
  });

  it("of a new dot's workspaces is shown too, and Create dot stays on", async () => {
    backend.on("workspace_list", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp("/dots/new");
    expect(await screen.findByText(/database is locked/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Create dot" })).toBeEnabled();
  });
});
