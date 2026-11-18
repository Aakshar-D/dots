import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Dot } from "@/api/types";
import {
  MAKE_PER_RUN_RULE,
  MAKE_TOTAL_RULE,
  makingError,
  specErrors,
  specOf,
} from "@/lib/dot-rules";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
const path = `/dots/${dot.id}`;

/// A backend holding `start`; a save changes it as the core would.
function stored(start: Dot = dot) {
  const held = { dot: start };
  backend.on("get_dot", () => held.dot);
  backend.on("update_dot", (args) => {
    held.dot = { ...held.dot, ...(args.spec as object) };
    return held.dot;
  });
  return held;
}

const section = async () =>
  within((await screen.findByText("Making sols")).closest("fieldset") as HTMLElement);

describe("the sol editor's Making sols section", () => {
  it("is off for a sol until the person turns it on, and hides the limits until then", async () => {
    stored();
    renderApp(path);
    const making = await section();
    expect(making.getByRole("switch", { name: "Can make sols" })).not.toBeChecked();
    expect(making.queryByLabelText("Sols it may make in one run")).toBeNull();
    expect(making.queryByLabelText("Sols it may keep in total")).toBeNull();
    // What turning it on means is said before it is on.
    expect(making.getByText(/without asking you/)).toBeVisible();
    expect(making.getByText(/Sage asks you on a card first/)).toBeVisible();
  });

  it("turns it on with the core's defaults and saves the limits typed", async () => {
    stored();
    const { user } = renderApp(path);
    const making = await section();
    await user.click(making.getByRole("switch", { name: "Can make sols" }));
    const perRun = making.getByLabelText("Sols it may make in one run");
    const total = making.getByLabelText("Sols it may keep in total");
    expect(perRun).toHaveValue(3);
    expect(total).toHaveValue(10);
    await user.clear(perRun);
    await user.type(perRun, "5");
    await user.clear(total);
    await user.type(total, "20");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([
      { id: dot.id, spec: { ...specOf(dot), making: { on: true, per_run: 5, total: 20 } } },
    ]);
  });

  it("marks a limit out of its range with the core's words, and does not save", async () => {
    stored({ ...dot, making: { on: true, per_run: 3, total: 10 } });
    const { user } = renderApp(path);
    const making = await section();
    const total = making.getByLabelText("Sols it may keep in total");
    await user.clear(total);
    await user.type(total, "51");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await making.findByText("A sol may keep 1 to 50 made sols.")).toBeVisible();
    expect(backend.callsTo("update_dot")).toEqual([]);

    // The other limit has its own words, under its own field.
    await user.clear(total);
    await user.type(total, "50");
    const perRun = making.getByLabelText("Sols it may make in one run");
    await user.clear(perRun);
    await user.type(perRun, "0");
    expect(await making.findByText("A sol may make 1 to 10 sols in one run.")).toBeVisible();
    expect(making.queryByText("A sol may keep 1 to 50 made sols.")).toBeNull();
    await user.clear(perRun);
    await user.type(perRun, "10");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    expect(backend.callsTo("update_dot")[0]).toMatchObject({
      spec: { making: { on: true, per_run: 10, total: 50 } },
    });
  });

  it("shows the core's refusal when a save is refused", async () => {
    const held = stored();
    backend.on("update_dot", () => {
      throw { kind: "not_allowed", message: "Acme: a Viewer can't change sols." };
    });
    const { user } = renderApp(path);
    const making = await section();
    await user.click(making.getByRole("switch", { name: "Can make sols" }));
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Acme: a Viewer can't change sols.")).toBeVisible();
    expect(held.dot.making.on).toBe(false);
  });

  it("warns when the sol's own permissions keep the tools from it", async () => {
    stored({
      ...dot,
      making: { on: true, per_run: 3, total: 10 },
      policy: { ...dot.policy, rules: [...dot.policy.rules, { tool: "MakeDots", action: "deny" }] },
    });
    renderApp(path);
    const making = await section();
    expect(making.getByText(/permissions deny MakeDots/)).toBeVisible();
  });

  it("mirrors the core's ranges", () => {
    expect(makingError({ on: false, per_run: 1, total: 1 })).toBeUndefined();
    expect(makingError({ on: true, per_run: 10, total: 50 })).toBeUndefined();
    expect(makingError({ on: true, per_run: 11, total: 10 })).toBe(MAKE_PER_RUN_RULE);
    expect(makingError({ on: true, per_run: 2.5, total: 10 })).toBe(MAKE_PER_RUN_RULE);
    // Checked while the switch is off too, as the core checks them.
    expect(makingError({ on: false, per_run: 3, total: 0 })).toBe(MAKE_TOTAL_RULE);
    expect(makingError({ on: false, per_run: 3, total: Number.NaN })).toBe(MAKE_TOTAL_RULE);
    expect(specErrors({ ...specOf(dot), making: { on: true, per_run: 3, total: 51 } })).toEqual({
      making: "A sol may keep 1 to 50 made sols.",
    });
    expect(MAKE_PER_RUN_RULE).toBe("A sol may make 1 to 10 sols in one run.");
  });
});

describe("a made sol in the editor", () => {
  const made: Dot = {
    ...dot,
    made: { by_dot_id: "01J0MAKER", by_name: "Researcher", run_id: "01J0RUN", purpose: "Reads the feeds" },
  };

  it("says who made it, with a link to the maker", async () => {
    stored(made);
    renderApp(path);
    const link = await screen.findByRole("link", { name: "Researcher" });
    expect(link).toHaveAttribute("href", "/dots/01J0MAKER");
    expect(link.closest("p")).toHaveTextContent("Made by Researcher · Reads the feeds");
  });

  it("names a deleted maker without a link", async () => {
    stored({ ...made, made: { ...made.made!, by_dot_id: null } });
    renderApp(path);
    expect(await screen.findByText(/Made by Researcher \(deleted\)/)).toBeVisible();
    expect(screen.queryByRole("link", { name: "Researcher" })).toBeNull();
  });

  it("says nothing of a maker for a sol a person made", async () => {
    stored();
    renderApp(path);
    await section();
    expect(screen.queryByText(/Made by/)).toBeNull();
  });

  it("warns that with the switch on it has more than its maker", async () => {
    stored(made);
    const { user } = renderApp(path);
    const making = await section();
    expect(making.queryByText(/more than its maker/)).toBeNull();
    await user.click(making.getByRole("switch", { name: "Can make sols" }));
    expect(
      making.getByText(/Sage asks you before Researcher gives it a job or changes it/),
    ).toBeVisible();
  });
});
