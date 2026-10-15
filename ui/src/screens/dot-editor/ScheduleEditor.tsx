// The dot editor's Schedule section (spec "Scheduler v2" §9): a kind, its fields, catch-up,
// the stop, and the next runs as the core computes them. The UI parses no schedule: what is
// wrong with one comes from the core, through the preview, and shows under the fields.
import { useId, useState, type ReactNode } from "react";

import type { Schedule, ScheduleRule, Until, Weekday } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { everyRule, kindOf, localDateTimeIn, ScheduleKind, withKind } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";

/// The core's next runs of the schedule being edited, and what it said about the stored
/// schedule when that is not an error (`note`).
export type SchedulePreview = { times: string[]; pending: boolean; note?: string };

type Rule<K extends ScheduleRule["kind"]> = Extract<ScheduleRule, { kind: K }>;

const KINDS: [ScheduleKind, string][] = [
  ["none", "None"],
  ["once", "Once"],
  ["every", "Every few minutes or hours"],
  ["daily", "Daily"],
  ["weekly", "Weekly"],
  ["cron", "Cron expression"],
];

const DAYS: [Weekday, string][] = [
  ["mon", "Monday"],
  ["tue", "Tuesday"],
  ["wed", "Wednesday"],
  ["thu", "Thursday"],
  ["fri", "Friday"],
  ["sat", "Saturday"],
  ["sun", "Sunday"],
];

/// A fieldset inside the Schedule section, its legend styled like a field's label.
function Group(props: { legend: string; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1.5 text-sm leading-none font-medium">{props.legend}</legend>
      {props.children}
    </fieldset>
  );
}

function TimeField(props: { value: string; onChange: (time: string) => void }) {
  return (
    <Field label="Time" htmlFor="dot-schedule-time" hint="In this PC's time zone.">
      <Input
        id="dot-schedule-time"
        type="time"
        className="w-36"
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      />
    </Field>
  );
}

function OnceFields(props: { rule: Rule<"once">; onChange: (rule: ScheduleRule) => void }) {
  return (
    <Field label="Date and time" htmlFor="dot-schedule-at" hint="In this PC's time zone.">
      <Input
        id="dot-schedule-at"
        type="datetime-local"
        className="w-56"
        value={props.rule.at}
        onChange={(e) => props.onChange({ kind: "once", at: e.target.value })}
      />
    </Field>
  );
}

type Unit = "minutes" | "hours";
const FACTOR: Record<Unit, number> = { minutes: 1, hours: 60 };

/// `n` units in minutes, without the float noise of a decimal number of hours (0.1 h is 6).
function minutesOf(n: number, unit: Unit): number {
  const m = n * FACTOR[unit];
  return Math.abs(m - Math.round(m)) < 1e-9 ? Math.round(m) : m;
}

/// The minutes of an every rule, typed in minutes or in hours. The unit only helps typing: the
/// rule holds minutes. A schedule of whole hours opens in hours.
function EveryFields(props: {
  rule: Rule<"every">;
  stored: Schedule | null;
  onChange: (rule: ScheduleRule) => void;
}) {
  const { minutes } = props.rule;
  const [unit, setUnit] = useState<Unit>(() =>
    minutes >= 60 && minutes % 60 === 0 ? "hours" : "minutes",
  );
  const set = (m: number) => props.onChange(everyRule(m, props.stored));
  return (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor="dot-schedule-every">Every</Label>
      <div className="flex gap-2">
        <Input
          id="dot-schedule-every"
          type="number"
          min={1}
          className="w-28"
          value={Number.isNaN(minutes) ? "" : minutes / FACTOR[unit]}
          onChange={(e) =>
            set(e.target.value === "" ? Number.NaN : minutesOf(Number(e.target.value), unit))
          }
        />
        <NativeSelect
          aria-label="Unit"
          value={unit}
          onChange={(e) => {
            const next: Unit = e.target.value === "hours" ? "hours" : "minutes";
            setUnit(next);
            // The number stays as typed: 2 minutes become 2 hours.
            if (!Number.isNaN(minutes)) set(minutesOf(minutes / FACTOR[unit], next));
          }}
        >
          <option value="minutes">minutes</option>
          <option value="hours">hours</option>
        </NativeSelect>
      </div>
      <p className="text-xs text-muted-foreground">
        Counted in elapsed time from when the schedule is saved; daylight saving does not shift
        it.
      </p>
    </div>
  );
}

function WeeklyFields(props: { rule: Rule<"weekly">; onChange: (rule: ScheduleRule) => void }) {
  const { rule } = props;
  function toggle(day: Weekday) {
    const on = rule.days.includes(day);
    // In week order, whatever order the days were picked in.
    const days = DAYS.map(([d]) => d).filter((d) => (d === day ? !on : rule.days.includes(d)));
    props.onChange({ ...rule, days });
  }
  return (
    <>
      <Group legend="Days">
        <div className="flex flex-wrap gap-1">
          {DAYS.map(([day, name]) => {
            const on = rule.days.includes(day);
            return (
              <Button
                key={day}
                type="button"
                size="sm"
                variant={on ? "default" : "outline"}
                aria-pressed={on}
                aria-label={name}
                onClick={() => toggle(day)}
              >
                {name.slice(0, 3)}
              </Button>
            );
          })}
        </div>
      </Group>
      <TimeField value={rule.time} onChange={(time) => props.onChange({ ...rule, time })} />
    </>
  );
}

