/// Where every version's installer is (updates spec §8).
const RELEASES = "https://github.com/Aakshar-D/dots/releases";

/// Shown instead of the app when the runtime could not start (spec §8). There is no tray in
/// that state: closing the window exits Dots.
export function FatalPage(props: {
  message: string;
  logDir: string | null;
  backup: string | null;
}) {
  return (
    <main className="mx-auto flex h-screen max-w-2xl flex-col justify-center gap-4 p-8">
      <h1 className="text-xl font-semibold">Dots could not start</h1>
      <p role="alert" className="rounded-md border border-destructive/50 p-3 font-mono text-sm">
        {props.message}
      </p>
      {props.logDir && (
        <p className="text-sm text-muted-foreground">
          The log folder has the details:{" "}
          <span className="font-mono text-foreground select-all">{props.logDir}</span>
        </p>
      )}
      {props.backup && (
        <p className="text-sm text-muted-foreground">
          If this began after an update, the data from before it is backed up in{" "}
          <span className="font-mono text-foreground select-all">{props.backup}</span>. The
          previous version's installer is on{" "}
          <span className="font-mono text-foreground select-all">{RELEASES}</span>.
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        Close this window to quit. Dots tries again the next time you start it.
      </p>
    </main>
  );
}
