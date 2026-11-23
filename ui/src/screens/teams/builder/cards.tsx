import { Plus, Settings2 } from "lucide-react";
import { useEffect, useState } from "react";

import type { Budget, InputField, InputKind, Trigger } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";

import { MAX_INPUTS, nameOk, newInput } from "./draft";
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

const INPUT_KINDS: [InputKind, string][] = [
  ["text", "A line of text"],
  ["long_text", "Several lines"],
  ["date_time", "A date and time"],
];

/// The Inputs card (general workflows spec section 3): what a start is asked for besides the
/// task. Steps name an input by its name, as `{{input.<name>}}`.
export function InputsCard(props: {
  inputs: InputField[];
  onChange: (inputs: InputField[]) => void;
}) {
  const { inputs } = props;
  const set = (i: number, change: Partial<InputField>) =>
    props.onChange(inputs.map((f, j) => (j === i ? { ...f, ...change } : f)));
  return (
    <section
      aria-labelledby="workflow-inputs"
      className="flex w-full flex-col gap-2 rounded-xl border bg-card p-3"
    >
      <h2 id="workflow-inputs" className="font-semibold">
        Inputs
      </h2>
      <p className="text-xs text-muted-foreground">
        What a run is asked for when it starts, besides the task. Every step is told them, and a
        Send or Save step uses one as {"{{input.name}}"}.
      </p>
      {inputs.length > 0 && (
        <ul aria-label="Inputs" className="flex flex-col gap-3">
          {inputs.map((field, i) => (
            <li
              key={i}
              aria-label={field.label.trim() || field.name}
              className="flex flex-col gap-2 rounded-md border p-2"
            >
              <div className="grid grid-cols-2 gap-2">
                <Field label="Asks for" htmlFor={`input-label-${i}`}>
                  <Input
                    id={`input-label-${i}`}
                    placeholder="Product name"
                    maxLength={80}
                    value={field.label}
                    onChange={(e) => set(i, { label: e.target.value })}
                  />
                </Field>
                <Field
                  label="Name"
                  htmlFor={`input-name-${i}`}
                  error={nameOk(field.name) ? undefined : "Use a-z, 0-9, - and _."}
                >
                  <Input
                    id={`input-name-${i}`}
                    maxLength={40}
                    value={field.name}
                    onChange={(e) => set(i, { name: e.target.value })}
                  />
                </Field>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <Field label="Kind" htmlFor={`input-kind-${i}`}>
                  <NativeSelect
                    id={`input-kind-${i}`}
                    className="w-full"
                    value={field.kind}
                    onChange={(e) => set(i, { kind: e.target.value as InputKind, default: null })}
                  >
                    {INPUT_KINDS.map(([value, label]) => (
                      <NativeSelectOption key={value} value={value}>
                        {label}
                      </NativeSelectOption>
                    ))}
                  </NativeSelect>
                </Field>
                <Field label="Default" htmlFor={`input-default-${i}`}>
                  <Input
                    id={`input-default-${i}`}
                    type={field.kind === "date_time" ? "datetime-local" : "text"}
                    value={field.default ?? ""}
                    onChange={(e) => set(i, { default: e.target.value === "" ? null : e.target.value })}
                  />
                </Field>
              </div>
              <div className="flex items-center justify-between gap-2">
                <label className="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={field.required}
                    onChange={(e) => set(i, { required: e.target.checked })}
                  />
                  A run cannot start without it
                </label>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={`Remove the input ${field.label.trim() || field.name}`}
                  onClick={() => props.onChange(inputs.filter((_, j) => j !== i))}
                >
                  Remove
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}
      <div>
        <Button
          variant="outline"
          size="sm"
          disabled={inputs.length >= MAX_INPUTS}
          onClick={() => props.onChange([...inputs, newInput(inputs)])}
        >
          <Plus aria-hidden /> Add input
        </Button>
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
