import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import { useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { Run, Spend, TriggerKind } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { capOf, costAgainst, tokensAgainst, worstLevel, type CapLevel } from "@/lib/spend";

/// The cost of a Claude run as the CLI reports it: a subscription bills no run, so this is
/// what it would cost on the API.
export const ESTIMATED =
  "What the runs would cost on the API, as the engine reported it. A Claude subscription does not bill per run, so this is an estimate.";

/// Today's spend: the query every spend line shares.
export function useSpendToday() {
  return useQuery({ queryKey: keys.spend, queryFn: commands.spendToday });
}

/// A cap's field: empty is no cap. It keeps what is typed ("2." on the way to "2.5") and hands
/// the number on (NaN when it is not one).
export function CapInput(props: {
  id: string;
  label: string;
  value: number | null;
  onChange: (cap: number | null) => void;
  error?: string;
  hint?: string;
  placeholder?: string;
}) {
  const [text, setText] = useState(props.value === null ? "" : String(props.value));
  return (
    <Field label={props.label} htmlFor={props.id} error={props.error} hint={props.hint}>
      <Input
        id={props.id}
        inputMode="decimal"
        className="w-40"
        placeholder={props.placeholder ?? "No cap"}
        aria-invalid={props.error ? true : undefined}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          props.onChange(capOf(e.target.value));
        }}
      />
    </Field>
  );
}

const LEVEL_TEXT: Record<CapLevel, string> = {
  under: "",
  warn: "Over 80% of the cap",
  reached: "Cap reached: runs wait until it is raised or the period ends",
};

/// "Today: $1.60 of $2.00 estimated · 120,000 of 200,000 tokens", with a warning from 80% of a
/// cap and a note once one is reached.
export function SpendLine(props: {
  label: string;
  spend: Spend;
  usdCap: number | null;
  tokenCap?: number | null;
  /// Show the tokens even without a token cap.
  showTokens?: boolean;
}) {
  const tokenCap = props.tokenCap ?? null;
  const level = worstLevel([
    [props.spend.cost_usd, props.usdCap],
    [props.spend.tokens, tokenCap],
  ]);
  const tokens = tokenCap !== null || props.showTokens;
  return (
    <p
      className={cn(
        "text-sm",
        level === "warn" && "text-needs",
        level === "reached" && "text-destructive",
      )}
    >
      {props.label}: {costAgainst(props.spend.cost_usd, props.usdCap)}{" "}
      <abbr title={ESTIMATED} className="no-underline">
        estimated
      </abbr>
      {tokens && ` · ${tokensAgainst(props.spend.tokens, tokenCap)}`}
      {level !== null && level !== "under" && (
        <span role="status"> · {LEVEL_TEXT[level]}</span>
      )}
    </p>
  );
}

const BACKGROUND: TriggerKind[] = ["schedule", "watcher", "follow_up", "webhook", "api"];

/// Whether **Run once anyway** is offered: the person's own runs only; background runs wait.
export const overridable = (trigger: TriggerKind) => !BACKGROUND.includes(trigger);

/// The core's words for the cap of every dot together begin so.
const ALL_DOTS = "All sols reached the ";

/// Where **Raise cap** goes: the dot's editor for its own cap, Settings for the cap of all dots.
export function raiseCapLink(dotId: string, text: string): string {
  return text.startsWith(ALL_DOTS) ? "/settings/runs" : `/dots/${dotId}`;
}

/// The buttons of a run held by a spending cap: **Raise cap**, and **Run once anyway** for the
/// person's own runs.
export function CapActions(props: {
  runId: string;
  dotId: string;
  trigger: TriggerKind;
  text: string;
  size?: "xs" | "sm";
}) {
  const qc = useQueryClient();
  const release = useMutation({
    mutationFn: () => commands.runReleaseCap(props.runId),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.inbox, ["thread"], ["runs"]]) invalidate(qc, queryKey);
    },
  });
  const size = props.size ?? "sm";
  return (
    <>
      <Button asChild size={size} variant="outline">
        <Link to={raiseCapLink(props.dotId, props.text)}>Raise cap</Link>
      </Button>
      {overridable(props.trigger) && (
        <Button
          size={size}
          variant="outline"
          disabled={release.isPending}
          onClick={onSingleClick(() => release.mutate())}
        >
          Run once anyway
        </Button>
      )}
    </>
  );
}

/// A queued run waiting for a spending cap: why, and what the person can do. Nothing for any
/// other run.
export function CapNote(props: { run: Run; className?: string }) {
  const { run } = props;
  if (run.status !== "queued" || run.wait_reason !== "spend_cap") return null;
  const text = run.wait_detail ?? "A spending cap is reached";
  return (
    <div className={cn("flex flex-wrap items-center gap-2 text-sm", props.className)}>
      <span className="text-needs">Waiting: {text}</span>
      <CapActions
        runId={run.id}
        dotId={run.dot_id}
        trigger={run.trigger}
        text={text}
        size="xs"
      />
    </div>
  );
}
