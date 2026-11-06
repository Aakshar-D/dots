// A run's plan and timeline (run plan spec): the to-do list the dot keeps with the `claude` CLI's
// TodoWrite tool, newest state first, and what the run did, as plain steps with their times.
import { z } from "zod";

import { RunEventData, type RunEventRecord } from "@/api/types";
import { BROWSER_PREFIX } from "@/lib/browser";
import { checkInLine, PARKED_PREFIX, stepsLine, type CheckInKind } from "@/lib/transcript";

/// A step of a plan as TodoWrite gives it: what to do, what the dot says while it does it, and
/// where it is.
export type PlanStatus = "pending" | "in_progress" | "completed";

export type PlanStep = {
  content: string;
  activeForm: string;
  status: PlanStatus;
};

/// Who keeps a plan or did a step: the run's dot, or a helper it started with the CLI's `Task` or
/// `Agent` tool (named by what the dot asked of it).
export type Owner = { kind: "dot" } | { kind: "helper"; id: string; label: string };

export type Plan = {
  owner: Owner;
  steps: PlanStep[];
  /// The `seq` of the TodoWrite call this state comes from.
  seq: number;
  /// When that call was made.
  ts: string;
};

/// The tool whose calls are a plan. Only the Claude engine has it.
export const PLAN_TOOL = "TodoWrite";

/// The CLI's tools that start a helper (a sub-agent) whose own calls name the helper's call.
const HELPER_TOOLS = new Set(["Task", "Agent"]);

const TodoItem = z.object({
  content: z.string(),
  status: z.string().optional(),
  activeForm: z.string().optional(),
});

/// The steps of a TodoWrite call's input, or null when the input is not a to-do list at all (the
/// call then changes nothing). An item without words is left out; a status other than the three
/// TodoWrite knows reads as not started.
export function planSteps(input: unknown): PlanStep[] | null {
  const todos =
    typeof input === "object" && input !== null ? (input as Record<string, unknown>).todos : null;
  if (!Array.isArray(todos)) return null;
  const steps: PlanStep[] = [];
  for (const raw of todos) {
    const item = TodoItem.safeParse(raw);
    if (!item.success) continue;
    const content = item.data.content.trim();
    if (content === "") continue;
    const status: PlanStatus =
      item.data.status === "in_progress" || item.data.status === "completed"
        ? item.data.status
        : "pending";
    const activeForm = item.data.activeForm?.trim() || content;
    steps.push({ content, activeForm, status });
  }
  return steps;
}

type Call = {
  seq: number;
  ts: string;
  id: string;
  tool: string;
  input: unknown;
  parent: string | null;
};

type Result = { ts: string; output: string; isError: boolean };

/// The run's events read once: its calls in `seq` order (events can come in any order, live ones
/// after a reload's), each call's result by id, and its text in order.
function read(events: RunEventRecord[]) {
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  const calls: Call[] = [];
  const results = new Map<string, Result>();
  const texts: number[] = [];
  const seen = new Set<number>();
  for (const record of sorted) {
    // A record delivered twice (a reload and the live event) counts once.
    if (seen.has(record.seq)) continue;
    seen.add(record.seq);
    const parsed = RunEventData.safeParse(record.data);
    if (!parsed.success) continue;
    const event = parsed.data;
    if (event.kind === "tool_call") {
      calls.push({
        seq: record.seq,
        ts: record.ts,
        id: event.id,
        tool: event.tool,
        input: event.input,
        parent: event.parent ?? null,
      });
    } else if (event.kind === "tool_result") {
      results.set(event.id, {
        ts: record.ts,
        output: event.output,
        isError: event.is_error,
      });
    } else if (event.kind === "assistant_text") {
      texts.push(record.seq);
    }
  }
  return { calls, results, texts };
}

/// The trimmed string argument `key` of a call, or "".
function field(input: unknown, key: string): string {
  const value =
    typeof input === "object" && input !== null
      ? (input as Record<string, unknown>)[key]
      : undefined;
  return typeof value === "string" ? value.trim() : "";
}

/// What a helper is called: what the dot asked of it, else its kind, else "a helper".
function helperLabel(input: unknown): string {
  return field(input, "description") || field(input, "subagent_type") || "a helper";
}

/// The owner of a call: the helper whose call `parent` names, when the run started one with that
/// id; the dot otherwise (a parent the run does not know is still the dot's own work).
function ownerOf(parent: string | null, helpers: ReadonlyMap<string, Owner>): Owner {
  return (parent !== null && helpers.get(parent)) || { kind: "dot" };
}

function helpersOf(calls: Call[]): Map<string, Owner> {
  const helpers = new Map<string, Owner>();
  for (const call of calls) {
    if (HELPER_TOOLS.has(call.tool)) {
      helpers.set(call.id, {
        kind: "helper",
        id: call.id,
        label: helperLabel(call.input),
      });
    }
  }
  return helpers;
}

