import { Settings2 } from "lucide-react";
import { useEffect, useState } from "react";

import type { Budget, Trigger } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

import { type NameOf, triggerSummary } from "./labels";

/// The trigger card, first and in the accent (section 10.2): what starts the workflow. A click
/// on it, Enter on it or its settings button opens the trigger's settings.
export function TriggerCard(props: { trigger: Trigger; nameOf: NameOf; onOpen: () => void }) {
  const { trigger } = props;
  const [title, detail] = triggerSummary(trigger, props.nameOf);
  return (
    <div
      role="group"
      aria-label="Trigger"
      tabIndex={0}
      data-kind={trigger.kind}
      onClick={(e) => !(e.target as Element).closest("button") && props.onOpen()}
      onKeyDown={(e) => {
        if (e.key !== "Enter" || e.target !== e.currentTarget) return;
        e.preventDefault();
        props.onOpen();
      }}
      className="flex w-full items-start gap-2 rounded-xl border border-primary bg-card p-3 outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-semibold">When: {title}</span>
        <span className="text-xs text-muted-foreground">{detail}</span>
      </div>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label="Settings of the trigger"
        title="Settings"
        onClick={props.onOpen}
      >
        <Settings2 aria-hidden />
      </Button>
    </div>
  );
}

const LIMITS: { key: keyof Budget; label: string }[] = [
  { key: "max_runs", label: "Runs" },
  { key: "max_tokens", label: "Tokens" },
  { key: "max_minutes", label: "Minutes" },
  { key: "max_send_backs", label: "Send-backs" },
];

/// The Budget card at the bottom (section 5.7): the four limits of each run, edited in place.
export function BudgetCard(props: { budget: Budget; onChange: (budget: Budget) => void }) {
  return (
    <section
      aria-labelledby="workflow-budget"
      className="flex w-full flex-col gap-2 rounded-xl border bg-card p-3"
    >
      <h2 id="workflow-budget" className="font-semibold">
        Budget
      </h2>
      <p className="text-xs text-muted-foreground">
        What one run may spend. A run that reaches a limit waits for you: add budget or stop it.
      </p>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        {LIMITS.map(({ key, label }) => (
          <LimitField
            key={key}
            id={`budget-${key}`}
            label={label}
            value={props.budget[key]}
            onChange={(value) => props.onChange({ ...props.budget, [key]: value })}
          />
        ))}
      </div>
    </section>
  );
}

/// A whole number of at least 1; anything else is 0, which the live check refuses.
const limitOf = (text: string) => {
  const n = Number(text.trim());
  return Number.isInteger(n) && n > 0 ? n : 0;
};

/// One limit. Its text is kept as typed, so a field can be emptied on the way to a new number;
/// the draft gets the number it reads as. A delegate's at once and total use it too.
export function LimitField(props: {
  id: string;
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  const [text, setText] = useState(String(props.value));
  // The draft's value when it changes elsewhere (a refresh, or a save's answer).
  useEffect(() => {
    setText((t) => (limitOf(t) === props.value ? t : String(props.value)));
  }, [props.value]);
  return (
    <Field label={props.label} htmlFor={props.id}>
      <Input
        id={props.id}
        type="number"
        min={1}
        step={1}
        inputMode="numeric"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          props.onChange(limitOf(e.target.value));
        }}
      />
    </Field>
  );
}
