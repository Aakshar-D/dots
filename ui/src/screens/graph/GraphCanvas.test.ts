import cytoscape from "cytoscape";
import { describe, expect, it } from "vitest";

import { communityColor } from "@/screens/graph/GraphCanvas";

/// The colour Cytoscape draws a node with `color` in its data.
function drawn(color: string): string {
  const cy = cytoscape({
    headless: true,
    styleEnabled: true,
    elements: [{ data: { id: "a", color } }],
    style: [{ selector: "node", style: { "background-color": "data(color)" } }],
  });
  const value = cy.getElementById("a").style("background-color") as string;
  cy.destroy();
  return value;
}

describe("communityColor", () => {
  it("gives a colour Cytoscape can parse, and different ones for different clusters", () => {
    const grey = drawn("not a colour");
    const first = drawn(communityColor(0));
    const second = drawn(communityColor(1));
    expect(first).not.toBe(grey);
    expect(second).not.toBe(grey);
    expect(first).not.toBe(second);
  });
});
