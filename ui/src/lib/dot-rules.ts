// The dot editor's checks. They mirror `DotSpec::validate` in the core (model.rs) so most
// mistakes show next to their field before a save; the core still has the last word.
import { z } from "zod";

import {
  LIMITS,
  type ContextRules,
  type DotSpec,
  type Schedule,
  type ScheduleRule,
} from "@/api/types";
import { tokenCapError, usdCapError } from "@/lib/spend";

export type SpecErrors = Partial<Record<keyof DotSpec, string>>;

const filled = (message: string) => z.string().refine((s) => s.trim() !== "", message);
const whole = (min: number, max: number, message: string) =>
  z.number({ error: message }).int(message).min(min, message).max(max, message);

const QUIET_MESSAGE = `The quiet limit must be a whole number of minutes, 1 to ${LIMITS.max_quiet_limit_secs / 60}.`;

const Fields = z.object({
  name: z
    .string()
    .regex(
      /^(?! )(?!.*  )(?!.* $)[A-Za-z0-9 ._-]{1,64}$/,
      "Use 1 to 64 letters, digits, spaces, '-', '_' or '.'.",
    ),
  instructions: filled("Say what the dot should do."),
  model: filled("Name the model to use."),
  workdir: filled("Choose the folder the dot works in."),
  max_turns: whole(1, Number.MAX_SAFE_INTEGER, "Max turns must be a whole number, 1 or more."),
  timeout_secs: whole(
    1,
    LIMITS.max_timeout_secs,
    `Timeout must be a whole number of seconds, 1 to ${LIMITS.max_timeout_secs} (7 days).`,
  ),
  approval_wait_secs: whole(
    0,
    LIMITS.max_approval_wait_secs,
    `Approval wait must be a whole number of seconds, 0 to ${LIMITS.max_approval_wait_secs}.`,
  ),
  // null follows the app default and 0 is off.
  quiet_limit_secs: z
    .number({ error: QUIET_MESSAGE })
    .int(QUIET_MESSAGE)
    // Whole minutes, as in Settings; the core itself takes any seconds in range.
    .refine(
      (n) =>
        n === 0 ||
        (n >= LIMITS.min_quiet_limit_secs && n <= LIMITS.max_quiet_limit_secs && n % 60 === 0),
      QUIET_MESSAGE,
    )
    .nullable(),
});

/// The folder name of a dot name, as the core's `folder_name` makes it: lower case, each run of
/// characters other than ASCII letters and digits one `-`, trimmed of `-`, at most 40
/// characters; `dot` when nothing is left.
export function folderName(name: string): string {
  const dashed = name
    .replace(/[^A-Za-z0-9]+/gu, "-")
    .toLowerCase()
    .replace(/^-+|-+$/g, "");
  return dashed.slice(0, 40).replace(/^-+|-+$/g, "") || "dot";
}

/// Why `name` cannot name a dot; undefined when it can.
export function nameError(name: string): string | undefined {
  return Fields.shape.name.safeParse(name).error?.issues[0]?.message;
}

/// What is wrong with a draft, by field. Empty when it can be saved.
/// A new dot may leave its folder blank: the core makes one.
export function specErrors(spec: DotSpec, isNew = false): SpecErrors {
  const errors: SpecErrors = {};
  const fields = Fields.safeParse(spec);
  if (!fields.success) {
    for (const issue of fields.error.issues) {
      const field = issue.path[0] as keyof DotSpec;
      if (isNew && field === "workdir") continue;
      errors[field] ??= issue.message;
    }
  }
  if (spec.engine === "local") {
    const url = (spec.endpoint_url ?? "").trim();
    if (url.startsWith("https://")) {
      errors.endpoint_url = "Use http://. https is not supported for local endpoints.";
    } else if (!/^http:\/\/[^\s/]+/.test(url)) {
      errors.endpoint_url = "A local dot needs its endpoint, starting with http://.";
    }
  }
  if (spec.policy.rules.some((r) => r.tool.trim() === "")) {
    errors.policy = "Every rule needs a tool pattern.";
  }
  const schedule = scheduleError(spec.schedule);
  if (schedule) errors.schedule = schedule;
  const cap = usdCapError(spec.spend_cap.daily_usd) ?? tokenCapError(spec.spend_cap.daily_tokens);
  if (cap) errors.spend_cap = cap;
  const context = spec.context_rules && contextRulesError(spec.context_rules);
  if (context) errors.context_rules = context;
  return errors;
}

/// The core's words when context rules are out of bounds (context rules spec section 4).
export const CONTEXT_RULE = "Each threshold is 10 to 95%, and start fresh must be above compact.";

/// Why `rules` cannot be saved: a threshold outside 10 to 95%, or start fresh not above compact.
export function contextRulesError(rules: ContextRules): string | undefined {
  const ok = (n: number) =>
    Number.isInteger(n) && n >= LIMITS.context_min_pct && n <= LIMITS.context_max_pct;
  if (!ok(rules.compact_at) || !ok(rules.clear_at) || rules.clear_at <= rules.compact_at) {
    return CONTEXT_RULE;
  }
  return undefined;
}

/// The schedule a draft saves and previews: a cron expression without its surrounding spaces.
/// Other kinds as they are.
export function scheduleOf(schedule: Schedule | null): Schedule | null {
  if (schedule?.rule.kind !== "cron") return schedule;
  return { ...schedule, rule: { kind: "cron", expr: schedule.rule.expr.trim() } };
}

/// The schedule editor's kinds: a rule's kind, or no schedule.
export const ScheduleKind = z.enum(["none", "once", "every", "daily", "weekly", "cron"]);
export type ScheduleKind = z.infer<typeof ScheduleKind>;

