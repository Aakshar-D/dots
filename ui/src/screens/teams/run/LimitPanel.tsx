import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { WorkflowRun } from "@/api/types";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";

import { settle, StopRun } from "./actions";
import { budgetReached, joinAnd, stepName } from "./steps";

/// What a workflow run held at a limit waits for (section 9), with what the person does about
/// it: at the budget, what it spent of each limit it reached and **Add budget**, which raises
/// each of them once by its default amount (the core's choice: it takes no amounts); at the
/// send-back cap, the send-back it holds and **Allow one more send-back**. Then, unless `stop` is
/// false, Stop. Shown only while the run waits on one of the two.
export function LimitPanel({ run, stop = true }: { run: WorkflowRun; stop?: boolean }) {
  const qc = useQueryClient();
  const add = useMutation({
    mutationFn: () =>
      run.waiting_on === "budget"
        ? commands.workflowAddBudget(run.id)
        : commands.workflowAllowSendBack(run.id),
    onSuccess: (next) => settle(qc, next),
    onError: notifyError,
  });
  if (run.status !== "waiting") return null;
  if (run.waiting_on !== "budget" && run.waiting_on !== "send_backs") return null;

  const atBudget = run.waiting_on === "budget";
  const reached = budgetReached(run);
  const held = run.send_back?.held ? run.send_back : null;
  return (
    <section
      aria-label={atBudget ? "Budget reached" : "Send-back limit reached"}
      className="flex flex-col gap-2 rounded-lg border border-needs-border bg-needs-surface p-3 text-sm"
    >
      <p className="font-semibold text-needs">
        {atBudget ? "It hit its budget" : "It hit its send-back limit"}
      </p>
      {atBudget ? (
        reached.length > 0 ? (
          <p>
            It reached its {reached.length === 1 ? "limit" : "limits"}:{" "}
            {joinAnd(reached.map((l) => `${l.label.toLowerCase()} ${l.text}`))}. Add budget gives
            it {joinAnd(reached.map((l) => l.more))} more, once.
          </p>
        ) : (
          <p>It spent what its budget allows. Add budget raises the limits it reached.</p>
        )
      ) : (
        <>
          <p>
            The work was sent back {run.send_backs}{" "}
            {run.send_backs === 1 ? "time" : "times"}, and its limit is {run.budget.max_send_backs}.
          </p>
          {held && (
            <p>
              {held.by === "person"
                ? `You sent it back from ${stepName(run, held.from)} to ${stepName(run, held.to)}`
                : `${stepName(run, held.from)} asks to send it back to ${stepName(run, held.to)}`}
              : “{held.note}”
            </p>
          )}
        </>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={add.isPending} onClick={() => add.mutate()}>
          {atBudget ? "Add budget" : "Allow one more send-back"}
        </Button>
        {stop && <StopRun run={run} />}
      </div>
    </section>
  );
}
