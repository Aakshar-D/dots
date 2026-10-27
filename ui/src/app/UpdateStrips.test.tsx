import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";

import type { UpdateNote } from "@/api/types";
import { FatalPage } from "@/app/FatalPage";
import { localTime } from "@/lib/format";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";

const AT = "2026-10-27T03:12:00.000Z";

function withNote(note: UpdateNote) {
  backend.on("update_note", () => note);
  return renderApp("/dots");
}

describe("the strips after an update", () => {
  it("an unhealthy version offers Go back with the exact words", async () => {
    backend.on("update_go_back", () => ({
      state: "installing",
      version: "0.2.0",
      purpose: "go_back",
    }));
    const { user } = withNote({ state: "unhealthy", version: "0.3.0", back_to: "0.2.0" });
    expect(
      await screen.findByText(
        "Dots 0.3.0 did not start properly. Go back to 0.2.0? Your data from before the update comes back with it; anything done since the update is lost.",
      ),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Go back" }));
    await waitFor(() => expect(backend.callsTo("update_go_back")).toHaveLength(1));
    expect(await screen.findByText(/Going back to Dots 0\.2\.0/)).toBeVisible();
  });

  it("Not now hides an unhealthy strip until the next note", async () => {
    backend.on("update_later", () => ({ state: "none" }));
    const { user } = withNote({ state: "unhealthy", version: "0.3.0", back_to: "0.2.0" });
    await user.click(await screen.findByRole("button", { name: "Not now" }));
    await waitFor(() => expect(screen.queryByText(/did not start properly/)).toBeNull());
  });

  it("a withdrawn version offers Go back, or says a fix will come when there is nothing to go back to", async () => {
    const first = withNote({ state: "withdrawn", version: "0.3.0", back_to: "0.2.0" });
    expect(
      await screen.findByText("The developers withdrew Dots 0.3.0. Go back to 0.2.0?"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Go back" })).toBeVisible();
    first.unmount();
    withNote({ state: "withdrawn", version: "0.3.0", back_to: null });
    expect(
      await screen.findByText(
        "The developers withdrew Dots 0.3.0. A fixed version will be offered when it is out.",
      ),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Go back" })).toBeNull();
  });

  it("after updating itself, says so once with What's new and Dismiss", async () => {
    backend.on("update_dismiss_updated", () => ({ state: "none" }));
    backend.on("update_release_notes", () => null);
    const { user } = withNote({ state: "updated_itself", version: "0.3.0", at: AT });
    expect(
      await screen.findByText(`Dots updated itself to 0.3.0 at ${localTime(AT)}.`),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "What's new" }));
    expect(await screen.findByRole("dialog", { name: "What's new in 0.3.0" })).toBeVisible();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(backend.callsTo("update_dismiss_updated")).toHaveLength(1));
    await waitFor(() => expect(screen.queryByText(/updated itself/)).toBeNull());
  });

  it("repair and go back waiting for runs say what they wait to do", async () => {
    const first = withNote({
      state: "waiting",
      version: "0.3.0",
      waiting_for: ["Daily digest"],
      purpose: "repair",
    });
    expect(await screen.findByText("Dots will repair itself when Daily digest finishes.")).toBeVisible();
    first.unmount();
    withNote({
      state: "waiting",
      version: "0.2.0",
      waiting_for: ["Daily digest"],
      purpose: "go_back",
    });
    expect(
      await screen.findByText("Dots will go back to 0.2.0 when Daily digest finishes."),
    ).toBeVisible();
  });
});

const withClient = (node: ReactNode) => (
  <QueryClientProvider client={createQueryClient()}>{node}</QueryClientProvider>
);

describe("the fatal page", () => {
  it("offers Go back when the start after an update failed", async () => {
    backend.on("update_go_back", () => ({
      state: "installing",
      version: "0.2.0",
      purpose: "go_back",
    }));
    const first = render(
      withClient(
        <FatalPage message="migration 0017 failed" logDir={null} backup={null} goBack={null} />,
      ),
    );
    expect(screen.queryByRole("button", { name: /Go back/ })).toBeNull();
    first.unmount();
    render(
      withClient(
        <FatalPage message="migration 0017 failed" logDir={null} backup={null} goBack="0.2.0" />,
      ),
    );
    const { default: userEvent } = await import("@testing-library/user-event");
    await userEvent.setup().click(screen.getByRole("button", { name: "Go back to 0.2.0" }));
    await waitFor(() => expect(backend.callsTo("update_go_back")).toHaveLength(1));
  });
});
