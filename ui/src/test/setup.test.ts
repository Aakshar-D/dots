import { act, screen } from "@testing-library/react";
import { toast } from "sonner";
import { describe, expect, it } from "vitest";

import { renderApp } from "@/test/render";

// One check in two tests: the second looks at what the first left behind. sonner hands every
// new toaster the toasts that are still live, so without the clearing in setup.ts a test would
// see the toasts of the tests before it.
describe("the toasts between tests", () => {
  it("shows a toast", async () => {
    renderApp("/dots");
    toast.success("Left over");
    expect(await screen.findByText("Left over")).toBeVisible();
  });

  it("starts without the toast of the test before", async () => {
    expect(toast.getToasts()).toEqual([]);
    renderApp("/dots");
    await screen.findByRole("navigation", { name: "Main" });
    // A new toaster shows the toasts it is handed on a timer.
    await act(() => new Promise<void>((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByText("Left over")).toBeNull();
  });
});
