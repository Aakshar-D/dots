import { errorMessage } from "@/lib/notify";

/// A failed load, shown where the data would be.
export function ErrorNote({ error }: { error: unknown }) {
  return (
    <p role="alert" className="rounded-md border border-destructive/50 p-3 text-destructive">
      {errorMessage(error)}
    </p>
  );
}
