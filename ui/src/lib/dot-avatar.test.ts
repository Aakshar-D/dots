import { describe, expect, it } from "vitest";

import { AVATAR_MARKS, AVATAR_SHAPES, avatarFor } from "@/lib/dot-avatar";

const seeds = (n: number) => Array.from({ length: n }, (_, i) => `dot-${i}-${i * 7919}`);

describe("avatarFor", () => {
  it("gives the same avatar for the same seed", () => {
    expect(avatarFor("abc")).toEqual(avatarFor("abc"));
    expect(avatarFor("")).toEqual(avatarFor(""));
  });

  it("always gives a valid shape, hue and mark", () => {
    for (const seed of ["", "a", "Ünïcode ✓", ...seeds(500)]) {
      const a = avatarFor(seed);
      expect(AVATAR_SHAPES).toContain(a.shape);
      expect(AVATAR_MARKS).toContain(a.mark);
      expect(Number.isInteger(a.hue)).toBe(true);
      expect(a.hue).toBeGreaterThanOrEqual(0);
      expect(a.hue).toBeLessThan(360);
    }
  });

  it("spreads 200 seeds over every shape and mark, and over many hues", () => {
    const all = seeds(200).map(avatarFor);
    expect(new Set(all.map((a) => a.shape))).toEqual(new Set(AVATAR_SHAPES));
    expect(new Set(all.map((a) => a.mark))).toEqual(new Set(AVATAR_MARKS));
    expect(new Set(all.map((a) => Math.floor(a.hue / 30))).size).toBeGreaterThanOrEqual(10);
  });

  it("tells near seeds apart", () => {
    const keys = new Set(["a1", "a2", "a3", "a4", "a5", "a6"].map((s) => JSON.stringify(avatarFor(s))));
    expect(keys.size).toBeGreaterThan(3);
  });
});
