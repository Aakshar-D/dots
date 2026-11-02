import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { Run } from "@/api/types";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

const MINUTE_MS = 60_000;

/// A running run that has gone quiet: how long it has been quiet, when it stops by itself, and
/// Stop or Keep waiting. The minutes come from the limit the run started with
/// (`run.quiet_limit_secs`), not from the dot's current setting. The card shows nothing for any
/// other run. The core clears `quiet_since` when the run speaks again or ends, which removes it.
export function QuietCard({ run }: { run: Run }) {
  const qc = useQueryClient();
  const quiet = run.status === "running" && !!run.quiet_since && !!run.quiet_limit_secs;
  // The minutes move with the clock; a half-minute tick is fine for minute figures.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!quiet) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, [quiet]);
  const stop = useMutation({ mutationFn: commands.cancelRun });
  const keep = useMutation({
    mutationFn: commands.keepWaiting,
    onSuccess: () => {
      invalidate(qc, ["thread"]);
      invalidate(qc, ["runs"]);
      invalidate(qc, keys.overview);
    },
  });
  if (!quiet || !run.quiet_since || !run.quiet_limit_secs) return null;

  const since = Date.parse(run.quiet_since);
  const limitMin = run.quiet_limit_secs / 60;
  const quietMin = limitMin + Math.max(0, Math.floor((now - since) / MINUTE_MS));
  const stopsInMin = Math.max(
    0,
    Math.ceil((since + run.quiet_limit_secs * 1000 - now) / MINUTE_MS),
  );
  const busy = stop.isPending || keep.isPending;
  return (
    <article
      aria-label="Quiet run"
      className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
    >
      <header className="flex items-baseline gap-2">
        <span className="font-semibold text-needs">Quiet for {quietMin} min</span>
        <span className="text-xs text-muted-foreground">Stops by itself in {stopsInMin} min</span>
      </header>
      <div className="flex gap-2">
        <Button size="sm" disabled={busy} onClick={() => stop.mutate(run.id)}>
          Stop
        </Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={() => keep.mutate(run.id)}>
          Keep waiting
        </Button>
      </div>
      {(stop.isError || keep.isError) && (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage(stop.isError ? stop.error : keep.error)}
        </p>
      )}
    </article>
  );
}
