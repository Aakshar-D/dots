import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { ContextRules, Run, ThreadContext } from "@/api/types";
import {
  ContextMeter,
  ContextRulesFields,
  contextEventLine,
  contextLevel,
  contextText,
  tokensShort,
  windowOf,
} from "@/components/ContextMeter";
import { CONTEXT_RULE, contextRulesError } from "@/lib/dot-rules";
import { settingsErrors } from "@/screens/SettingsScreen";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home } from "@/test/home";
import { renderApp } from "@/test/render";

const rules: ContextRules = { enabled: true, compact_at: 60, clear_at: 80 };

const thread = (over: Partial<ThreadContext> = {}): ThreadContext => ({
  root_run_id: "current",
  fill: 164_000,
  window: 200_000,
  rules,
  keep_going: false,
  request: null,
  asks_fresh: true,
  ...over,
});

describe("the context meter", () => {
  it("says the fill against the window in short numbers", () => {
    expect(tokensShort(92_000)).toBe("92k");
    expect(tokensShort(1_000_000)).toBe("1M");
    expect(tokensShort(1_500_000)).toBe("1.5M");
    expect(tokensShort(800)).toBe("800");
    expect(contextText(92_000, 200_000)).toBe("Context 92k / 200k (46%)");
  });

  it("is a meter with its words, amber from compact and red from clear", () => {
    expect(contextLevel(100_000, 200_000, rules)).toBe("ok");
    expect(contextLevel(120_000, 200_000, rules)).toBe("compact");
    expect(contextLevel(160_000, 200_000, rules)).toBe("clear");
    render(<ContextMeter fill={130_000} window={200_000} rules={rules} />);
    const meter = screen.getByRole("meter", { name: "Context" });
    expect(meter).toHaveAttribute("aria-valuenow", "65");
    expect(meter).toHaveAttribute("aria-valuetext", "Context 130k / 200k (65%)");
    expect(meter).toHaveAttribute("data-level", "compact");
    expect(screen.getByText("Context 130k / 200k (65%)")).toBeVisible();
  });

  it("takes a run's window, a [1m] model's, or 200k", () => {
    expect(windowOf({ context_window: 1_000_000, model: "sonnet" })).toBe(1_000_000);
    expect(windowOf({ context_window: null, model: "opus[1m]" })).toBe(1_000_000);
    expect(windowOf({ context_window: null, model: null })).toBe(200_000);
  });
});

