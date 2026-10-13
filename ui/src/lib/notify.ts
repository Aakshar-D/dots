import { toast } from "sonner";

import { CommandError } from "@/api/commands";

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/// Shows a failed action as a toast. A `cancelled` command is the user's own "no" to a native
/// confirmation, so it shows nothing.
export function notifyError(e: unknown): void {
  if (e instanceof CommandError && e.kind === "cancelled") return;
  toast.error(errorMessage(e));
}
