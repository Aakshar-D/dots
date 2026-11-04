import type { ScheduleSkip } from "@/api/types";
import { localDayTime } from "@/lib/format";

/// A slot of the dot's schedule that came due while it was still working on the previous run,
/// as one muted line in the conversation. A skip from an earlier day says its date too.
export function SkipLine({ skip }: { skip: ScheduleSkip }) {
  return (
    <p className="text-center text-xs text-muted-foreground">
      Schedule skipped at {localDayTime(skip.slot_at)}: still working on the previous run
    </p>
  );
}
