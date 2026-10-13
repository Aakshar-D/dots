import { describe, expect, it } from "vitest";

import { mcpOf, parseMcp, specErrors, specOf } from "@/lib/dot-rules";
import { blankSpec, sample } from "@/test/fixtures";

const good = { ...blankSpec, name: "nightly", instructions: "Run the tests.", workdir: "C:/dev" };

describe("dot editor rules", () => {
  it("accepts a complete dot", () => {
    expect(specErrors(good, "")).toEqual({});
    expect(specErrors(specOf(sample.dot), JSON.stringify(sample.dot.mcp_servers))).toEqual({});
  });

  it("names what a new dot still lacks", () => {
    expect(Object.keys(specErrors(blankSpec, "")).sort()).toEqual([
      "instructions",
      "name",
      "workdir",
    ]);
  });

  it("checks the name the way the core does", () => {
    for (const name of ["", "has space", "ünï", "x".repeat(65)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeDefined();
    }
    for (const name of ["a", "nightly-tests_2", "x".repeat(64)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeUndefined();
    }
  });

  it("wants an http endpoint for a local dot only", () => {
    const local = { ...good, engine: "local" as const };
    expect(specErrors({ ...local, endpoint_url: null }, "").endpoint_url).toMatch(/needs its/);
    expect(specErrors({ ...local, endpoint_url: "https://x" }, "").endpoint_url).toMatch(/https/);
    expect(specErrors({ ...local, endpoint_url: "http://" }, "").endpoint_url).toBeDefined();
    expect(specErrors({ ...local, endpoint_url: " http://127.0.0.1:1234 " }, "")).toEqual({});
    expect(specErrors({ ...good, endpoint_url: null }, "")).toEqual({});
  });

  it("keeps the limits inside the core's bounds", () => {
    const bad = { max_turns: 0, timeout_secs: 604801, approval_wait_secs: 281 };
    expect(Object.keys(specErrors({ ...good, ...bad }, "")).sort()).toEqual(Object.keys(bad).sort());
    const edge = { max_turns: 1, timeout_secs: 604800, approval_wait_secs: 280 };
    expect(specErrors({ ...good, ...edge }, "")).toEqual({});
    // An emptied number field.
    expect(specErrors({ ...good, max_turns: Number.NaN }, "").max_turns).toMatch(/whole number/);
    expect(specErrors({ ...good, timeout_secs: 1.5 }, "").timeout_secs).toBeDefined();
  });

  it("refuses a rule without a pattern", () => {
    const policy = { ...good.policy, rules: [{ tool: " ", action: "ask" as const }] };
    expect(specErrors({ ...good, policy }, "").policy).toBe("Every rule needs a tool pattern.");
  });

  it("reads the MCP servers field", () => {
    expect(parseMcp("  ")).toEqual({ value: null });
    expect(parseMcp('{"gh": {"type": "http"}}')).toEqual({ value: { gh: { type: "http" } } });
    expect(parseMcp("{nope")).toEqual({ error: "This is not valid JSON." });
    expect(parseMcp("[1]")).toHaveProperty("error");
    expect(parseMcp('"x"')).toHaveProperty("error");
    expect(parseMcp('{"dots": {}}')).toEqual({ error: "The server name 'dots' is reserved." });
    expect(specErrors(good, "{nope").mcp_servers).toBe("This is not valid JSON.");
  });

  it("checks and saves the MCP servers of a Claude dot only", () => {
    const local = { ...good, engine: "local" as const, endpoint_url: "http://127.0.0.1:1234" };
    // A local dot's form hides the field: what it still holds is neither checked nor saved.
    expect(specErrors(local, "{nope")).toEqual({});
    expect(mcpOf(local, '{"gh": {}}')).toEqual({ value: null });
    expect(mcpOf(local, "{nope")).toEqual({ value: null });
    expect(mcpOf(good, '{"gh": {}}')).toEqual({ value: { gh: {} } });
    expect(mcpOf(good, "{nope")).toEqual({ error: "This is not valid JSON." });
  });

  it("takes the editable fields of a stored dot", () => {
    const spec = specOf(sample.dot);
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("webhook_token");
    expect(spec.name).toBe(sample.dot.name);
  });
});