export const kindOf = (schedule: Schedule | null): ScheduleKind => schedule?.rule.kind ?? "none";

const MAX_EVERY_MINUTES = 7 * 24 * 60;
const everyMinutes = whole(
  1,
  MAX_EVERY_MINUTES,
  "Repeat every 1 minute to 7 days (10080 minutes), in whole minutes.",
);
const runCount = whole(1, 2 ** 32 - 1, "Stop after a whole number of runs, 1 or more.");

/// What is wrong with a schedule that the core cannot even be sent: an emptied field, a
/// number that is not a whole one or out of the core's range. The rest (no day picked, a cron
/// expression that does not parse, a date that leaves no run) the core answers through the
/// preview, so the editor shows its words.
export function scheduleError(schedule: Schedule | null): string | undefined {
  if (schedule === null) return undefined;
  const { rule, until } = schedule;
  switch (rule.kind) {
    case "once":
      if (rule.at === "") return "Pick the date and time of the run.";
      break;
    case "every": {
      const minutes = everyMinutes.safeParse(rule.minutes);
      if (!minutes.success) return minutes.error.issues[0]?.message;
      break;
    }
    case "daily":
    case "weekly":
      if (rule.time === "") return "Pick a time.";
      break;
    case "cron":
      if (rule.expr.trim() === "") return "Write a cron expression.";
      break;
  }
  if (until !== null && "count" in until) {
    const count = runCount.safeParse(until.count);
    if (!count.success) return count.error.issues[0]?.message;
  }
  if (until !== null && "date" in until && until.date === "") {
    return "Pick the date to stop after.";
  }
  return undefined;
}

const pad = (n: number) => String(n).padStart(2, "0");

/// The local date `days` after `now`'s, at `time` (`HH:MM`), as a schedule writes it:
/// `YYYY-MM-DDTHH:MM`.
export function localDateTimeIn(days: number, time: string, now: Date = new Date()): string {
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
  return `${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}T${time}`;
}

/// An every rule of `minutes`. It keeps the stored rule's anchor while the minutes are the
/// stored ones; otherwise it has none, and the core starts the phase when it is saved (spec
/// "Scheduler v2" §2).
export function everyRule(minutes: number, stored: Schedule | null): ScheduleRule {
  const anchor =
    stored?.rule.kind === "every" && stored.rule.minutes === minutes ? stored.rule.anchor : null;
  return { kind: "every", minutes, anchor };
}

function timeOf(rule: ScheduleRule | undefined): string {
  switch (rule?.kind) {
    case "daily":
    case "weekly":
      return rule.time || "09:00";
    case "once":
      return rule.at.slice(11) || "09:00";
    default:
      return "09:00";
  }
}

/// The schedule after `kind` is picked in the editor. Picking the stored schedule's kind again
/// gives its rule back (with an every rule's anchor). A new rule takes the time of the one
/// before, or 09:00: a once is tomorrow, a weekly is on Mondays, an every is hourly, a cron
/// expression is empty. Catch-up (on for a new schedule) and the stop go along; a once has
/// no stop and always catches up, as its checkbox is hidden.
export function withKind(
  current: Schedule | null,
  kind: ScheduleKind,
  stored: Schedule | null,
  now: Date = new Date(),
): Schedule | null {
  if (kind === "none") return null;
  if (current?.rule.kind === kind) return current;
  const fromStored = stored?.rule.kind === kind ? stored : null;
  const time = timeOf(current?.rule);
  const fresh: Record<ScheduleRule["kind"], () => ScheduleRule> = {
    once: () => ({ kind: "once", at: localDateTimeIn(1, time, now) }),
    every: () => everyRule(60, stored),
    daily: () => ({ kind: "daily", time }),
    weekly: () => ({ kind: "weekly", days: ["mon"], time }),
    cron: () => ({ kind: "cron", expr: "" }),
  };
  const rule = fromStored?.rule ?? fresh[kind]();
  if (kind === "once") return { rule, catch_up: fromStored?.catch_up ?? true, until: null };
  const keep = current !== null && current.rule.kind !== "once" ? current : fromStored;
  return { rule, catch_up: keep?.catch_up ?? true, until: keep?.until ?? null };
}

/// The editable part of a stored dot.
export function specOf(dot: DotSpec): DotSpec {
  return {
    name: dot.name,
    instructions: dot.instructions,
    engine: dot.engine,
    model: dot.model,
    endpoint_url: dot.endpoint_url,
    workdir: dot.workdir,
    workspace_mode: dot.workspace_mode,
    schedule: dot.schedule,
    policy: dot.policy,
    uses: dot.uses,
    variables: dot.variables,
    browser: dot.browser,
    use_global_instructions: dot.use_global_instructions,
    check_results: dot.check_results,
    use_graph: dot.use_graph,
    use_user_settings: dot.use_user_settings,
    max_turns: dot.max_turns,
    timeout_secs: dot.timeout_secs,
    approval_wait_secs: dot.approval_wait_secs,
    quiet_limit_secs: dot.quiet_limit_secs,
    spend_cap: dot.spend_cap,
    enabled: dot.enabled,
    context_rules: dot.context_rules,
    making: dot.making,
  };
}

/// The three named presets in one plain sentence each (setup agent spec §6).
export const PRESET_SENTENCES = {
  "read-only": "May read files. Never writes, runs commands, reads websites or uses MCP servers.",
  sandboxed:
    "May edit files in its folder and run a few safe git commands. Asks before anything else.",
  trusted: "May run commands without asking.",
} as const;
