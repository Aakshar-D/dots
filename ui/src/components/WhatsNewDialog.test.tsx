import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { WhatsNewDialog } from "@/components/WhatsNewDialog";
import { createQueryClient } from "@/lib/query";
import { backend } from "@/test/backend";

function open(version: string) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <WhatsNewDialog version={version} open onOpenChange={() => {}} />
    </QueryClientProvider>,
  );
}

describe("What's new", () => {
  it("shows the release's notes", async () => {
    backend.on("update_release_notes", () => ({
      version: "0.3.0",
      body: "- Dots holds background runs when your Claude usage is high.",
      url: "https://github.com/Aakshar-D/dots/releases/tag/v0.3.0",
    }));
    open("0.3.0");
    expect(await screen.findByRole("dialog", { name: "What's new in 0.3.0" })).toBeVisible();
    expect(
      await screen.findByText("Dots holds background runs when your Claude usage is high."),
    ).toBeVisible();
    expect(backend.callsTo("update_release_notes")).toEqual([{ version: "0.3.0" }]);
  });

  it("says when a version has no notes", async () => {
    backend.on("update_release_notes", () => null);
    open("0.3.0");
    expect(await screen.findByText("No notes for 0.3.0.")).toBeVisible();
  });
});
