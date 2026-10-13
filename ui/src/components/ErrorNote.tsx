import { errorMessage } from "@/lib/notify";

/// A failed load, shown where the data would be.
export function ErrorNote({ error }: { error: unknown }) {
  return (
    <p role="alert" className="rounded-md border border-destructive/50 p-3 text-destructive">
      {errorMessage(error)}
    </p>
  );
}

/// The failed loads among `queries`, shown where their data would be. A thread asks the same
/// thing once per run and one cause fails them all, so each message is shown once.
export function LoadErrors({ queries }: { queries: { isError: boolean; error: unknown }[] }) {
  const messages = new Set(queries.filter((q) => q.isError).map((q) => errorMessage(q.error)));
  return [...messages].map((message) => <ErrorNote key={message} error={message} />);
}
