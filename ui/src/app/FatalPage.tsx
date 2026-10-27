import { useMutation, useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/notify";
import { keys } from "@/lib/query";

/// Where every version's installer is (updates spec §8).
const RELEASES = "https://github.com/Aakshar-D/dots/releases";

/// Shown instead of the app when the runtime could not start (spec §8). There is no tray in
/// that state: closing the window exits Dots.
export function FatalPage(props: {
  message: string;
  logDir: string | null;
  backup: string | null;
  /// The version Go back installs when this start followed a failed update.
  goBack: string | null;
}) {
  const goBack = useMutation({ mutationFn: commands.updateGoBack });
  // Why the last install or go back did not happen (deferred minor M-7).
  const installError = useQuery({
    queryKey: keys.updateStatus,
    queryFn: commands.updateStatus,
    retry: false,
  }).data?.install_error;
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
      {installError && (
        <p role="alert" className="text-sm text-destructive">
          {installError}
        </p>
      )}
      {props.goBack && (
        <div className="flex flex-col items-start gap-2">
          <p className="text-sm">
            This start followed an update. Go back to {props.goBack}? Your data from before the
            update comes back with it; anything done since the update is lost.
          </p>
          <Button size="sm" disabled={goBack.isPending} onClick={() => goBack.mutate()}>
            Go back to {props.goBack}
          </Button>
          {goBack.isError && (
            <p role="alert" className="text-sm text-destructive">
              {errorMessage(goBack.error)}
            </p>
          )}
          {goBack.isSuccess && (
            <p role="status" className="text-sm">
              Going back to Dots {props.goBack}. Dots restarts in a moment.
            </p>
          )}
        </div>
      )}
      <p className="text-sm text-muted-foreground">
        Close this window to quit. Dots tries again the next time you start it.
      </p>
    </main>
  );
}
