import type { ReactNode } from "react";
import { Link } from "react-router";
import { z } from "zod";

import { LIMITS, type Run, type RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
import { isActive } from "@/lib/chat-threads";
import { localDateTime } from "@/lib/format";
import { toolPreview, transcriptItems, type TranscriptItem } from "@/lib/transcript";

const TRIGGER: Record<Run["trigger"], string> = {
  schedule: "Scheduled run",
  webhook: "Webhook run",
  manual: "Manual run",
  chat: "Chat message",
  resume: "Continued after approval",
};

/// What the person wrote, for a chat run.
function chatMessage(run: Run): string | null {
  if (run.trigger !== "chat" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const message = (run.payload as Record<string, unknown>).message;
  return typeof message === "string" ? message : null;
}

/// What the scheduler wrote into the payload of a run it started: the slot, whether the slot
/// had been missed, and which attempt of the occurrence the run is (a first one has none).
const SchedulePayload = z.object({
  scheduled_for: z.string(),
  caught_up: z.boolean().optional(),
  attempt: z.number().int().optional(),
});

/// The line under a scheduled run's trigger: the slot it served, or the attempt it retries,
/// with a link to the run that failed. Nothing for another trigger or a payload it cannot read.
function ScheduleLine({ run }: { run: Run }) {
  if (run.trigger !== "schedule") return null;
  const payload = SchedulePayload.safeParse(run.payload);
  if (!payload.success) return null;
  const { scheduled_for: slot, caught_up: caughtUp, attempt } = payload.data;
  let line: ReactNode = `Scheduled for ${localDateTime(slot)}`;
  if (caughtUp) line = `${line}, caught up after the PC was off`;
  const of = LIMITS.max_schedule_attempts;
  if (attempt !== undefined && attempt > 1) {
    // A retry names the run it retries. A slot that dropped a pending retry carries its attempt
    // without `retry_of`: it still serves its own slot.
    line =
      run.retry_of === null ? (
        `${line} · attempt ${attempt} of ${of}`
      ) : (
        <>
          Retry {attempt} of {of} of{" "}
          <Link to={`/runs/${run.retry_of}`} className="text-foreground hover:underline">
            the failed run
          </Link>
        </>
      );
  }
  return <p className="mb-2 text-xs text-muted-foreground">{line}</p>;
}

/// `active`: the item's run is still going. Only then can a tool call without a result be one
/// that runs; in a run that has ended, the result never came.
function Item({ item, active }: { item: TranscriptItem; active: boolean }) {
  switch (item.type) {
    case "text":
      return <Markdown text={item.text} />;
    case "error":
      return (
        <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
          {item.text}
        </p>
      );
    case "raw":
      return (
        <details className="rounded-md border px-2 py-1 text-muted-foreground">
          <summary className="cursor-pointer text-xs">{item.label}</summary>
          <pre className="mt-1 overflow-x-auto text-xs whitespace-pre-wrap">{item.text}</pre>
        </details>
      );
    case "tool":
      return (
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="font-semibold">{item.tool}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
              {toolPreview(item.input)}
            </span>
            <span className={item.result?.isError ? "text-destructive" : "text-muted-foreground"}>
              {item.result === null
                ? active
                  ? "running"
                  : "no result"
                : item.result.isError
                  ? "failed"
                  : "done"}
            </span>
          </summary>
          <pre className="mt-1 overflow-x-auto text-xs whitespace-pre-wrap">
            {JSON.stringify(item.input, null, 2)}
          </pre>
          {item.result && (
            <pre
              className={`mt-1 max-h-80 overflow-auto border-t pt-1 text-xs whitespace-pre-wrap ${
                item.result.isError ? "text-destructive" : "text-muted-foreground"
              }`}
            >
              {item.result.output}
            </pre>
          )}
        </details>
      );
  }
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
  runs: Run[];
  events: Map<string, RunEventRecord[]>;
  /// The last run's error, when the screen shows it by itself. A failed run stores its error as
  /// an event too: the one of the last run that says the same is left out, so that the error is
  /// on screen once.
  shownError?: string | null;
}) {
  const many = props.runs.length > 1;
  return (
    <div className="flex flex-col gap-4">
      {props.runs.map((run, i) => {
        const message = chatMessage(run);
        const last = i === props.runs.length - 1;
        const items = transcriptItems(props.events.get(run.id) ?? []).filter(
          (item) => !(last && item.type === "error" && item.text === props.shownError),
        );
        return (
          <section key={run.id} aria-label={`${TRIGGER[run.trigger]}, ${localDateTime(run.queued_at)}`}>
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{TRIGGER[run.trigger]}</span>
              <span>{localDateTime(run.queued_at)}</span>
              {many && <StatusBadge status={run.status} />}
            </header>
            <ScheduleLine run={run} />
            <div className="flex flex-col gap-2">
              {message !== null && (
                <p className="self-end rounded-lg bg-primary px-3 py-1.5 whitespace-pre-wrap text-primary-foreground">
                  {message}
                </p>
              )}
              {items.map((item) => (
                <Item key={item.seq} item={item} active={isActive(run)} />
              ))}
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}