const ownerKey = (owner: Owner) => (owner.kind === "dot" ? "" : owner.id);

/// The run's plans (run plan spec section 3): for the dot and for each helper that kept one, the
/// newest to-do list it wrote, by `seq`. A call that failed or is not a to-do list changes
/// nothing; an empty list clears its owner's plan. The dot's plan comes first, then the helpers'
/// in the order they started.
export function runPlans(events: RunEventRecord[]): Plan[] {
  const { calls, results } = read(events);
  const helpers = helpersOf(calls);
  const latest = new Map<string, Plan>();
  for (const call of calls) {
    if (call.tool !== PLAN_TOOL) continue;
    if (results.get(call.id)?.isError) continue;
    const steps = planSteps(call.input);
    if (steps === null) continue;
    const owner = ownerOf(call.parent, helpers);
    latest.set(ownerKey(owner), { owner, steps, seq: call.seq, ts: call.ts });
  }
  const order = (plan: Plan) =>
    plan.owner.kind === "dot" ? -1 : calls.findIndex((c) => c.id === ownerKey(plan.owner));
  return [...latest.values()]
    .filter((plan) => plan.steps.length > 0)
    .sort((a, b) => order(a) - order(b));
}

/// How far a plan is: its steps done, of all of them.
export function planProgress(steps: PlanStep[]): {
  done: number;
  total: number;
} {
  return {
    done: steps.filter((s) => s.status === "completed").length,
    total: steps.length,
  };
}

/// What became of a timeline step: it still runs, it was done, it failed, it waits for the
/// person's approval, or the run ended before it had a result.
export type EntryStatus = "running" | "done" | "failed" | "waiting" | "stopped";

/// One step of the timeline.
export type TimelineEntry = {
  /// The `seq` of its first event: a stable key.
  key: number;
  kind: "tools" | "checkin" | "helper" | "member" | "workflow" | "browser" | "milestone";
  /// Who did it.
  owner: Owner;
  /// What it says: "Read 4 files", "Asked you: …", "Done: Run the tests".
  text: string;
  /// When it started and, once every call of it has its result, when it ended.
  start: string;
  end: string | null;
  status: EntryStatus;
  /// Its calls, for its details.
  calls: {
    tool: string;
    input: unknown;
    result: { output: string; isError: boolean } | null;
  }[];
};

const CHECK_INS = new Map<string, CheckInKind>([
  ["Notify", "notify"],
  ["mcp__dots__notify", "notify"],
  ["Ask", "ask"],
  ["mcp__dots__ask", "ask"],
  ["FollowUp", "follow_up"],
  ["mcp__dots__follow_up", "follow_up"],
]);

/// Which of the workflow tools a call is: the Claude engine names its call by Dots' own server's
/// tool, the local engine by the tool's own name or by its policy name `Workflow`.
function workflowTool(tool: string, input: unknown): "member" | "workflow" | null {
  const has = (key: string) => typeof input === "object" && input !== null && key in input;
  if (tool === "mcp__dots__start_member" || tool === "start_member") return "member";
  if (tool === "mcp__dots__start_workflow" || tool === "start_workflow") return "workflow";
  if (tool === "Workflow" && has("member")) return "member";
  if (tool === "Workflow" && has("workflow")) return "workflow";
  return null;
}

/// The words of a call that stands by itself on the timeline, or null for one that groups.
function ownLine(call: Call, result: Result | null): Pick<TimelineEntry, "kind" | "text"> | null {
  const checkIn = CHECK_INS.get(call.tool);
  if (checkIn !== undefined) {
    const line = checkInLine(checkIn, call.input, result);
    // "Asked: …" reads to the person as what the dot asked them.
    return { kind: "checkin", text: line.replace(/^Asked: /, "Asked you: ") };
  }
  if (HELPER_TOOLS.has(call.tool)) {
    return {
      kind: "helper",
      text: `Handed to a helper: ${helperLabel(call.input)}`,
    };
  }
  const workflow = workflowTool(call.tool, call.input);
  if (workflow === "member") {
    const task = field(call.input, "task").split("\n")[0] ?? "";
    const who = field(call.input, "member") || "a member";
    return {
      kind: "member",
      text: task ? `Handed to ${who}: ${task}` : `Handed to ${who}`,
    };
  }
  if (workflow === "workflow") {
    const name = field(call.input, "workflow");
    return {
      kind: "workflow",
      text: name ? `Started the workflow ${name}` : "Started a workflow",
    };
  }
  if (call.tool.startsWith(BROWSER_PREFIX)) return { kind: "browser", text: "Used the browser" };
  return null;
}