describe("a topic's context", () => {
  async function ready() {
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
  }

  it("asks to start fresh when full, and sends the answer", async () => {
    home([chat("current", "10:00")]);
    backend.on("list_topics", () => []);
    let now = thread();
    backend.on("thread_context", () => now);
    backend.on("thread_context_choose", () => {
      now = thread({ request: "fresh", asks_fresh: false });
      return now;
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    expect(
      await screen.findByText("Context is 82% full — start fresh with a summary?"),
    ).toBeVisible();
    expect(screen.getByRole("meter", { name: "Context" })).toHaveAttribute(
      "aria-valuetext",
      "Context 164k / 200k (82%)",
    );
    const group = screen.getByRole("group", { name: "Context is full" });
    expect(group).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Start fresh" }));
    await waitFor(() =>
      expect(backend.callsTo("thread_context_choose")).toEqual([
        { rootRunId: "current", choice: "fresh" },
      ]),
    );
    expect(
      await screen.findByText("Your next message starts a fresh session with a summary."),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: "Keep going" })).toBeNull();
  });

  it("hides the question on Keep going", async () => {
    home([chat("current", "10:00")]);
    backend.on("list_topics", () => []);
    let now = thread();
    backend.on("thread_context", () => now);
    backend.on("thread_context_choose", () => {
      now = thread({ keep_going: true, asks_fresh: false });
      return now;
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    await user.click(await screen.findByRole("button", { name: "Keep going" }));
    await waitFor(() =>
      expect(screen.queryByText(/start fresh with a summary\?/)).toBeNull(),
    );
    expect(backend.callsTo("thread_context_choose")).toEqual([
      { rootRunId: "current", choice: "keep_going" },
    ]);
    expect(screen.getByRole("meter", { name: "Context" })).toBeVisible();
  });

  it("shows nothing before a run reported a fill", async () => {
    home([chat("current", "10:00")]);
    backend.on("list_topics", () => []);
    renderApp(`/home/${dot.id}`);
    await ready();
    expect(screen.queryByRole("meter", { name: "Context" })).toBeNull();
  });

  it("offers Compact now and Start fresh in the topic menu", async () => {
    home([chat("current", "10:00")]);
    backend.on("list_topics", () => []);
    let now = thread({ fill: null, asks_fresh: false });
    backend.on("thread_context", () => now);
    backend.on("thread_context_choose", () => {
      now = thread({ fill: 50_000, request: "compact", asks_fresh: false });
      return now;
    });
    const { user } = renderApp(`/home/${dot.id}`);
    await ready();
    await user.click(screen.getByRole("button", { name: "Topic" }));
    await user.click(await screen.findByRole("menuitem", { name: "Compact now" }));
    await waitFor(() =>
      expect(backend.callsTo("thread_context_choose")).toEqual([
        { rootRunId: "current", choice: "compact" },
      ]),
    );
    expect(
      await screen.findByText("Your next message compacts this conversation first."),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Topic" }));
    expect(await screen.findByRole("menuitem", { name: "Start fresh" })).toBeVisible();
  });
});

describe("the run page's context", () => {
  it("shows the meter, each compaction and fresh start, and the dot's totals", async () => {
    const run: Run = { ...sample.run, context_fill: 36_000, context_window: 200_000 };
    backend.on("run_thread", () => [run]);
    backend.on("get_dot", () => sample.dot);
    backend.on("list_run_events", () => []);
    backend.on("approvals_for_run", () => []);
    backend.on("worktree_diff", () => sample.diff);
    backend.on("run_context_report", () => ({
      events: [
        {
          id: "c1",
          run_id: run.id,
          dot_id: run.dot_id,
          root_run_id: run.id,
          kind: "compact",
          trigger: "auto",
          fill_before: 124_000,
          fill_after: 36_000,
          from_session: "s1",
          tokens: 3_200,
          ok: true,
          at: "2026-10-09T12:00:00.000Z",
        },
      ],
      dot_compactions: 4,
      dot_clears: 1,
      rules,
    }));
    renderApp(`/runs/${run.id}`);
    const section = await screen.findByRole("region", { name: "Context" });
    expect(section).toHaveTextContent("Context 36k / 200k (18%)");
    expect(section).toHaveTextContent(/Compacted 124k → 36k · compaction used 3k tokens · /);
    expect(section).toHaveTextContent("This dot: 4 compactions, 1 fresh start");
    // The compaction's tokens are inside the run's own, and the line says so.
    expect(screen.getByText(/includes 3k for compactions and hand-off notes/)).toBeVisible();
  });

  it("words a fresh start the person asked for", () => {
    const line = contextEventLine({
      id: "c2",
      run_id: "r",
      dot_id: "d",
      root_run_id: "r",
      kind: "clear",
      trigger: "manual",
      fill_before: 168_000,
      fill_after: null,
      from_session: "s1",
      tokens: 2_000,
      ok: true,
      at: "2026-10-09T12:00:00.000Z",
    });
    expect(line).toMatch(/^Started fresh at 168k \(you asked\) · the note used 2k tokens · /);
  });

  it("words a compaction that failed or did not help", () => {
    const base = {
      id: "c3",
      run_id: "r",
      dot_id: "d",
      root_run_id: "r",
      kind: "compact" as const,
      trigger: "auto" as const,
      fill_before: 130_000,
      from_session: "s1",
      tokens: null,
      at: "2026-10-09T12:00:00.000Z",
    };
    expect(contextEventLine({ ...base, fill_after: null, ok: false })).toMatch(
      /^Compaction failed at 130k · /,
    );
    expect(contextEventLine({ ...base, fill_after: 125_000, ok: false })).toMatch(
      /^Compacted 130k → 125k, still over the threshold · /,
    );
  });
});

describe("the context rules' fields", () => {
  it("keep thresholds within 10 to 95% with start fresh above compact", () => {
    expect(contextRulesError(rules)).toBeUndefined();
    expect(contextRulesError({ ...rules, compact_at: 9 })).toBe(CONTEXT_RULE);
    expect(contextRulesError({ ...rules, clear_at: 96 })).toBe(CONTEXT_RULE);
    expect(contextRulesError({ ...rules, clear_at: 60 })).toBe(CONTEXT_RULE);
    expect(contextRulesError({ ...rules, compact_at: Number.NaN })).toBe(CONTEXT_RULE);
    expect(
      settingsErrors({ port: 1, max_concurrent_runs: 1, context_rules: { ...rules, clear_at: 50 } }),
    ).toEqual({ context: CONTEXT_RULE });
  });

  it("are labelled, and say what is wrong", async () => {
    function Harness() {
      const [value, setValue] = useState(rules);
      return (
        <ContextRulesFields
          id="t"
          value={value}
          onChange={setValue}
          error={contextRulesError(value)}
        />
      );
    }
    const user = userEvent.setup();
    render(<Harness />);
    const compact = screen.getByLabelText("Compact at (% of context)");
    const clear = screen.getByLabelText("Start fresh at (% of context)");
    expect(compact).toHaveValue(60);
    expect(clear).toHaveValue(80);
    await user.clear(clear);
    await user.type(clear, "50");
    expect(screen.getByText(CONTEXT_RULE)).toBeVisible();
    expect(clear).toHaveAttribute("aria-invalid", "true");
    expect(clear).toHaveAccessibleDescription(CONTEXT_RULE);
    // Off: the thresholds cannot be changed.
    await user.click(screen.getByRole("switch", { name: "Compact and start fresh by themselves" }));
    expect(compact).toBeDisabled();
  });
});

describe("a dot's own context rules in its editor", () => {
  it("follow Settings until switched on, then save in bounds", async () => {
    let stored = { ...sample.dot, context_rules: null };
    backend.on("get_dot", () => stored);
    backend.on("update_dot", (args) => {
      stored = { ...stored, ...(args.spec as object) };
      return stored;
    });
    backend.on("spend_today", () => sample.spend);
    backend.on("get_settings", () => sample.settings);
    const { user } = renderApp(`/dots/${sample.dot.id}`);
    expect(
      await screen.findByText(/It follows Settings › Runs & server › Context/),
    ).toBeVisible();
    await user.click(screen.getByRole("switch", { name: "Use its own context rules" }));
    const clear = screen.getByLabelText("Start fresh at (% of context)");
    // Out of bounds: the rule shows at once and Save is off, as in Settings.
    await user.clear(clear);
    await user.type(clear, "50");
    expect(screen.getByText(CONTEXT_RULE)).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await user.clear(clear);
    await user.type(clear, "90");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_dot")).toHaveLength(1));
    const spec = backend.callsTo("update_dot")[0].spec as { context_rules: ContextRules };
    expect(spec.context_rules).toEqual({ enabled: true, compact_at: 60, clear_at: 90 });
  });
});
