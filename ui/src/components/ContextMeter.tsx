import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";

import { commands } from "@/api/commands";
import type { ContextChoice, ContextEvent, ContextRules, Run } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { CONTEXT_RULE } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// 92000 → "92k", 1000000 → "1M", 1500000 → "1.5M", 800 → "800".
export function tokensShort(n: number): string {
  if (n >= 1_000_000) return `${Math.round(n / 100_000) / 10}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

/// `fill` as a whole percent of `window`.
export const contextPct = (fill: number, window: number) =>
  window > 0 ? Math.round((fill * 100) / window) : 0;

export type ContextLevel = "ok" | "compact" | "clear";

/// Where `fill` stands against the rules: under compact, at or above compact, at or above clear.
export function contextLevel(fill: number, window: number, rules: ContextRules): ContextLevel {
  if (fill * 100 >= window * rules.clear_at) return "clear";
  if (fill * 100 >= window * rules.compact_at) return "compact";
  return "ok";
}

/// "Context 92k / 200k (46%)"
export const contextText = (fill: number, window: number) =>
  `Context ${tokensShort(fill)} / ${tokensShort(window)} (${contextPct(fill, window)}%)`;

/// How full a session's context is (context rules spec section 3): the words and a bar, amber
/// from the compact threshold and red from the clear threshold.
export function ContextMeter(props: {
  fill: number;
  window: number;
  rules: ContextRules;
  className?: string;
}) {
  const pct = contextPct(props.fill, props.window);
  const level = contextLevel(props.fill, props.window, props.rules);
  const text = contextText(props.fill, props.window);
  return (
    <div className={cn("flex items-center gap-2 text-xs", props.className)}>
      <span
        role="meter"
        aria-label="Context"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.min(pct, 100)}
        aria-valuetext={text}
        data-level={level}
        className="relative h-1.5 w-24 overflow-hidden rounded-full bg-muted"
      >
        <span
          className={cn(
            "absolute inset-y-0 left-0 rounded-full",
            level === "ok" && "bg-primary",
            level === "compact" && "bg-needs",
            level === "clear" && "bg-destructive",
          )}
          style={{ width: `${Math.min(pct, 100)}%` }}
        />
      </span>
      <span
        className={cn(
          "text-muted-foreground",
          level === "compact" && "text-needs",
          level === "clear" && "text-destructive",
        )}
      >
        {text}
      </span>
    </div>
  );
}

/// Sends the person's answer for a topic and refreshes what shows it.
export function useContextChoice(rootRunId: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (choice: ContextChoice) => commands.threadContextChoose(rootRunId, choice),
    onError: notifyError,
    onSuccess: (ctx) => qc.setQueryData(keys.threadContext(rootRunId), ctx),
    onSettled: () => invalidate(qc, keys.threadContext(rootRunId)),
  });
}

/// A topic's meter, above its reply box, and its question (context rules spec section 6):
/// "Context is 82% full — start fresh with a summary?" with Start fresh and Keep going, or what
/// its next message does first when the person asked. Nothing until a run reported a fill.
export function ThreadContextBar(props: { rootRunId: string; className?: string }) {
  const ctx = useQuery({
    queryKey: keys.threadContext(props.rootRunId),
    queryFn: () => commands.threadContext(props.rootRunId),
  }).data;
  const choose = useContextChoice(props.rootRunId);
  if (!ctx || ctx.fill === null) return null;
  const pct = contextPct(ctx.fill, ctx.window);
  return (
    <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1", props.className)}>
      <ContextMeter fill={ctx.fill} window={ctx.window} rules={ctx.rules} />
      {ctx.request === "fresh" && (
        <span role="status" className="text-xs text-muted-foreground">
          Your next message starts a fresh session with a summary.
        </span>
      )}
      {ctx.request === "compact" && (
        <span role="status" className="text-xs text-muted-foreground">
          Your next message compacts this conversation first.
        </span>
      )}
      {ctx.asks_fresh && (
        <div role="group" aria-label="Context is full" className="flex items-center gap-2">
          <span className="text-xs text-destructive">
            Context is {pct}% full — start fresh with a summary?
          </span>
          <Button
            size="xs"
            variant="outline"
            disabled={choose.isPending}
            onClick={onSingleClick(() => choose.mutate("fresh"))}
          >
            Start fresh
          </Button>
          <Button
            size="xs"
            variant="ghost"
            disabled={choose.isPending}
            onClick={onSingleClick(() => choose.mutate("keep_going"))}
          >
            Keep going
          </Button>
        </div>
      )}
    </div>
  );
}

/// The rules' fields (context rules spec section 4): on or off, compact at, start fresh at.
/// `error` is shown under them; `contextRulesError` gives it.
export function ContextRulesFields(props: {
  id: string;
  value: ContextRules;
  onChange: (rules: ContextRules) => void;
  error?: string;
}) {
  const { value } = props;
  const num = (text: string) => (text === "" ? Number.NaN : Number(text));
  const pctField = (key: "compact_at" | "clear_at", label: string) => (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={`${props.id}-${key}`}>{label}</Label>
      <Input
        id={`${props.id}-${key}`}
        type="number"
        min={10}
        max={95}
        className="w-28"
        disabled={!value.enabled}
        aria-invalid={props.error ? true : undefined}
        aria-describedby={props.error ? `${props.id}-error` : undefined}
        value={Number.isNaN(value[key]) ? "" : value[key]}
        onChange={(e) => props.onChange({ ...value, [key]: num(e.target.value) })}
      />
    </div>
  );
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Switch
          id={`${props.id}-enabled`}
          checked={value.enabled}
          onCheckedChange={(enabled) => props.onChange({ ...value, enabled })}
        />
        <Label htmlFor={`${props.id}-enabled`}>Compact and start fresh by themselves</Label>
      </div>
      <div className="flex flex-wrap gap-4">
        {pctField("compact_at", "Compact at (% of context)")}
        {pctField("clear_at", "Start fresh at (% of context)")}
      </div>
      {props.error ? (
        <p id={`${props.id}-error`} className="text-xs text-destructive">
          {props.error}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {CONTEXT_RULE} Background runs start fresh by themselves; a chat asks you first.
        </p>
      )}
    </div>
  );
}


/// The window a run's meter shows: the one its engine reported, else 1M for a `[1m]` model,
/// else 200k.
export function windowOf(run: Pick<Run, "context_window" | "model">): number {
  if (run.context_window !== null && run.context_window > 0) return run.context_window;
  return run.model?.toLowerCase().includes("[1m]") ? 1_000_000 : 200_000;
}

/// "Compacted 124k → 36k" or "Started fresh at 168k", with who asked and when.
export function contextEventLine(e: ContextEvent): string {
  const what =
    e.kind === "clear"
      ? `Started fresh at ${tokensShort(e.fill_before)}` +
        (e.fill_after !== null ? `, new session at ${tokensShort(e.fill_after)}` : "")
      : e.fill_after === null
        ? `Compaction failed at ${tokensShort(e.fill_before)}`
        : `Compacted ${tokensShort(e.fill_before)} → ${tokensShort(e.fill_after)}` +
          (e.ok ? "" : ", still over the threshold");
  const who = e.trigger === "manual" ? " (you asked)" : "";
  const used =
    e.tokens !== null && e.tokens > 0
      ? ` · ${e.kind === "clear" ? "the note" : "compaction"} used ${tokensShort(e.tokens)} tokens`
      : "";
  return `${what}${who}${used} · ${localDateTime(e.at)}`;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/// A thread's context on the run page (context rules spec sections 3 and 7): the meter of its
/// newest run with a fill, each compaction and fresh start, and the dot's totals.
function useRunContextReports(runs: Run[]) {
  return useQueries({
    queries: runs.map((run) => ({
      queryKey: keys.runContext(run.id),
      queryFn: () => commands.runContextReport(run.id),
    })),
  });
}

/// The tokens (in and out) the runs' compactions and hand-off notes used: Dots adds them to
/// the run's own tokens (context rules spec section 5).
export function useExtraCallTokens(runs: Run[]): number {
  return useRunContextReports(runs).reduce(
    (n, q) => n + (q.data?.events ?? []).reduce((m, e) => m + (e.tokens ?? 0), 0),
    0,
  );
}

export function RunContextSection(props: { runs: Run[] }) {
  const reports = useRunContextReports(props.runs);
  const report = reports.find((q) => q.data)?.data;
  const last = [...props.runs].reverse().find((r) => r.context_fill !== null);
  const events = reports.flatMap((q) => q.data?.events ?? []);
  if (!report || (last === undefined && events.length === 0)) return null;
  return (
    <section aria-label="Context" className="flex flex-col gap-1 text-xs text-muted-foreground">
      {last?.context_fill != null && (
        <ContextMeter fill={last.context_fill} window={windowOf(last)} rules={report.rules} />
      )}
      {events.length > 0 && (
        <ul className="flex flex-col gap-0.5">
          {events.map((e) => (
            <li key={e.id}>{contextEventLine(e)}</li>
          ))}
        </ul>
      )}
      {report.dot_compactions + report.dot_clears > 0 && (
        <p>
          This dot: {plural(report.dot_compactions, "compaction", "compactions")},{" "}
          {plural(report.dot_clears, "fresh start", "fresh starts")}
        </p>
      )}
    </section>
  );
}
