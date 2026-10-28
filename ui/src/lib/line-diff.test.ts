import { describe, expect, it } from "vitest";

import { lineDiff } from "@/lib/line-diff";

describe("a line diff of instructions", () => {
  it("keeps the common lines and marks what was removed and added", () => {
    const before = "Order the usual groceries.\nPay with the saved card.\nSend the receipt.";
    const after =
      "Order the usual groceries.\nIf an item is out of stock, buy the closest brand.\n" +
      "Send the receipt.";
    expect(lineDiff(before, after)).toEqual([
      { kind: "same", text: "Order the usual groceries." },
      { kind: "removed", text: "Pay with the saved card." },
      { kind: "added", text: "If an item is out of stock, buy the closest brand." },
      { kind: "same", text: "Send the receipt." },
    ]);
  });

  it("adds every line to an empty text and removes every line of one emptied", () => {
    expect(lineDiff("", "a\nb")).toEqual([
      { kind: "added", text: "a" },
      { kind: "added", text: "b" },
    ]);
    expect(lineDiff("a", "")).toEqual([{ kind: "removed", text: "a" }]);
    expect(lineDiff("same", "same")).toEqual([{ kind: "same", text: "same" }]);
  });

  it("appends lines added at the end after the common ones", () => {
    expect(lineDiff("Do the thing.", "Do the thing.\nThen report.")).toEqual([
      { kind: "same", text: "Do the thing." },
      { kind: "added", text: "Then report." },
    ]);
  });

  it("reads Windows line ends as the same lines", () => {
    expect(lineDiff("Do the thing.\r\nThen report.", "Do the thing.\nThen report.")).toEqual([
      { kind: "same", text: "Do the thing." },
      { kind: "same", text: "Then report." },
    ]);
  });
});
