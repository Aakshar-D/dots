import { onlineManager } from "@tanstack/react-query";
import { act, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

/// What the webview does when the machine loses its network: TanStack's online manager listens
/// for these window events.
function report(state: "offline" | "online") {
  act(() => void window.dispatchEvent(new Event(state)));
  expect(onlineManager.isOnline()).toBe(state === "online");
}

describe("while the webview reports offline", () => {
  // Back online, so the state does not leak into other tests.
  afterEach(() => report("online"));

  it("still sends a command", async () => {
    const { dot } = sample.overview;
    backend.on("dot_overview", () => [sample.overview]);
    backend.on("run_now", () => ({ ...sample.run, id: "run-new", status: "queued" }));
    const { user } = renderApp(`/home/${dot.id}`);
    const settings = await screen.findByRole("button", { name: "Dot settings" });

    report("offline");
    await user.click(settings);
    await user.click(await screen.findByRole("menuitem", { name: "Run now" }));
    await waitFor(() => expect(backend.callsTo("run_now")).toEqual([{ dotId: dot.id }]));
  });

  it("loads the data of a screen opened meanwhile", async () => {
    const { user } = renderApp("/settings");
    await screen.findByRole("heading", { name: "Settings" });

    report("offline");
    backend.on("dot_overview", () => [sample.overview]);
    await user.click(screen.getByRole("link", { name: "Home" }));
    expect(await screen.findByRole("listitem", { name: sample.dot.name })).toBeVisible();
  });
});
