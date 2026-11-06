import { describe, expect, it } from "vitest";

import { autoLabel, isAuto, modelLabel, modelLine } from "@/lib/auto-model";

const run = (
  model: string | null,
  model_reason: string | null,
  model_fallback_from: string | null = null,
) => ({ model, model_reason, model_fallback_from });

describe("Auto's words", () => {
  it("reads Auto as the core does", () => {
    expect(isAuto("auto")).toBe(true);
    expect(isAuto(" AUTO ")).toBe(true);
    expect(isAuto("autopilot")).toBe(false);
  });

  it("names curated models and leaves other ids as they are", () => {
    expect(modelLabel("sonnet")).toBe("Sonnet");
    expect(modelLabel("Haiku")).toBe("Haiku");
    expect(modelLabel("claude-opus-4-1")).toBe("claude-opus-4-1");
  });

  it("labels a run Auto picked the model of, and only such a run", () => {
    expect(autoLabel(run("sonnet", "code work: refactor"))).toBe(
      "Auto · Sonnet — code work: refactor",
    );
    expect(autoLabel(run("opus", null))).toBeNull();
    expect(autoLabel(run(null, null))).toBeNull();
  });

  it("says in a run's details which model it used, and a fallback", () => {
    expect(modelLine(run("opus", null))).toBe("Opus");
    expect(modelLine(run("haiku", "light task: summarise"))).toBe(
      "Auto · Haiku — light task: summarise",
    );
    expect(modelLine(run("sonnet", "you asked for Opus", "opus"))).toBe(
      "Auto · Sonnet — you asked for Opus (Opus could not be used)",
    );
    expect(modelLine(run(null, null))).toBeNull();
  });
});
