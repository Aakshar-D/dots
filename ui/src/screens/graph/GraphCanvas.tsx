import cytoscape, { type Core, type ElementDefinition } from "cytoscape";
import { useEffect, useRef } from "react";

import type { ViewEdge, ViewNode } from "@/api/types";

/// A community's colour: hues spread around the wheel, the same for the same id.
export function communityColor(id: number): string {
  return `hsl(${(id * 47) % 360} 62% 52%)`;
}

/// Positions for a large view: each community a disc of its nodes (a sunflower spiral), the
/// discs around a circle, largest first. Linear in the nodes, and the same each time.
export function clusterPositions(nodes: ViewNode[]): Record<string, { x: number; y: number }> {
  const groups = new Map<number, ViewNode[]>();
  for (const n of nodes) {
    const list = groups.get(n.community) ?? [];
    list.push(n);
    groups.set(n.community, list);
  }
  const ordered = [...groups.entries()].sort((a, b) => b[1].length - a[1].length || a[0] - b[0]);
  const step = 12;
  const radius = (count: number) => step * Math.sqrt(count) + step;
  const ring = Math.max(1, ordered.reduce((sum, [, list]) => sum + 2 * radius(list.length), 0));
  const big = ring / (2 * Math.PI);
  const out: Record<string, { x: number; y: number }> = {};
  let angle = 0;
  for (const [, list] of ordered) {
    const r = radius(list.length);
    angle += r / big;
    const cx = ordered.length === 1 ? 0 : big * Math.cos(angle);
    const cy = ordered.length === 1 ? 0 : big * Math.sin(angle);
    angle += r / big;
    list.forEach((n, i) => {
      const d = step * Math.sqrt(i);
      const a = i * 2.39996;
      out[n.id] = { x: cx + d * Math.cos(a), y: cy + d * Math.sin(a) };
    });
  }
  return out;
}

/// The nodes and edges of a view on a Cytoscape canvas, coloured by community and sized by
/// their links. Clicking a node calls `onSelect` with its id. Cytoscape draws on a canvas, so
/// the screen keeps a list of the same nodes for the keyboard and screen readers.
export function GraphCanvas(props: {
  nodes: ViewNode[];
  edges: ViewEdge[];
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const onSelect = useRef(props.onSelect);
  onSelect.current = props.onSelect;

  useEffect(() => {
    if (!box.current) return;
    const max = Math.max(1, ...props.nodes.map((n) => n.weight));
    const ids = new Set(props.nodes.map((n) => n.id));
    const elements: ElementDefinition[] = [
      ...props.nodes.map((n) => ({
        data: {
          id: n.id,
          label: n.label,
          color: communityColor(n.community),
          size: 8 + Math.round(28 * Math.sqrt(n.weight / max)),
        },
      })),
      ...props.edges
        .filter((e) => ids.has(e.source) && ids.has(e.target))
        .map((e, i) => ({
          data: { id: `e${i}`, source: e.source, target: e.target, kind: e.kind },
        })),
    ];
    const instance = cytoscape({
      container: box.current,
      elements,
      minZoom: 0.05,
      maxZoom: 4,
      style: [
        {
          selector: "node",
          style: {
            "background-color": "data(color)",
            width: "data(size)",
            height: "data(size)",
            label: "data(label)",
            "font-size": 9,
            color: "#888",
            "text-valign": "bottom",
            "min-zoomed-font-size": 8,
          },
        },
        {
          selector: "edge",
          style: { width: 0.6, "line-color": "#9993", "curve-style": "haystack" },
        },
        {
          selector: 'edge[kind = "calls"]',
          style: { "line-color": "#6b8afd66" },
        },
        {
          selector: "node:selected",
          style: { "border-width": 3, "border-color": "#f59e0b" },
        },
      ],
      // A force layout for a small view; a large one is placed by community at once.
      layout:
        props.nodes.length > 600
          ? { name: "preset", positions: clusterPositions(props.nodes) }
          : ({ name: "cose", animate: false, numIter: 600 } as cytoscape.LayoutOptions),
    });
    instance.on("tap", "node", (e) => onSelect.current(e.target.id()));
    cy.current = instance;
    return () => {
      instance.destroy();
      cy.current = null;
    };
  }, [props.nodes, props.edges]);

  useEffect(() => {
    const instance = cy.current;
    if (!instance) return;
    instance.nodes().unselect();
    if (props.selected) {
      const node = instance.getElementById(props.selected);
      if (node.nonempty()) {
        node.select();
        instance.animate({ center: { eles: node }, duration: 200 });
      }
    }
  }, [props.selected, props.nodes]);

  return (
    <div
      ref={box}
      role="img"
      aria-label={`Graph of ${props.nodes.length} nodes`}
      className="h-full min-h-[420px] w-full rounded-lg border bg-card"
    />
  );
}
