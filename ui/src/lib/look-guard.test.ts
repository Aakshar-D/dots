// Guards of the Deep Teal look that the contrast table cannot see (deep-teal spec sections 2
// and 4): colours from Tailwind's palette bypass the tokens, and a control that removes the
// outline must draw the shared focus ring instead.
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";

import { describe, expect, it } from "vitest";

const SRC = path.resolve(import.meta.dirname, "..");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return sources(full);
    return /\.tsx$/.test(name) && !/\.test\.tsx$/.test(name) ? [full] : [];
  });
}

const read = (relative: string) => readFileSync(path.join(SRC, relative), "utf8");

describe("the look's guards", () => {
  it("colours text, backgrounds and borders with the tokens, not Tailwind's palette", () => {
    const palette =
      /\b(?:text|bg|border|ring|outline)-(?:red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|slate|gray|zinc|neutral|stone)-\d{2,3}\b/g;
    const found = sources(SRC).flatMap((file) =>
      [...readFileSync(file, "utf8").matchAll(palette)].map(
        (m) => `${path.relative(SRC, file)}: ${m[0]}`,
      ),
    );
    expect(found).toEqual([]);
  });

  it("gives switches, selects and tabs the shared focus ring, not a faint one", () => {
    for (const file of [
      "components/ui/switch.tsx",
      "components/ui/native-select.tsx",
      "components/ui/tabs.tsx",
    ]) {
      const source = read(file);
      expect(source, file).toContain("focus-visible:outline-ring");
      expect(source, file).not.toContain("ring-ring/50");
    }
  });

  it("writes inactive tabs in the muted text colour, which keeps AA on the tab list", () => {
    expect(read("components/ui/tabs.tsx")).not.toContain("text-foreground/60");
  });

  it("draws the idle status mark solid, so it keeps 3:1 against the column", () => {
    expect(read("screens/home/StatusMark.tsx")).not.toMatch(/bg-muted-foreground\/\d+/);
  });

  it("tints destructive buttons within the measured alphas", () => {
    const button = read("components/ui/button.tsx");
    // Light hover at most /15; dark at rest /15 and hovered /20.
    expect(button).not.toMatch(/(?<!dark:)hover:bg-destructive\/(?:2|3)\d/);
    expect(button).not.toMatch(/dark:(?:hover:)?bg-destructive\/(?:25|3\d)/);
  });
});
