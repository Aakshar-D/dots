import type { ScheduleSkip } from "@/api/types";
import { localDateTime, localTime } from "@/lib/format";

/// A slot of the dot's schedule that came due while it was still working on the previous run,
/// as one muted line in the conversation. A skip from an earlier day says its date too.
export function SkipLine({ skip }: { skip: ScheduleSkip }) {
  const at = new Date(skip.slot_at);
  const today = at.toDateString() === new Date().toDateString();
  const when = today ? localTime(skip.slot_at) : localDateTime(skip.slot_at);
  return (
    <p className="text-center text-xs text-muted-foreground">
      Schedule skipped at {when}: still working on the previous run
    </p>
  );
}
