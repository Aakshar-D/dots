import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const row = (name: string, org_id: string | null): DotOverview => ({
  ...sample.overview,
  dot: { ...sample.dot, id: `dot-${name}`, name, org_id, schedule: null, enabled: true },
  active_run: null,
  last_run: null,
});
const dots = [row("mine", null), row("builder", "acme-eng"), row("pixels", "design")];
const column = () => screen.getByRole("navigation", { name: "Dots" });

describe("the home follows the current account", () => {
  it("lists only the dots of the current org", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("scope_get", () => ({ account: "acme-eng", workspace: "default" }));
    renderApp("/home");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole("listitem", {
      name: "builder",
    });
    expect(within(column()).queryByRole("listitem", { name: "mine" })).toBeNull();
    expect(within(column()).queryByRole("listitem", { name: "pixels" })).toBeNull();
  });

  it("lists only the dots without an org in Personal", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("scope_get", () => ({ account: "personal", workspace: "default" }));
    renderApp("/home");
    await within(await screen.findByRole("navigation", { name: "Dots" })).findByRole("listitem", {
      name: "mine",
    });
    expect(within(column()).queryByRole("listitem", { name: "builder" })).toBeNull();
    expect(within(column()).queryByRole("listitem", { name: "pixels" })).toBeNull();
  });

  it("lists every dot while the scope loads", async () => {
    backend.on("dot_overview", () => dots);
    backend.on("scope_get", () => new Promise(() => {}));
    renderApp("/home");
    const nav = await screen.findByRole("navigation", { name: "Dots" });
    for (const name of ["mine", "builder", "pixels"]) {
      expect(await within(nav).findByRole("listitem", { name })).toBeVisible();
    }
  });
});
