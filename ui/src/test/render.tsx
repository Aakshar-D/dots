import { QueryClientProvider } from "@tanstack/react-query";
import { render, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createMemoryRouter, RouterProvider } from "react-router";
import { expect } from "vitest";

import { routes } from "@/app/routes";
import { createQueryClient } from "@/lib/query";

import { backend } from "./backend";

/// Renders the whole app at `path`, against the fake backend.
export function renderApp(path = "/home") {
  const queryClient = createQueryClient();
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  /// Resolves once the app listens to the backend and has its answers to what it asked again at
  /// that moment (lib/runtime-events.ts): the frame's Inbox was loaded twice and nothing is on
  /// its way. A test that counts the calls of a query the first screen makes waits for this.
  const listening = () =>
    waitFor(() => {
      expect(backend.callsTo("inbox").length).toBeGreaterThan(1);
      expect(queryClient.isFetching()).toBe(0);
    });
  return { ...view, router, queryClient, listening, user: userEvent.setup() };
}
