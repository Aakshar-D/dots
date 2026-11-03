import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { Run } from "@/api/types";
import { Button } from "@/components/ui/button";
import { deliveryLine } from "@/lib/delivery";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { useNow } from "@/lib/use-now";

/// Whether a message got to its dot: how long it has waited and why, or that it did not get
/// there, with Retry and Discard for a chat message. Nothing for a message that is fine.
export function DeliveryLine({ run }: { run: Run }) {
  const qc = useQueryClient();
  const now = useNow(run.delivery === "waiting");
  const settle = () => {
    invalidate(qc, ["runs"]);
    invalidate(qc, ["thread"]);
    invalidate(qc, keys.overview);
  };
  const retry = useMutation({ mutationFn: commands.retryDelivery, onSettled: settle });
  const discard = useMutation({ mutationFn: commands.discardDelivery, onSettled: settle });
  const line = deliveryLine(run, now);
  if (!line) return null;

  if (line.kind === "waiting") {
    return <p className="text-xs text-muted-foreground">{line.text}</p>;
  }
  if (line.kind === "dismissed") {
    return <p className="text-xs text-muted-foreground">{line.text}</p>;
  }
  // Each press clears the other's error, so the alert shows the latest failure only.
  const press = (own: typeof retry, other: typeof retry) => {
    other.reset();
    own.mutate(run.id);
  };
  const busy = retry.isPending || discard.isPending;
  const failed = retry.isError ? retry : discard.isError ? discard : null;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="text-needs">{line.text}</span>
        {run.trigger === "chat" && (
          <>
            <Button size="xs" variant="outline" disabled={busy} onClick={() => press(retry, discard)}>
              Retry
            </Button>
            <Button size="xs" variant="ghost" disabled={busy} onClick={() => press(discard, retry)}>
              Discard
            </Button>
          </>
        )}
      </div>
      {failed && (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage(failed.error)}
        </p>
      )}
    </div>
  );
}
