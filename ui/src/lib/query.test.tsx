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
    const { user } = renderApp("/dots");
    const runNow = await screen.findByRole("button", { name: `Run ${dot.name} now` });

    report("offline");
    await user.click(runNow);
    await waitFor(() => expect(backend.callsTo("run_now")).toEqual([{ dotId: dot.id }]));
  });

  it("loads the data of a screen opened meanwhile", async () => {
    backend.on("list_dots", () => [sample.dot]);
    const { user } = renderApp("/dots");
    await screen.findByText(/No dots yet/);

    report("offline");
    await user.click(screen.getByRole("link", { name: "Chat" }));
    expect(await screen.findByRole("option", { name: sample.dot.name })).toBeInTheDocument();
  });
});