function CronField(props: { rule: Rule<"cron">; onChange: (rule: ScheduleRule) => void }) {
  return (
    <Field
      label="Cron expression"
      htmlFor="dot-schedule-cron"
      hint="Six fields: second minute hour day-of-month month day-of-week, and an optional year."
    >
      <Input
        id="dot-schedule-cron"
        className="font-mono"
        placeholder="0 0 2 * * *"
        value={props.rule.expr}
        onChange={(e) => props.onChange({ kind: "cron", expr: e.target.value })}
      />
    </Field>
  );
}

function RuleFields(props: {
  rule: ScheduleRule;
  stored: Schedule | null;
  onChange: (rule: ScheduleRule) => void;
}) {
  const { rule, onChange } = props;
  switch (rule.kind) {
    case "once":
      return <OnceFields rule={rule} onChange={onChange} />;
    case "every":
      return <EveryFields rule={rule} stored={props.stored} onChange={onChange} />;
    case "daily":
      return <TimeField value={rule.time} onChange={(time) => onChange({ ...rule, time })} />;
    case "weekly":
      return <WeeklyFields rule={rule} onChange={onChange} />;
    case "cron":
      return <CronField rule={rule} onChange={onChange} />;
  }
}

/// When the schedule stops: never, after a number of runs, or after a local date and time.
/// Picking a stop the stored schedule has gives its value back.
function StopFields(props: {
  until: Until | null;
  stored: Schedule | null;
  onChange: (until: Until | null) => void;
}) {
  const name = useId();
  const { until, stored } = props;
  const stop = until === null ? "never" : "count" in until ? "count" : "date";
  const storedUntil = stored?.rule.kind === "once" ? null : (stored?.until ?? null);
  const radio = (value: typeof stop, label: string, next: () => Until | null) => (
    <label className="flex items-center gap-2 text-sm">
      <input
        type="radio"
        name={name}
        value={value}
        checked={stop === value}
        onChange={() => props.onChange(next())}
      />
      {label}
    </label>
  );
  return (
    <Group legend="Stop">
      {radio("never", "Never", () => null)}
      {radio("count", "After a number of runs", () =>
        storedUntil && "count" in storedUntil ? storedUntil : { count: 10 },
      )}
      {until !== null && "count" in until && (
        <div className="ml-6">
          <Field label="Number of runs" htmlFor="dot-schedule-count">
            <Input
              id="dot-schedule-count"
              type="number"
              min={1}
              className="w-28"
              value={Number.isNaN(until.count) ? "" : until.count}
              onChange={(e) =>
                props.onChange({
                  count: e.target.value === "" ? Number.NaN : Number(e.target.value),
                })
              }
            />
          </Field>
        </div>
      )}
      {radio("date", "After a date", () =>
        storedUntil && "date" in storedUntil ? storedUntil : { date: localDateTimeIn(30, "23:59") },
      )}
      {until !== null && "date" in until && (
        <div className="ml-6">
          <Field
            label="Stop date"
            htmlFor="dot-schedule-until"
            hint="The last run is at or before this time."
          >
            <Input
              id="dot-schedule-until"
              type="datetime-local"
              className="w-56"
              value={until.date}
              onChange={(e) => props.onChange({ date: e.target.value })}
            />
          </Field>
        </div>
      )}
    </Group>
  );
}

/// The Schedule section's controls. `stored` is the dot's saved schedule (null for a new dot):
/// picking its kind or stop again gives its values back, and an every rule keeps its anchor
/// while its minutes are the stored ones. `error` is what blocks a save.
export function ScheduleEditor(props: {
  schedule: Schedule | null;
  stored: Schedule | null;
  onChange: (schedule: Schedule | null) => void;
  error?: string;
  preview: SchedulePreview;
}) {
  const { schedule, stored, onChange, preview } = props;
  return (
    <div className="flex flex-col gap-3">
      <Field label="Kind" htmlFor="dot-schedule-kind">
        <NativeSelect
          id="dot-schedule-kind"
          value={kindOf(schedule)}
          onChange={(e) => onChange(withKind(schedule, ScheduleKind.parse(e.target.value), stored))}
        >
          {KINDS.map(([kind, label]) => (
            <option key={kind} value={kind}>
              {label}
            </option>
          ))}
        </NativeSelect>
      </Field>
      {schedule && (
        <RuleFields
          rule={schedule.rule}
          stored={stored}
          onChange={(rule) => onChange({ ...schedule, rule })}
        />
      )}
      {schedule && schedule.rule.kind !== "once" && (
        <>
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <Switch
                id="dot-schedule-catch-up"
                checked={schedule.catch_up}
                onCheckedChange={(catch_up) => onChange({ ...schedule, catch_up })}
              />
              <Label htmlFor="dot-schedule-catch-up">Catch up a missed run</Label>
            </div>
            <p className="text-xs text-muted-foreground">
              When the PC was off or Dots was closed at a run's time, the dot runs once when Dots
              is back, if that is within 24 hours.
            </p>
          </div>
          <StopFields
            until={schedule.until}
            stored={stored}
            onChange={(until) => onChange({ ...schedule, until })}
          />
        </>
      )}
      {/* The core answers while the user types; a screen reader hears it too. */}
      <div aria-live="polite">
        {props.error ? (
          <p className="text-xs text-destructive">{props.error}</p>
        ) : (
          preview.note && <p className="text-xs text-muted-foreground">{preview.note}</p>
        )}
      </div>
      {preview.times.length > 0 && (
        <div className="text-xs text-muted-foreground">
          <span>Next runs:</span>
          <ul aria-label="Next runs" className="mt-1 flex flex-col gap-0.5">
            {preview.times.map((t) => (
              <li key={t}>{localDateTime(t)}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
