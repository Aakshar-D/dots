import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { ResultCheck, Run } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// "1 concern", "2 concerns".
export function concernsLabel(n: number): string {
  return n === 1 ? "1 concern" : `${n} concerns`;
}

/// What the badge says for a check.
export function checkLabel(check: ResultCheck): string {
  switch (check.state) {
    case "checking":
      return "Checking…";
    case "skipped":
      return "Check skipped";
    case "done":
      if (check.verdict === "looks_good") return "✓ Checked";
      if (check.verdict === "failed_task") return "⚠ May have missed the task";
      return `⚠ ${concernsLabel(check.concerns.length)}`;
  }
}

/// The result check of a run's answer (result check spec section 4), under the answer: Checking…
/// while it runs, ✓ Checked, Check skipped (with why on hover), or ⚠ and the number of concerns,
/// which opens the list and "Ask the dot to fix this". Nothing for a run without a check.
/// `canFix`: the fix may be sent now (the run's conversation is not busy).
export function ResultCheckBadge({ run, canFix = true }: { run: Run; canFix?: boolean }) {
  const check = run.result_check;
  const [open, setOpen] = useState(false);
  const qc = useQueryClient();
  const fix = useMutation({
    mutationFn: () => commands.resultCheckFix(run.id),
    onSuccess: () => {
      setOpen(false);
      invalidate(qc, ["thread"]);
      invalidate(qc, ["runs"]);
      invalidate(qc, keys.overview);
    },
  });
  if (!check || run.status !== "succeeded") return null;
  const label = checkLabel(check);
  if (check.state === "checking") {
    return (
      <Badge role="status" variant="secondary" className="self-start bg-muted text-muted-foreground">
        {label}
      </Badge>
    );
  }
  if (check.state === "skipped") {
    return (
      <Badge
        variant="secondary"
        className="self-start bg-muted text-muted-foreground"
        title={check.reason}
      >
        {label}
      </Badge>
    );
  }
  if (check.verdict === "looks_good") {
    const sure =
      check.confidence === null ? "" : ` (${Math.round(check.confidence * 100)}% sure)`;
    return (
      <Badge
        variant="secondary"
        className="self-start border-success/40 bg-transparent text-success"
        title={`A quick automatic check found no problems${sure}.`}
      >
        {label}
      </Badge>
    );
  }
  return (
    <div className="flex flex-col gap-2 self-start">
      <Button
        size="xs"
        variant="outline"
        className="self-start border-needs-border text-needs"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        {label}
      </Button>
      {open && (
        <section
          aria-label="Result check concerns"
          className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
        >
          <p className="text-xs text-muted-foreground">
            A quick automatic check of this answer
            {check.verdict === "failed_task" ? " says it may not have done the task:" : " raised:"}
          </p>
          <ul className="list-disc pl-5">
            {check.concerns.map((concern, i) => (
              <li key={i}>{concern}</li>
            ))}
          </ul>
          <Button
            size="sm"
            variant="outline"
            className="self-start"
            disabled={!canFix || fix.isPending || fix.isSuccess}
            onClick={onSingleClick(() => fix.mutate())}
          >
            Ask the sol to fix this
          </Button>
          {!canFix && (
            <p className="text-xs text-muted-foreground">
              Wait until the sol has finished in this conversation.
            </p>
          )}
          {fix.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(fix.error)}
            </p>
          )}
        </section>
      )}
    </div>
  );
}
