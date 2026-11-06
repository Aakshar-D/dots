import { describe, expect, it } from "vitest";

import {
  capLevel,
  capOf,
  costAgainst,
  tokenCapError,
  tokensAgainst,
  TOKENS_RULE,
  usdCapError,
  USD_RULE,
  worstLevel,
} from "@/lib/spend";

describe("spending caps", () => {
  it("read an empty field as no cap and the rest as a number", () => {
    expect(capOf("")).toBeNull();
    expect(capOf("  ")).toBeNull();
    expect(capOf("2.5")).toBe(2.5);
    expect(capOf("$2.00")).toBe(2);
    expect(capOf("200,000")).toBe(200000);
    expect(capOf("two")).toBeNaN();
  });

  it("check a cost cap as the core does", () => {
    for (const ok of [null, 0.01, 2, 100000]) expect(usdCapError(ok), String(ok)).toBeUndefined();
    for (const bad of [0, -1, 0.001, 100000.5, Number.NaN, Infinity]) {
      expect(usdCapError(bad), String(bad)).toBe(USD_RULE);
    }
  });

  it("check a token cap as the core does", () => {
    for (const ok of [null, 1, 200000, 10_000_000_000]) expect(tokenCapError(ok)).toBeUndefined();
    for (const bad of [0, 1.5, -5, 10_000_000_001, Number.NaN]) {
      expect(tokenCapError(bad), String(bad)).toBe(TOKENS_RULE);
    }
  });

  it("warn from 80% of a cap and count it reached at the cap", () => {
    expect(capLevel(1.59, 2)).toBe("under");
    expect(capLevel(1.6, 2)).toBe("warn");
    expect(capLevel(2, 2)).toBe("reached");
    expect(capLevel(3, 2)).toBe("reached");
    expect(worstLevel([[5, null]])).toBeNull();
    expect(
      worstLevel([
        [1, 2],
        [190, 200],
      ]),
    ).toBe("warn");
    expect(
      worstLevel([
        [2, 2],
        [10, 200],
      ]),
    ).toBe("reached");
  });

  it("say what was spent against a cap", () => {
    expect(costAgainst(1.6, 2)).toBe("$1.60 of $2.00");
    expect(costAgainst(1.6, null)).toBe("$1.60");
    expect(tokensAgainst(120000, 200000)).toBe("120,000 of 200,000 tokens");
    expect(tokensAgainst(42, null)).toBe("42 tokens");
  });
});
