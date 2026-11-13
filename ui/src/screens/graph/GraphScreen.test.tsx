import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import type { GraphStatus, GraphView, NodeDetail, ViewNode } from "@/api/types";
import { backend } from "@/test/backend";
import { dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

// Cytoscape draws on a canvas jsdom does not have: the canvas is a list of buttons here.
vi.mock("@/screens/graph/GraphCanvas", () => ({
  communityColor: () => "red",
  clusterPositions: () => ({}),
  GraphCanvas: (props: {
    nodes: ViewNode[];
    selected: string | null;
    onSelect: (id: string) => void;
  }) => (
    <div aria-label="canvas">
      {props.nodes.map((n) => (
        <button
          key={n.id}
          type="button"
          aria-pressed={props.selected === n.id}
          onClick={() => props.onSelect(n.id)}
        >
          {`node ${n.label}`}
        </button>
      ))}
    </div>
  ),
}));

const [ready, building, none, failed] = T.GraphStatus.array().parse(contract.GraphStatus) as [
  GraphStatus,
  GraphStatus,
  GraphStatus,
  GraphStatus,
];
const [whole, collapsed] = T.GraphView.array().parse(contract.GraphView) as [GraphView, GraphView];
const detail: NodeDetail = T.NodeDetail.parse(contract.NodeDetail);
const hits: ViewNode[] = T.ViewNode.array().parse(contract.ViewNode);

/// A backend with the dot (the graph on unless `useGraph` is false) and its graph in `status`.
function holding(status: GraphStatus, view: GraphView = whole, useGraph = true) {
  home([]);
  const held = { status };
  backend.on("get_dot", () => ({ ...dot, use_graph: useGraph }));
  backend.on("graph_status", () => held.status);
  backend.on("graph_view", (args) =>
    args.community === null ? { ...view, status: held.status } : { ...whole, community: 0 },
  );
  backend.on("graph_node", () => detail);
  backend.on("graph_search", (args) =>
    hits.filter((h) => h.label.toLowerCase().includes(String(args.query).toLowerCase())),
  );
  backend.on("graph_rebuild", () => {
    held.status = building;
    return building;
  });
  backend.on("graph_export", () => "C:/code/shop/graphify-out/graph.json");
  backend.on("graph_reveal", () => null);
  return held;
}

/// Clicks the button named `name`.
async function userEventClick(name: string) {
  const { default: userEvent } = await import("@testing-library/user-event");
  await userEvent.click(await screen.findByRole("button", { name }));
}

async function open() {
  const view = renderApp(`/graph/${dot.id}`);
  expect(await screen.findByRole("heading", { name: `${dot.name} · Graph` })).toBeVisible();
  return view;
}

describe("the Graph screen", () => {
  it("is reached from the sol's header", async () => {
    holding(ready);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.click(await screen.findByRole("link", { name: "Graph" }));
    expect(await screen.findByRole("heading", { name: `${dot.name} · Graph` })).toBeVisible();
  });

  it("shows the graph with its status line, coloured nodes and actions", async () => {
    holding(ready);
    await open();
    expect(await screen.findByRole("button", { name: "node Runner.start" })).toBeVisible();
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("412 files · 5,210 nodes · 14,877 edges · 3 skipped");
    expect(status).toHaveTextContent("1 file could not be parsed");
    expect(screen.getByRole("button", { name: "Rebuild" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Export for graphify" })).toBeEnabled();
  });

  it("selects a node from the canvas and shows its neighbours, file and Show in folder", async () => {
    holding(ready);
    const { user } = await open();
    await user.click(await screen.findByRole("button", { name: "node Runner.start" }));
    const panel = await screen.findByRole("complementary", { name: "Selected node" });
    expect(within(panel).getByRole("heading", { name: "Runner.start" })).toBeVisible();
    expect(within(panel).getByText("src/runner.rs:120")).toBeVisible();
    expect(within(panel).getByText(/pub async fn start/)).toBeVisible();
    const calls = within(panel).getByRole("region", { name: "calls" });
    expect(within(calls).getByRole("button", { name: /tokio/ })).toBeVisible();
    expect(within(panel).getByText("and 3 more")).toBeVisible();
    await user.click(within(panel).getByRole("button", { name: "Show in folder" }));
    await waitFor(() =>
      expect(backend.callsTo("graph_reveal")).toEqual([
        { dotId: dot.id, id: "src/runner.rs#Runner.start" },
      ]),
    );
  });

  it("searches by name and selects a hit", async () => {
    holding(ready);
    const { user } = await open();
    await user.type(await screen.findByLabelText("Search the graph"), "tok");
    const results = await screen.findByRole("region", { name: "Search results" });
    await user.click(within(results).getByRole("button", { name: /tokio/ }));
    await waitFor(() =>
      expect(backend.callsTo("graph_node").at(-1)).toEqual({ dotId: dot.id, id: "pkg:tokio" }),
    );
    await user.clear(screen.getByLabelText("Search the graph"));
    await user.type(screen.getByLabelText("Search the graph"), "zzz");
    expect(await screen.findByText("Nothing matches.")).toBeVisible();
  });

  it("opens a cluster of a large graph and goes back to all clusters", async () => {
    holding(ready, collapsed);
    const { user } = await open();
    expect(await screen.findByText(/each circle is a cluster/)).toBeVisible();
    const clusters = screen.getByRole("region", { name: "Clusters" });
    // Right under the page's h1.
    expect(within(clusters).getByRole("heading", { level: 2, name: "Clusters" })).toBeVisible();
    await user.click(within(clusters).getByRole("button", { name: /src \(240 files\)/ }));
    expect(await screen.findByRole("button", { name: "node Runner.start" })).toBeVisible();
    await waitFor(() =>
      expect(backend.callsTo("graph_view").at(-1)).toEqual({ dotId: dot.id, community: 0 }),
    );
    await user.click(screen.getByRole("button", { name: "All clusters" }));
    expect(await screen.findByRole("region", { name: "Clusters" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "All clusters" })).toBeNull();
  });

  it("offers Build graph when there is none, and shows the build's progress", async () => {
    const held = holding({ ...none, git: true });
    const { user } = await open();
    expect(await screen.findByText(/No graph yet/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Build graph" }));
    expect(backend.callsTo("graph_rebuild")).toEqual([{ dotId: dot.id }]);
    held.status = { ...building, built_at: null, progress: 42 };
    act(() => backend.emit("runtime", { type: "graph_updated", status: held.status }));
    expect(await screen.findByText("Building the graph, 42% done.")).toBeVisible();
    expect(screen.getByRole("button", { name: "Build graph" })).toBeDisabled();
    // The build ends: the graph is drawn.
    held.status = ready;
    act(() => backend.emit("runtime", { type: "graph_updated", status: ready }));
    expect(await screen.findByRole("button", { name: "node Runner.start" })).toBeVisible();
  });

  it("says a folder outside git is built only when asked, and that a cut walk is partial", async () => {
    holding({ ...none, git: false });
    await open();
    expect(await screen.findByText(/This folder is not a git repository/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Build graph" })).toBeEnabled();
  });

  it("says when the walk stopped at its cap", async () => {
    holding({ ...ready, truncated: true });
    await open();
    expect(await screen.findByRole("status")).toHaveTextContent(
      "stopped at the file cap; part of the project is left out",
    );
  });

  it("notes that graphify-out belongs in .gitignore next to Export", async () => {
    holding(ready);
    await open();
    await screen.findByRole("button", { name: "Export for graphify" });
    expect(screen.getByText(/Add graphify-out\/ to .gitignore/)).toBeVisible();
  });

  it("says when the last build failed, and keeps Rebuild", async () => {
    holding(failed);
    await open();
    expect(await screen.findByText(/The last build failed: the graph file cannot be opened/))
      .toBeVisible();
    expect(screen.getByRole("button", { name: "Rebuild" })).toBeEnabled();
  });

  it("points to the sol's settings when the graph is off", async () => {
    holding(ready, whole, false);
    await open();
    expect(await screen.findByText(/The project graph is off for this sol/)).toBeVisible();
    expect(screen.getByRole("link", { name: "its settings" })).toHaveAttribute(
      "href",
      `/dots/${dot.id}`,
    );
    expect(backend.callsTo("graph_status")).toEqual([]);
  });

  it("exports for graphify and says where", async () => {
    holding(ready);
    const { user } = await open();
    await user.click(await screen.findByRole("button", { name: "Export for graphify" }));
    expect(await screen.findByText(/Exported to C:\/code\/shop\/graphify-out\/graph.json/))
      .toBeVisible();
  });

  it("lists the view's nodes for the keyboard, with Show more past the first ones", async () => {
    const many: ViewNode[] = Array.from({ length: 60 }, (_, i) => ({
      ...hits[0]!,
      id: `src/a.rs#f${i}`,
      label: `f${i}`,
    }));
    holding(ready, { ...whole, nodes: many, edges: [] });
    const { user } = await open();
    const list = await screen.findByRole("region", { name: "Nodes in view" });
    expect(within(list).getAllByRole("button", { name: /^f\d+, / })).toHaveLength(50);
    await user.click(within(list).getByRole("button", { name: "Show 10 more" }));
    expect(within(list).getAllByRole("button", { name: /^f\d+, / })).toHaveLength(60);
    await user.click(within(list).getByRole("button", { name: /^f59, / }));
    await waitFor(() =>
      expect(backend.callsTo("graph_node").at(-1)).toEqual({ dotId: dot.id, id: "src/a.rs#f59" }),
    );
  });

  it("gives long labels their full text as a tooltip, and says 1 file", async () => {
    holding({ ...ready, files: 1 });
    await open();
    expect(await screen.findByRole("status")).toHaveTextContent("1 file ·");
    const list = await screen.findByRole("region", { name: "Nodes in view" });
    const label = within(list).getByText("Runner.start");
    expect(label).toHaveAttribute("title", "Runner.start");
  });

  it("names its controls for assistive technology", async () => {
    holding(ready);
    await open();
    expect(await screen.findByRole("link", { name: "Back to the sol" })).toBeVisible();
    expect(await screen.findByRole("searchbox", { name: "Search the graph" })).toBeVisible();
    // The signature scrolls sideways, so it can be focused and is named.
    await userEventClick("node Runner.start");
    const signature = await screen.findByLabelText("Signature");
    expect(signature).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("status")).toBeVisible();
  });
});
