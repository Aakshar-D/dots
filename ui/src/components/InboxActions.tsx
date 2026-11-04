import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Check } from "lucide-react";
import { createContext, useContext, useRef } from "react";

import { commands } from "@/api/commands";
import type { InboxKey } from "@/api/types";
import { LaterMenu } from "@/components/LaterMenu";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// Told, after Snooze or Done went through, the id of the heading of the section the item was
/// in (null outside a section). The Inbox moves focus there once the item is gone, so it does
/// not fall to the page's body.
export const InboxActed = createContext<(headingId: string | null) => void>(() => {});

/// **Snooze** and **Done** at the end of an Inbox row or card (snooze and done spec section 6).
/// Snooze hides every one of `items` (an approvals group snoozes each approval it shows); Done
/// is for one item, and not for approvals (`done` false). `what` names the item in the
/// buttons' accessible names ("Snooze: courier's question"), so each row's buttons differ.
export function InboxActions({
  items,
  done,
  what,
}: {
  items: InboxKey[];
  done: boolean;
  what: string;
}) {
  const qc = useQueryClient();
  const acted = useContext(InboxActed);
  const here = useRef<HTMLDivElement>(null);
  // The Inbox, and the counts under the overview (the dots' badges and the Home total).
  const reload = () => {
    for (const queryKey of [keys.inbox, keys.overview]) invalidate(qc, queryKey);
  };
  const section = () =>
    here.current?.closest("section")?.getAttribute("aria-labelledby") ?? null;
  const snooze = useMutation({
    mutationFn: (at: Date) => commands.inboxSnooze(items, at.toISOString()),
    onSuccess: () => acted(section()),
    onError: notifyError,
    onSettled: reload,
  });
  const markDone = useMutation({
    mutationFn: () => commands.inboxDone(items[0]),
    onSuccess: () => acted(section()),
    onError: notifyError,
    onSettled: reload,
  });
  const busy = snooze.isPending || markDone.isPending;
  return (
    <div ref={here} className="flex shrink-0 items-center gap-1">
      <LaterMenu
        label="Snooze"
        name={`Snooze: ${what}`}
        disabled={busy}
        onPick={(at) => snooze.mutate(at)}
      />
      {done && (
        <Button
          size="xs"
          variant="ghost"
          disabled={busy}
          aria-label={`Done: ${what}`}
          onClick={onSingleClick(() => markDone.mutate())}
        >
          <Check aria-hidden />
          Done
        </Button>
      )}
    </div>
  );
}
