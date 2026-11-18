// Sols that make sols (dots-make-dots design sections 8 and 11): which chat runs carry a
// maker's words or Sage's report instead of the person's, and how the making cards of a topic
// fall under its runs.
import type { JobEnd, MakingCard, Run } from "@/api/types";

/// Who wrote the message of a chat run the person did not type: a maker's job for a sol it
/// made, that job's text sent again after the job ended, or Sage's report that wakes a maker.
export type RunSender =
  | { kind: "job"; maker: string | null }
  | { kind: "job_again"; maker: string | null }
  | { kind: "report" };

/// Who wrote the message of `run`, when it was not the person (the core's `job_id` and
/// `job_from`, `from_maker` and `wake_id` in the run's payload); null for every other run. The
/// person's answer to a question is theirs, whatever else the payload carries.
export function runSender(run: Pick<Run, "trigger"> & { payload?: unknown }): RunSender | null {
  if (run.trigger !== "chat" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const payload = run.payload as Record<string, unknown>;
  if (typeof payload.answer === "string") return null;
  const name = (value: unknown) => (typeof value === "string" && value !== "" ? value : null);
  if (payload.job_id != null || payload.job_from != null) {
    return { kind: "job", maker: name(payload.job_from) };
  }
  if (payload.from_maker != null) return { kind: "job_again", maker: name(payload.from_maker) };
  if (payload.wake_id != null) return { kind: "report" };
  return null;
}

/// What the header of such a run says, as a sol's list of recent runs names them in its prompt:
/// "Job from Researcher", "Report from Sage".
export function senderLabel(sender: RunSender): string {
  if (sender.kind === "report") return "Report from Sage";
  const from = `Job from ${sender.maker ?? "its maker"}`;
  return sender.kind === "job_again" ? `${from}, sent again` : from;
}

/// The text such a run carries: the maker's job, or Sage's report. Written by a sol or quoted
/// from one, so it is shown as plain text only.
export function senderText(run: Pick<Run, "payload">): string | null {
  if (typeof run.payload !== "object" || run.payload === null) return null;
  const message = (run.payload as Record<string, unknown>).message;
  return typeof message === "string" ? message : null;
}

/// The making cards that show after `run` in a topic drawn from `runs`: those the run did. The
/// cards of a run that is not among the topic's runs here (an older one that was not loaded)
/// show after the last run, so no card is lost; `elsewhere` is a run of the topic drawn by
/// another part of the screen, whose cards are left to it.
export function cardsAfter(
  cards: MakingCard[],
  run: Run,
  runs: Run[],
  elsewhere: string | null = null,
): MakingCard[] {
  const last = runs.at(-1)?.id === run.id;
  return cards.filter(
    (card) =>
      card.run_id === run.id ||
      (last && card.run_id !== elsewhere && !runs.some((r) => r.id === card.run_id)),
  );
}

/// A maker's call that waits for approval, in words: what it wants to do, the sol it names, and
/// each thing it asks as a line of plain text. `tool` is the tool's own name (`delete_dot`).
export type MakingCall = {
  tool: string;
  what: string;
  name: string | null;
  asks: [string, string][];
};

const MAKING_VERBS: Record<string, string> = {
  make_dot: "make a sol:",
  change_dot: "change the sol",
  delete_dot: "delete the sol",
  give_job: "give a job to the sol",
  list_made_dots: "list the sols it made",
};

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/// Reads an approval of a making call (dots-make-dots design sections 5 and 9). The policy name
/// is `MakeDots`, with the input `{"tool", "input"}` and, for a call that names a made sol,
/// `"dot": {"id", "name"}` as the core found it; a call shown under the Claude engine's own
/// name (`mcp__dots__make_dot`) carries the arguments alone. The sol is named from the core's
/// `dot.name` when it is there, else from the arguments. Null for any other tool, and for a
/// shape that is not this one: the card then shows the call as it shows every other.
export function makingCall(tool: string, input: unknown): MakingCall | null {
  let which: unknown;
  let args: unknown;
  let found: unknown;
  if (tool === "MakeDots" && isObject(input)) {
    which = input.tool;
    args = input.input;
    found = isObject(input.dot) ? input.dot.name : undefined;
  } else if (tool.startsWith("mcp__dots__")) {
    which = tool.slice("mcp__dots__".length);
    args = input;
  }
  if (typeof which !== "string" || !Object.hasOwn(MAKING_VERBS, which) || !isObject(args)) {
    return null;
  }
  const named = typeof found === "string" ? found : which === "make_dot" ? args.name : args.dot;
  if (which !== "list_made_dots" && (typeof named !== "string" || named === "")) return null;
  return {
    tool: which,
    what: MAKING_VERBS[which],
    name: typeof named === "string" && which !== "list_made_dots" ? named : null,
    asks: Object.entries(args).map(([key, value]) => [
      key,
      typeof value === "string" ? value : JSON.stringify(value),
    ]),
  };
}

/// How a job ended, after the made sol's name: "finished", "failed".
export function jobEndWords(how: JobEnd | null): string {
  switch (how) {
    case "succeeded":
      return "finished";
    case "failed":
      return "failed";
    case "cancelled":
      return "was stopped";
    case "deleted":
      return "was deleted before its job ended";
    case null:
      return "ended";
  }
}
