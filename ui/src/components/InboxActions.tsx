import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check } from "lucide-react";

import { commands } from "@/api/commands";
import type { InboxKey } from "@/api/types";
import { LaterMenu } from "@/components/LaterMenu";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// **Snooze** and **Done** at the end of an Inbox row or card (snooze and done spec section 6).
/// Snooze hides every one of `items` (an approvals group snoozes each approval it shows); Done
/// is for one item, and not for approvals (`done` false).
export function InboxActions({ items, done }: { items: InboxKey[]; done: boolean }) {
  const qc = useQueryClient();
  // The Inbox, and the counts under the overview (the dots' badges and the Home total).
  const reload = () => {
    for (const queryKey of [keys.inbox, keys.overview]) invalidate(qc, queryKey);
  };
  const snooze = useMutation({
    mutationFn: (at: Date) => commands.inboxSnooze(items, at.toISOString()),
    onError: notifyError,
    onSettled: reload,
  });
  const markDone = useMutation({
    mutationFn: () => commands.inboxDone(items[0]),
    onError: notifyError,
    onSettled: reload,
  });
  const busy = snooze.isPending || markDone.isPending;
  return (
    <div className="flex shrink-0 items-center gap-1">
      <LaterMenu label="Snooze" disabled={busy} onPick={(at) => snooze.mutate(at)} />
      {done && (
        <Button
          size="xs"
          variant="ghost"
          disabled={busy}
          onClick={onSingleClick(() => markDone.mutate())}
        >
          <Check aria-hidden />
          Done
        </Button>
      )}
    </div>
  );
}
