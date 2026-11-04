import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useId, useState } from "react";

import { commands } from "@/api/commands";
import type { InboxMark } from "@/api/types";
import { Button } from "@/components/ui/button";
import { relativeTime } from "@/lib/format";
import { keyString } from "@/lib/inbox";
import { backText } from "@/lib/later";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// The collapsed **Snoozed** or **Done** list at the bottom of the Inbox (snooze and done spec
/// section 6): closed at first, and as the person left it while the screen is open. Nothing
/// when it is empty.
export function MarkedList({ title, marks }: { title: "Snoozed" | "Done"; marks: InboxMark[] }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  if (marks.length === 0) return null;
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <section className="flex flex-col gap-2">
      <Button
        variant="ghost"
        size="sm"
        className="self-start"
        aria-expanded={open}
        // The list is in the page only while open.
        aria-controls={open ? id : undefined}
        onClick={() => setOpen(!open)}
      >
        <Chevron aria-hidden />
        {title} ({marks.length})
      </Button>
      {open && (
        <ul id={id} aria-label={title} className="flex flex-col gap-1">
          {marks.map((mark) => (
            <MarkedRow key={keyString(mark)} mark={mark} />
          ))}
        </ul>
      )}
    </section>
  );
}

/// A snoozed item with when it comes back and **Unsnooze**, or a done one with when it was
/// done and **Undo**.
function MarkedRow({ mark }: { mark: InboxMark }) {
  const qc = useQueryClient();
  const key = { kind: mark.kind, id: mark.id };
  const snoozed = mark.snoozed_until !== null;
  const undo = useMutation({
    mutationFn: () => (snoozed ? commands.inboxUnsnooze(key) : commands.inboxUndoDone(key)),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.inbox, keys.overview]) invalidate(qc, queryKey);
    },
  });
  const when = snoozed
    ? backText(mark.snoozed_until!, Date.now())
    : `Done ${relativeTime(mark.done_at)}`;
  return (
    <li className="flex items-center gap-3 rounded-lg border px-3 py-2">
      <span className="min-w-0 flex-1 truncate">
        {mark.title} · {when}
      </span>
      <Button
        size="xs"
        variant="outline"
        disabled={undo.isPending}
        aria-label={`${snoozed ? "Unsnooze" : "Undo"}: ${mark.title}`}
        onClick={onSingleClick(() => undo.mutate())}
      >
        {snoozed ? "Unsnooze" : "Undo"}
      </Button>
    </li>
  );
}