/// How a set of calls went, in a run that is still going (`active`) or has ended.
function statusOf(results: (Result | null)[], active: boolean): EntryStatus {
  if (results.some((r) => r !== null && r.output.startsWith(PARKED_PREFIX))) {
    return active ? "waiting" : "done";
  }
  if (results.some((r) => r === null)) return active ? "running" : "stopped";
  return results.some((r) => r?.isError) ? "failed" : "done";
}

/// The run's timeline (run plan spec section 4), oldest first: the dot's and each helper's
/// tool calls, consecutive ones of the same owner as one step ("Read 4 files · Ran a command");
/// a check-in, a helper, a member or workflow started, and the browser each as a step of its
/// own; and each plan step as it is marked done. Plan updates themselves are not steps.
/// `owners`: MCP server name to the plugin it comes with, as the transcript names them.
export function runTimeline(
  events: RunEventRecord[],
  active: boolean,
  owners: ReadonlyMap<string, string> = new Map(),
): TimelineEntry[] {
  const { calls, results, texts } = read(events);
  const helpers = helpersOf(calls);
  const out: TimelineEntry[] = [];
  // The steps of each owner's plan already done, so that a step marked done shows once.
  const doneBefore = new Map<string, Set<string>>();
  // The group the next plain call of an owner joins, while nothing else came between.
  let open: { entry: TimelineEntry; calls: Call[] } | null = null;
  const close = () => {
    if (open === null) return;
    const res = open.calls.map((c) => results.get(c.id) ?? null);
    open.entry.text = stepsLine(open.calls, owners);
    open.entry.status = statusOf(res, active);
    open.entry.end = res.every((r) => r !== null)
      ? res.reduce((last, r) => (r!.ts > last ? r!.ts : last), open.entry.start)
      : null;
    open.entry.calls = open.calls.map((c, i) => ({
      tool: c.tool,
      input: c.input,
      result: res[i] ? { output: res[i].output, isError: res[i].isError } : null,
    }));
    open = null;
  };
  let textAt = 0;
  for (const call of calls) {
    const owner = ownerOf(call.parent, helpers);
    // The dot's words between two calls end a group of its own calls: it moved on.
    let spoke = false;
    while (textAt < texts.length && texts[textAt]! < call.seq) {
      spoke = true;
      textAt += 1;
    }
    if (spoke && open?.entry.owner.kind === "dot" && owner.kind === "dot") close();
    if (call.tool === PLAN_TOOL) {
      if (results.get(call.id)?.isError) continue;
      const steps = planSteps(call.input);
      if (steps === null) continue;
      const key = ownerKey(owner);
      const before = doneBefore.get(key) ?? new Set<string>();
      const now = new Set(steps.filter((s) => s.status === "completed").map((s) => s.content));
      for (const step of steps) {
        if (step.status !== "completed" || before.has(step.content)) continue;
        close();
        out.push({
          key: call.seq,
          kind: "milestone",
          owner,
          text: `Done: ${step.content}`,
          start: call.ts,
          end: call.ts,
          status: "done",
          calls: [],
        });
      }
      doneBefore.set(key, now);
      continue;
    }
    const result = results.get(call.id) ?? null;
    const own = ownLine(call, result);
    if (own !== null) {
      close();
      out.push({
        key: call.seq,
        ...own,
        owner,
        start: call.ts,
        end: result?.ts ?? null,
        status: statusOf([result], active),
        calls: [
          {
            tool: call.tool,
            input: call.input,
            result: result ? { output: result.output, isError: result.isError } : null,
          },
        ],
      });
      continue;
    }
    if (open !== null && ownerKey(open.entry.owner) !== ownerKey(owner)) close();
    if (open === null) {
      const entry: TimelineEntry = {
        key: call.seq,
        kind: "tools",
        owner,
        text: "",
        start: call.ts,
        end: null,
        status: "running",
        calls: [],
      };
      out.push(entry);
      open = { entry, calls: [] };
    }
    open.calls.push(call);
  }
  close();
  return out;
}

/// Whether a run is worth a plan card (run plan spec section 2): it kept a plan, it handed work
/// on, or it took more than a couple of steps. A quick answer shows none.
export function worthShowing(plans: Plan[], timeline: TimelineEntry[]): boolean {
  if (plans.length > 0) return true;
  if (timeline.some((e) => e.kind === "helper" || e.kind === "member" || e.kind === "workflow")) {
    return true;
  }
  const calls = timeline.reduce((n, e) => n + e.calls.length, 0);
  return timeline.length >= 2 && calls >= 3;
}

/// The words of an owner, for a line that says who did a step.
export function ownerName(owner: Owner, dotName: string): string {
  return owner.kind === "dot" ? dotName : `Helper: ${owner.label}`;
}
