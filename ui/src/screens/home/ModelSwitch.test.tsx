import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { backend, later } from "@/test/backend";
import { allowed } from "@/test/fixtures";
import { dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const switcher = () => screen.findByRole("combobox", { name: "Model for the next reply" });

describe("the model switch above a dot's conversation", () => {
  it("shows the dot's model and saves a new one through update_dot for the next reply", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    backend.on("update_dot", (args) => ({ ...dot, ...(args.spec as object) }));
    const { user } = renderApp(`/home/${dot.id}`);
    const select = await switcher();
    expect(select).toHaveValue("sonnet");
    expect(select.closest("[title]")).toHaveAttribute(
      "title",
      expect.stringContaining("next reply"),
    );
    await user.selectOptions(select, await within(select).findByRole("option", { name: "Opus" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    const [call] = backend.callsTo("update_dot");
    expect(call.id).toBe(dot.id);
    // Everything else as the core has it; only the model changes.
    expect(call.spec).toMatchObject({ name: dot.name, instructions: dot.instructions, model: "opus" });
    expect(await screen.findByText(`${dot.name} will use Opus from its next reply.`)).toBeVisible();
  });

  it("switches a Claude dot to Auto", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    backend.on("update_dot", (args) => ({ ...dot, ...(args.spec as object) }));
    const { user } = renderApp(`/home/${dot.id}`);
    const select = await switcher();
    await user.selectOptions(select, await within(select).findByRole("option", { name: "Auto" }));
    await waitFor(() =>
      expect(backend.callsTo("update_dot")[0]?.spec).toMatchObject({ model: "auto" }),
    );
    expect(await screen.findByText(`${dot.name} will use Auto from its next reply.`)).toBeVisible();
  });

  it("takes any model id through Custom…", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    backend.on("update_dot", (args) => ({ ...dot, ...(args.spec as object) }));
    const { user } = renderApp(`/home/${dot.id}`);
    const select = await switcher();
    await within(select).findByRole("option", { name: "Opus" });
    await user.selectOptions(select, "Custom…");
    const box = screen.getByRole("textbox", { name: "Custom model ID" });
    await user.clear(box);
    await user.type(box, "claude-opus-4-1");
    expect(backend.callsTo("update_dot")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Use" }));
    await waitFor(() =>
      expect(backend.callsTo("update_dot")[0]?.spec).toMatchObject({ model: "claude-opus-4-1" }),
    );
  });

  it("shows a refusal and keeps the dot's model", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    backend.on("update_dot", () => {
      throw { kind: "not_allowed", message: "A Viewer can't change dots." };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const select = await switcher();
    await user.selectOptions(select, await within(select).findByRole("option", { name: "Haiku" }));
    expect(await screen.findByText("A Viewer can't change dots.")).toBeVisible();
    await waitFor(() => expect(select).toHaveValue("sonnet"));
  });

  it("drops the Custom box after a refused change", async () => {
    home([]);
    backend.on("get_dot", () => dot);
    backend.on("update_dot", () => {
      throw { kind: "not_allowed", message: "A Viewer can't change dots." };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const select = await switcher();
    await within(select).findByRole("option", { name: "Opus" });
    await waitFor(() => expect(select).toBeEnabled());
    await user.selectOptions(select, "Custom…");
    const box = screen.getByRole("textbox", { name: "Custom model ID" });
    await user.clear(box);
    await user.type(box, "claude-opus-4-1");
    await user.click(screen.getByRole("button", { name: "Use" }));
    expect(await screen.findByText("A Viewer can't change dots.")).toBeVisible();
    await waitFor(() =>
      expect(screen.queryByRole("textbox", { name: "Custom model ID" })).toBeNull(),
    );
    expect(await switcher()).toHaveValue("sonnet");
  });

  it("is off until it is known whether the person may edit the dot", async () => {
    home([]);
    const access = later<typeof allowed>();
    backend.on("dot_access", () => access.answer);
    renderApp(`/home/${dot.id}`);
    const select = await switcher();
    expect(select).toBeDisabled();
    access.give(allowed);
    await waitFor(() => expect(select).toBeEnabled());
  });

  it("is off with the reason when the person may not edit the dot", async () => {
    home([]);
    backend.on("dot_access", () => ({ ...allowed, edit: "A Viewer can't change dots." }));
    renderApp(`/home/${dot.id}`);
    const select = await switcher();
    await waitFor(() => expect(select).toBeDisabled());
    expect(select.closest("[title]")).toHaveAttribute("title", "A Viewer can't change dots.");
  });
});
