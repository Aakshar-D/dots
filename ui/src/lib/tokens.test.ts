import { readFileSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

import { CONTRAST_PAIRS, THEMES, contrastRatio, pairColours, type ThemeName } from "@/lib/tokens";

const css = readFileSync(path.resolve(import.meta.dirname, "../index.css"), "utf8");

/// The declarations of the block that starts with `selector {` at the start of a line.
function block(selector: string): Map<string, string> {
  const start = css.search(new RegExp(String.raw`^${selector.replace(".", "\\.")} \{`, "m"));
  expect(start, `${selector} block`).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf("}", start));
  return new Map(
    [...body.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1]!, m[2]!.trim()]),
  );
}

const SELECTOR: Record<ThemeName, string> = { light: ":root", dark: ".dark" };

describe("the Deep Teal tokens", () => {
  it("index.css declares every token with the value of the table, in both themes", () => {
    for (const theme of ["light", "dark"] as const) {
      const declared = block(SELECTOR[theme]);
      for (const [name, value] of Object.entries(THEMES[theme])) {
        expect(declared.get(name), `${theme} --${name}`).toBe(value);
      }
    }
  });

  it("lists in the table every colour index.css declares, so none escapes the contrast test", () => {
    for (const theme of ["light", "dark"] as const) {
      const declared = [...block(SELECTOR[theme]).keys()].filter(
        (name) => !name.startsWith("chart-") && name !== "radius",
      );
      const listed = Object.keys(THEMES[theme]);
      expect(declared.filter((name) => !listed.includes(name)), theme).toEqual([]);
    }
  });

  it("measures contrast as WCAG does", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
    expect(contrastRatio("#767676", "#ffffff")).toBeCloseTo(4.54, 2);
  });

  it("gives every text and focus ring its AA contrast, in both themes", () => {
    const failures: string[] = [];
    for (const theme of ["light", "dark"] as const) {
      for (const pair of CONTRAST_PAIRS) {
        if (pair.theme !== undefined && pair.theme !== theme) continue;
        const [text, background] = pairColours(THEMES[theme], pair);
        const ratio = contrastRatio(text, background);
        if (ratio < pair.min) failures.push(`${theme}: ${pair.label} is ${ratio.toFixed(2)}`);
      }
    }
    expect(failures).toEqual([]);
  });
});
