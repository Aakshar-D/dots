import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import { cardsAfter, makingCall, runSender, senderLabel } from "@/lib/making";
import { chat, reply } from "@/test/home";

describe("who wrote a chat run's message", () => {
  const with_ = (payload: unknown) => chat("a", "10:00", { payload });

  it("knows a maker's job, a job sent again and Sage's report from the run's payload", () => {
    const job = runSender(with_({ message: "Read.", job_id: "j1", job_from: "Researcher" }));
    expect(job).toEqual({ kind: "job", maker: "Researcher" });
    expect(senderLabel(job!)).toBe("Job from Researcher");
    const again = runSender(with_({ message: "Read.", from_maker: "Researcher" }));
    expect(senderLabel(again!)).toBe("Job from Researcher, sent again");
    const report = runSender(with_({ message: "Blog Watcher finished.", wake_id: "w1" }));
    expect(senderLabel(report!)).toBe("Report from Sage");
    // A job whose maker's name is missing is still not the person's.
    expect(senderLabel(runSender(with_({ message: "Read.", job_id: "j1" }))!)).toBe(
      "Job from its maker",
    );
  });

  it("leaves the person's own messages and every other trigger alone", () => {
    expect(runSender(with_({ message: "hello", typed: true }))).toBeNull();
    expect(runSender(with_(null))).toBeNull();
    // The person's answer to a question a job asked is theirs.
    expect(runSender(with_({ answer: "Yes.", job_id: "j1", job_from: "Researcher" }))).toBeNull();
    expect(
      runSender({ trigger: "schedule", payload: { wake_id: "w1", job_from: "Researcher" } }),
    ).toBeNull();
  });
});

describe("which making cards follow a run", () => {
  const cards = T.MakingCard.array().parse(contract.MakingCard);
  const at = (run_id: string, id: string) => ({ ...cards[0]!, id, run_id });

  it("puts each card after the run that did it, and a card of a run not shown after the last", () => {
    const first = chat("a", "10:00");
    const second = reply("b", first, "10:05");
    const all = [at("a", "1"), at("b", "2"), at("older", "3"), at("a", "4")];
    const ids = (run: typeof first, runs = [first, second], elsewhere: string | null = null) =>
      cardsAfter(all, run, runs, elsewhere).map((card) => card.id);
    expect(ids(first)).toEqual(["1", "4"]);
    expect(ids(second)).toEqual(["2", "3"]);
    // A run topic's first run is drawn as a card of its own, which shows its cards itself.
    expect(ids(second, [second], "a")).toEqual(["2", "3"]);
    expect(ids(first, [first])).toEqual(["1", "2", "3", "4"]);
  });
});

describe("a making call that waits for approval", () => {
  it("names the sol to be made from the call's arguments", () => {
    const call = makingCall("MakeDots", {
      tool: "make_dot",
      input: { name: "Blog Watcher", instructions: "Read the feeds.", browser: true },
    });
    expect(call).toEqual({
      tool: "make_dot",
      what: "make a sol:",
      name: "Blog Watcher",
      asks: [
        ["name", "Blog Watcher"],
        ["instructions", "Read the feeds."],
        ["browser", "true"],
      ],
    });
  });

  it("names a made sol as the core found it, not as the call spelled it", () => {
    const input = {
      tool: "delete_dot",
      input: { dot: "blog watcher" },
      dot: { id: "01J0MADE", name: "Blog Watcher" },
    };
    expect(makingCall("MakeDots", input)).toMatchObject({
      what: "delete the sol",
      name: "Blog Watcher",
    });
    expect(
      makingCall("MakeDots", { tool: "give_job", input: { dot: "Feeds", message: "Go." } }),
    ).toMatchObject({ what: "give a job to the sol", name: "Feeds" });
    expect(makingCall("MakeDots", { tool: "list_made_dots", input: {} })).toMatchObject({
      what: "list the sols it made",
      name: null,
    });
  });

  it("reads a call under the Claude engine's own tool name", () => {
    expect(makingCall("mcp__dots__make_dot", { name: "Feeds", instructions: "x" })).toMatchObject({
      what: "make a sol:",
      name: "Feeds",
    });
  });

  it("is none for another tool and for a shape it does not know", () => {
    expect(makingCall("Bash", { command: "ls" })).toBeNull();
    expect(makingCall("MakeDots", null)).toBeNull();
    expect(makingCall("MakeDots", { tool: "make_dot" })).toBeNull();
    expect(makingCall("MakeDots", { tool: "make_dot", input: { name: 7 } })).toBeNull();
    expect(makingCall("MakeDots", { tool: "sell_dot", input: { name: "x" } })).toBeNull();
    expect(makingCall("MakeDots", { tool: "constructor", input: { name: "x" } })).toBeNull();
    expect(makingCall("mcp__dots__remember", { text: "x" })).toBeNull();
  });
});
