import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply } from "@/test/home";
import { renderApp } from "@/test/render";

/// Renders the app at `path` and waits until it has settled on `expected`.
async function lands(path: string, expected: string) {
  const { router } = renderApp(path);
  await waitFor(() => expect(router.state.location.pathname).toBe(expected));
  return router;
}

describe("the home's routes", () => {
  it("opens the home for the root, the old Dots and Chat screens and unknown paths", async () => {
    home([]);
    for (const path of ["/", "/dots", "/chat", "/nowhere"]) {
      const router = await lands(path, `/home/${dot.id}`);
      router.dispose();
    }
  });

  it("opens a dot's conversation for its old chat address", async () => {
    home([]);
    await lands(`/chat/${dot.id}`, `/home/${dot.id}`);
  });

  it("opens an old chat's topic in the conversation, or alone when it is older", async () => {
    const old = chat("old", "08:00");
    const current = chat("current", "10:00");
    home([old, reply("old2", old, "08:30"), current]);
    const first = await lands(`/chat/${dot.id}/current`, `/home/${dot.id}`);
    first.dispose();
    // Any run of the older topic names it.
    await lands(`/chat/${dot.id}/old2`, `/home/${dot.id}/old`);
    expect(await screen.findByText("Back to the current conversation")).toBeVisible();
  });

  it("opens the dot's conversation for an old chat address of a run that is gone", async () => {
    home([]);
    backend.on("run_thread", () => {
      throw { kind: "not_found", message: "run not found" };
    });
    await lands(`/chat/${dot.id}/gone`, `/home/${dot.id}`);
  });

  it("keeps the editor, the Inbox, the Library and Settings where they were", async () => {
    backend.on("get_dot", () => dot);
    for (const path of [`/dots/${dot.id}`, "/dots/new", "/inbox", "/library", "/settings"]) {
      const router = await lands(path, path);
      router.dispose();
    }
  });

  it("names each place of the rail and shows the Inbox count", async () => {
    backend.on("inbox", () => sample.inbox);
    const { user, router } = renderApp("/settings");
    const rail = await screen.findByRole("navigation", { name: "Main" });
    const inbox = await within(rail).findByRole("link", { name: "Inbox, 6 waiting" });
    expect(inbox).toHaveAttribute("title", "Inbox, 6 waiting");
    expect(within(inbox).getByText("6")).toBeVisible();
    await user.click(within(rail).getByRole("link", { name: "Home" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/home"));
    expect(await screen.findByRole("link", { name: "Create your first dot" })).toBeVisible();
  });
});
