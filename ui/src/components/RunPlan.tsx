import { useId, useState, type ReactNode } from "react";
import { cn } from "cn";
import { ChevronRight, LoaderCircle } from "lucide-react";

import type { Run, RunEventRecord } from "@/api/types";
import { duration, localTime } from "@/lib/format";
import { isActive } from "@/lib/home-topics";
import { mcpToolLabel } from "@/lib/mcp-tool";
import {
  ownerName,
  planProgress,
  runPlans,
  runTimeline,
  worthShowing,
  type EntryStatus,
  type Plan,
  type PlanStep,
  type TimelineEntry,
} from "@/lib/run-plan";
import { callLabel, toolPreview } from "@/lib/transcript";

/// The marks of a step, in the look of a workflow card's steps (teams spec section 10.4): a
/// distinct shape for each state, so it reads without its colour, and its word for assistive
/// technology.
type Mark = "done" | "running" | "todo" | "failed" | "waiting" | "stopped";

const MARK: Record<Mark, { glyph: string; className: string }> = {
  done: { glyph: "✓", className: "text-success" },
  running: { glyph: "", className: "text-primary" },
  todo: { glyph: "○", className: "text-muted-foreground" },
  failed: { glyph: "✕", className: "text-destructive" },
  waiting: { glyph: "●", className: "text-needs" },
  stopped: { glyph: "–", className: "text-muted-foreground" },
};

function StepGlyph({ mark }: { mark: Mark }) {
  const look = MARK[mark];
  return mark === "running" ? (
    <LoaderCircle
      aria-hidden
      className={cn(
        "mt-0.5 size-3.5 shrink-0 animate-spin motion-reduce:animate-none",
        look.className,
      )}
    />
  ) : (
    <span aria-hidden className={cn("w-3.5 shrink-0 text-center", look.className)}>
      {look.glyph}
    </span>
  );
}

/// A plan step's mark and words: in a run that has ended, a step still being worked on stopped
/// there and one not started was never reached.
function planMark(step: PlanStep, active: boolean): { mark: Mark; label: string } {
  switch (step.status) {
    case "completed":
      return { mark: "done", label: "Done" };
    case "in_progress":
      return active
        ? { mark: "running", label: "Working on it" }
        : { mark: "stopped", label: "Stopped" };
    case "pending":
      return active ? { mark: "todo", label: "To do" } : { mark: "todo", label: "Not started" };
  }
}

const ENTRY: Record<EntryStatus, { mark: Mark; label: string }> = {
  done: { mark: "done", label: "Done" },
  running: { mark: "running", label: "Running" },
  failed: { mark: "failed", label: "Failed" },
  waiting: { mark: "waiting", label: "Waiting for your approval" },
  stopped: { mark: "stopped", label: "No result" },
};

/// "3 of 7 done", with a thin bar that says the same to assistive technology.
function Progress({ done, total, label }: { done: number; total: number; label: string }) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={total}
      aria-valuenow={done}
      aria-valuetext={`${done} of ${total} done`}
      className="h-1 w-full overflow-hidden rounded-full bg-muted"
    >
      <div
        className="h-full rounded-full bg-primary transition-[width] motion-reduce:transition-none"
        style={{ width: `${total === 0 ? 0 : (done / total) * 100}%` }}
      />
    </div>
  );
}

