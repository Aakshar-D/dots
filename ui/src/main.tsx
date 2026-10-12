import "./index.css";

import { QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";

import { routes } from "@/app/routes";
import { createQueryClient } from "@/lib/query";

const queryClient = createQueryClient();
// Hash routes: the app is loaded from a file-like origin, not from a server that could
// answer deep links.
const router = createHashRouter(routes);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
