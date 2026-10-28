import { describe, expect, it } from "vitest";

import { runTrigger } from "@/components/Transcript";
import { triggerLabel } from "@/lib/home-topics";
import { sample } from "@/test/fixtures";

describe("what started a run", () => {
  it("names the app that started it, by its title", () => {
    const run = {
      ...sample.run,
      trigger: "app" as const,
      payload: { message: "Task t1", app: "projects", app_title: "Projects", tag: "t1" },
    };
    expect(runTrigger(run)).toBe("Started by Projects");
    expect(triggerLabel(run)).toBe("Projects");
  });

  it("says an app started it when the run does not name one", () => {
    const run = { ...sample.run, trigger: "app" as const, payload: null };
    expect(runTrigger(run)).toBe("Started by an app");
    expect(triggerLabel(run)).toBe("App");
    expect(runTrigger({ ...sample.run, trigger: "manual" })).toBe("Manual run");
  });
});
