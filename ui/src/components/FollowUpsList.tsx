import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// A dot's pending follow-ups, soonest first, each with its local time, its note and Cancel
/// (check-ins spec section 9). The dot schedules them itself; only the person cancels one.
export function FollowUpsList({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.followUps(dotId),
    queryFn: () => commands.followUpsList(dotId),
  });
  const cancel = useMutation({
    mutationFn: (id: string) => commands.followUpCancel(id),
    onError: notifyError,
    // The Dots list shows the next one too. The core also announces the change.
    onSettled: () => invalidate(qc, ["follow-ups"]),
  });
  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  if (list.data.length === 0) {
    return (
      <p className="text-muted-foreground">
        No follow-up waits. A dot schedules one with its follow-up tool when its work has to
        wait.
      </p>
    );
  }
  return (
    <ul aria-label="Follow-ups" className="flex flex-col gap-2">
      {list.data.map((followUp) => (
        <li
          key={followUp.id}
          aria-label={`Follow-up due ${localDateTime(followUp.due_at)}`}
          className="flex items-center gap-3 rounded-lg border p-3"
        >
          <div className="flex min-w-0 flex-1 flex-col">
            <span className="font-medium">{localDateTime(followUp.due_at)}</span>
            <span className="truncate text-xs text-muted-foreground">{followUp.note}</span>
          </div>
          <Button
            size="sm"
            variant="outline"
            disabled={cancel.isPending}
            onClick={onSingleClick(() => cancel.mutate(followUp.id))}
          >
            Cancel
          </Button>
        </li>
      ))}
    </ul>
  );
}
