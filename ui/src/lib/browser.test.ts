import { describe, expect, it } from "vitest";

import type { Action, Approval, Policy } from "@/api/types";
import { sample } from "@/test/fixtures";

import {
  askDetails,
  askText,
  browserAsk,
  browserLine,
  screenshotName,
  sitesOf,
  withoutSite,
  type BrowserAsk,
} from "./browser";

const policy = (rules: [string, Action][]): Policy => ({
  ...sample.spec.policy,
  preset: "sandboxed",
  rules: rules.map(([tool, action]) => ({ tool, action })),
});

describe("the sites a dot's rules approve", () => {
  it("lists the allowed sites with their risky actions, and removes one", () => {
    const p = policy([
      ["Read", "allow"],
      ["Browser(shop.com)", "allow"],
      ["BrowserRisky(shop.com)", "allow"],
      ["BrowserRisky(pay.example)", "allow"],
      ["Browser(news.example)", "allow"],
      ["Browser(bank.com)", "deny"],
      ["Browser(ask.example)", "ask"],
    ]);
    expect(sitesOf(p)).toEqual([
      { site: "news.example", risky: false },
      { site: "pay.example", risky: true },
      { site: "shop.com", risky: true },
    ]);
    const removed = withoutSite(p, "shop.com");
    expect(removed.rules.map((r) => r.tool)).toEqual([
      "Read",
      "BrowserRisky(pay.example)",
      "Browser(news.example)",
      "Browser(bank.com)",
      "Browser(ask.example)",
    ]);
    expect(removed.preset).toBe("custom");
    expect(withoutSite(p, "other.example")).toBe(p);
  });
});

const ask = (over: Partial<BrowserAsk>): BrowserAsk => ({
  action: "click",
  site: "shop.com",
  page: "shop.com/cart",
  labels: ["Pay now"],
  risky: true,
  dangerous: false,
  ...over,
});

describe("a browser approval's card", () => {
  it("says what and where, in the spec's words", () => {
    expect(askText(ask({ action: "open", labels: [], risky: false }))).toBe(
      "wants to open shop.com/cart",
    );
    expect(askText(ask({}))).toBe('wants to click "Pay now" on shop.com');
    expect(askText(ask({ action: "type", labels: ["Search"], risky: false }))).toBe(
      'wants to type into "Search" on shop.com',
    );
    expect(askText(ask({ labels: [null] }))).toBe(
      "wants to click an element Dots could not identify on shop.com",
    );
    expect(askText(ask({ site: null, page: "about:blank" }))).toBe(
      'wants to click "Pay now" on a page with no site',
    );
    expect(askText(ask({ action: "fill", labels: ["Search", "Email"] }))).toBe(
      'wants to fill in "Search", "Email" on shop.com',
    );
    expect(askText(ask({ action: "press", labels: ["Enter"] }))).toBe(
      "wants to press Enter on shop.com",
    );
    expect(askText(ask({ action: "drag", labels: ["Item", "Bin"] }))).toBe(
      'wants to drag "Item" to "Bin" on shop.com',
    );
    expect(askText(ask({ action: "dialog", labels: [] }))).toBe(
      "wants to accept a dialog on shop.com",
    );
    expect(askText(ask({ action: "run_code", labels: [], dangerous: true }))).toBe(
      "wants to run its own code in the page on shop.com",
    );
    expect(askText(ask({ action: "other", labels: ["checkout_now"], dangerous: true }))).toBe(
      "wants to use checkout_now on shop.com",
    );
  });

  it("is read only from a browser tool's card", () => {
    const card = ask({});
    const browser: Approval = {
      ...sample.approval,
      tool: "mcp__browser__browser_click",
      input: { target: "e16", dots_browser: card },
    };
    expect(browserAsk(browser)).toEqual(card);
    expect(browserAsk({ ...browser, tool: "mcp__files__write" })).toBeNull();
    expect(browserAsk({ ...browser, input: { target: "e16" } })).toBeNull();
    expect(browserAsk(sample.approval)).toBeNull();
  });
});

describe("a browser call in the run view", () => {
  const done = (output: string) => ({ output, isError: false });

  it("is the lines Dots put in front of its result", () => {
    expect(
      browserLine(
        "mcp__browser__browser_navigate",
        { url: "https://shop.com/cart" },
        done("Dots: Opened shop.com/cart\n### Ran Playwright code"),
      ),
    ).toBe("Opened shop.com/cart");
    expect(
      browserLine(
        "mcp__browser__browser_click",
        { target: "e5" },
        done('Dots: Clicked "Add to cart"\nDots: Downloaded statement.pdf\n### Page'),
      ),
    ).toBe('Clicked "Add to cart" · Downloaded statement.pdf');
    // A line further down is the page's, not Dots'.
    expect(
      browserLine("mcp__browser__browser_click", { target: "e5" }, done("### Page\nDots: Paid")),
    ).toBe("Click");
    expect(
      browserLine("mcp__browser__browser_navigate", { url: "https://shop.com/cart" }, null),
    ).toBe("Open https://shop.com/cart");
    expect(
      browserLine("mcp__browser__browser_evaluate", {}, { output: "Denied", isError: true }),
    ).toBe("Use browser_evaluate");
  });

  it("finds the screenshot its result names", () => {
    expect(
      screenshotName(
        "### Result\n- [Screenshot of viewport](downloads\\page-2026-10-21T12-16-35-575Z.png)",
      ),
    ).toBe("page-2026-10-21T12-16-35-575Z.png");
    expect(screenshotName("- [Snapshot](./snap.md)")).toBeNull();
    expect(screenshotName("(downloads/../secret.png)")).toBeNull();
  });
});

// Review fix: the card shows what the call will really send, not only where.
describe("askDetails", () => {
  it("shows the full address, the text, the fields, the files and the code", () => {
    expect(askDetails("mcp__browser__browser_navigate", { url: "https://shop.com/pay?to=me&sum=900" })).toEqual([
      "Address: https://shop.com/pay?to=me&sum=900",
    ]);
    expect(askDetails("mcp__browser__browser_type", { target: "e5", text: "hats" })).toEqual(["Text: hats"]);
    expect(
      askDetails("mcp__browser__browser_fill_form", {
        fields: [
          { target: "e1", name: "Email", type: "textbox", value: "me@example.com" },
          { target: "e2", name: "Remember", type: "checkbox", value: "true" },
        ],
      }),
    ).toEqual(["Email: me@example.com", "Remember: true"]);
    expect(
      askDetails("mcp__browser__browser_file_upload", { paths: ["C:\Users\me\.ssh\id_ed25519"] }),
    ).toEqual(["File: C:\Users\me\.ssh\id_ed25519"]);
    expect(askDetails("mcp__browser__browser_drop", { target: "e3", paths: ["a.txt"], data: { "text/plain": "x" } })).toEqual([
      "File: a.txt",
      "Data: {\"text/plain\":\"x\"}",
    ]);
    expect(askDetails("mcp__browser__browser_evaluate", { function: "() => document.cookie" })).toEqual([
      "Code: () => document.cookie",
    ]);
    expect(askDetails("mcp__browser__browser_select_option", { target: "e4", values: ["Large"] })).toEqual([
      "Choice: Large",
    ]);
    expect(askDetails("mcp__browser__browser_click", { target: "e16" })).toEqual([]);
    // Long values are cut.
    const long = askDetails("mcp__browser__browser_type", { text: "x".repeat(500) })[0];
    expect(long.length).toBeLessThan(320);
    expect(long.endsWith("…")).toBe(true);
  });
});