/// One owner's plan: who keeps it, how far it is, and its steps in order, the one being worked on
/// in its "-ing" words.
function PlanList(props: { plan: Plan; active: boolean; dotName: string; nested: boolean }) {
  const { plan, active } = props;
  const { done, total } = planProgress(plan.steps);
  const who = ownerName(plan.owner, props.dotName);
  return (
    <div
      role="group"
      aria-label={`Plan of ${who}`}
      className={cn("flex flex-col gap-1.5", props.nested && "rounded-lg border border-dashed p-2")}
    >
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-medium text-foreground">{who}</span>
        <span>
          {done} of {total} done
        </span>
      </p>
      <ol className="flex flex-col gap-1 text-sm">
        {plan.steps.map((step, i) => {
          const { mark, label } = planMark(step, active);
          const now = mark === "running";
          return (
            <li
              key={`${i}-${step.content}`}
              data-status={step.status}
              title={label}
              className={cn(
                "flex items-start gap-2",
                step.status === "completed" && "text-muted-foreground",
                now && "font-medium",
              )}
            >
              <StepGlyph mark={mark} />
              <span className="min-w-0 flex-1">
                {now ? `${step.activeForm}…` : step.content}
                <span className="sr-only">: {label}</span>
              </span>
              {now && <span className="shrink-0 text-xs text-primary">Now</span>}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/// One step of the timeline: its time, its mark, who did it when not the dot, what it did, how
/// long it took, and its calls when opened.
function EntryRow(props: {
  entry: TimelineEntry;
  dotName: string;
  owners: ReadonlyMap<string, string>;
}) {
  const { entry, owners } = props;
  const { mark, label } = ENTRY[entry.status];
  const took = entry.end && entry.kind !== "milestone" ? duration(entry.start, entry.end) : "";
  const line = (
    <>
      <span className="w-12 shrink-0 text-muted-foreground tabular-nums">
        {localTime(entry.start)}
      </span>
      <StepGlyph mark={mark} />
      <span className="min-w-0 flex-1">
        {entry.owner.kind === "helper" && (
          <span className="mr-1 rounded-full bg-muted px-1.5 py-px text-[0.7rem]">
            {ownerName(entry.owner, props.dotName)}
          </span>
        )}
        {entry.text}
        <span className="sr-only">: {label}</span>
      </span>
      {took && <span className="shrink-0 text-muted-foreground tabular-nums">{took}</span>}
    </>
  );
  if (entry.calls.length === 0) {
    return (
      <li data-kind={entry.kind} className="flex items-start gap-2 text-xs">
        {line}
      </li>
    );
  }
  return (
    <li data-kind={entry.kind} className="text-xs">
      <details className="group/entry">
        <summary className="flex cursor-pointer list-none items-start gap-2 [&::-webkit-details-marker]:hidden">
          {line}
        </summary>
        <ul className="mt-1 ml-14 flex flex-col gap-1 border-l pl-2">
          {entry.calls.map((call, i) => (
            <li key={i} className="flex flex-col gap-0.5">
              <span className="font-medium">
                {mcpToolLabel(callLabel(call.tool, call.input), owners)}
              </span>
              <span className="truncate font-mono text-muted-foreground">
                {toolPreview(call.input)}
              </span>
              {call.result && (
                <pre
                  className={cn(
                    "max-h-40 overflow-auto whitespace-pre-wrap",
                    call.result.isError ? "text-destructive" : "text-muted-foreground",
                  )}
                >
                  {call.result.output}
                </pre>
              )}
            </li>
          ))}
        </ul>
      </details>
    </li>
  );
}

/// A disclosure button of the card: a chevron that turns, then its words.
function Toggle(props: {
  open: boolean;
  onToggle: () => void;
  /// The id of what it opens, while that is in the page.
  controls: string | undefined;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-expanded={props.open}
      aria-controls={props.controls}
      onClick={props.onToggle}
      className="flex min-w-0 items-center gap-1 text-left"
    >
      <ChevronRight
        aria-hidden
        className={cn(
          "size-3.5 shrink-0 transition-transform motion-reduce:transition-none",
          props.open && "rotate-90",
        )}
      />
      {props.children}
    </button>
  );
}

/// A run's plan and timeline (run plan spec section 2), live: the to-do list the dot keeps (and
/// each helper's, nested and named), how far it is, and a timeline of what the run did with times
/// and durations. A plan is open while the run works and one compact line once it has ended; the
/// person's own choice holds after that. A run that kept no plan shows the timeline alone, closed
/// (the transcript already shows its steps), and a quick one shows nothing. What is closed is not
/// in the page, so the transcript's lines are not there twice.
export function RunPlan(props: {
  run: Run;
  events: RunEventRecord[];
  dotName: string;
  /// MCP server name to the plugin it comes with.
  owners: ReadonlyMap<string, string>;
}) {
  const { run, events, dotName, owners } = props;
  const active = isActive(run);
  const [chosen, setChosen] = useState<boolean | null>(null);
  const [timelineChosen, setTimelineChosen] = useState<boolean | null>(null);
  const bodyId = useId();
  const timelineId = useId();
  const plans = runPlans(events);
  const timeline = runTimeline(events, active, owners);
  if (!worthShowing(plans, timeline)) return null;
  const own = plans.find((p) => p.owner.kind === "dot");
  const all = plans.flatMap((p) => p.steps);
  const { done, total } = planProgress(all);
  const hasPlan = total > 0;
  const open = chosen ?? (active && hasPlan);
  // Without a plan, the card's own toggle opens the timeline.
  const timelineOpen = timelineChosen ?? !hasPlan;
  const current = active ? all.find((s) => s.status === "in_progress") : undefined;
  const took = run.started_at ? duration(run.started_at, run.ended_at) : "";
  const state = active ? "Working" : took ? `Took ${took}` : "";
  const title = hasPlan ? "Plan" : "Timeline";
  return (
    <section
      aria-label={`${title} of ${dotName}`}
      data-slot="run-plan"
      data-open={open}
      className={cn(
        "flex flex-col gap-2 rounded-xl border bg-card px-3.5 py-2.5",
        active && "border-primary/40",
      )}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
        <Toggle open={open} onToggle={() => setChosen(!open)} controls={open ? bodyId : undefined}>
          <span className="font-semibold">{title}</span>
        </Toggle>
        {hasPlan && (
          <span className="text-xs">
            {done} of {total} done
          </span>
        )}
        {!hasPlan && (
          <span className="text-xs text-muted-foreground">
            {timeline.length === 1 ? "1 step" : `${timeline.length} steps`}
          </span>
        )}
        <span className="flex-1" />
        {state && <span className="text-xs text-muted-foreground">{state}</span>}
      </div>
      {/* Progress is announced as it changes, not every step's words. */}
      <p aria-live="polite" className="sr-only">
        {hasPlan ? `${done} of ${total} steps done` : ""}
      </p>
      {hasPlan && <Progress done={done} total={total} label={`Progress of ${dotName}'s plan`} />}
      {!open && current && (
        <p className="truncate text-xs text-muted-foreground">Now: {current.activeForm}…</p>
      )}
      {open && (
        <div id={bodyId} className="flex flex-col gap-3">
          {plans.map((plan) => (
            <PlanList
              key={plan.owner.kind === "dot" ? "dot" : plan.owner.id}
              plan={plan}
              active={active}
              dotName={dotName}
              nested={plan.owner.kind === "helper" && own !== undefined}
            />
          ))}
          {timeline.length > 0 && (
            <div className="flex flex-col gap-1.5">
              {hasPlan && (
                <div className="text-xs text-muted-foreground">
                  <Toggle
                    open={timelineOpen}
                    onToggle={() => setTimelineChosen(!timelineOpen)}
                    controls={timelineOpen ? timelineId : undefined}
                  >
                    Timeline · {timeline.length === 1 ? "1 step" : `${timeline.length} steps`}
                  </Toggle>
                </div>
              )}
              {timelineOpen && (
                <ol id={timelineId} aria-label="Timeline" className="flex flex-col gap-1">
                  {timeline.map((entry) => (
                    <EntryRow
                      key={`${entry.kind}-${entry.key}-${entry.text}`}
                      entry={entry}
                      dotName={dotName}
                      owners={owners}
                    />
                  ))}
                </ol>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
