// Browser use (browser spec): the sites a dot's rules approve, what a browser approval asks,
// and the run-view line of a browser call.
import { z } from "zod";

import type { Approval, Policy } from "@/api/types";

/// The built-in browser's tools are `mcp__browser__<tool>` on both engines.
export const BROWSER_PREFIX = "mcp__browser__";

/// A site the dot may act on without asking, and whether its risky actions are allowed too.
export type Site = { site: string; risky: boolean };

const SITE_RULE = /^(Browser|BrowserRisky)\((.+)\)$/;

/// The site of a rule `Browser(<site>)` or `BrowserRisky(<site>)`, and which of the two it is.
function siteRule(tool: string): Site | null {
  const match = SITE_RULE.exec(tool.trim());
  return match ? { site: match[2]!.trim(), risky: match[1] === "BrowserRisky" } : null;
}

/// The sites a policy approves, by name: its allow rules `Browser(<site>)` and
/// `BrowserRisky(<site>)` (an allowed `BrowserRisky` approves the site too).
export function sitesOf(policy: Policy): Site[] {
  const sites = new Map<string, boolean>();
  for (const rule of policy.rules) {
    const parsed = rule.action === "allow" ? siteRule(rule.tool) : null;
    if (parsed) sites.set(parsed.site, (sites.get(parsed.site) ?? false) || parsed.risky);
  }
  return [...sites]
    .map(([site, risky]) => ({ site, risky }))
    .sort((a, b) => a.site.localeCompare(b.site));
}

/// The policy without the allow rules of `site`. The preset becomes custom when a rule goes, as
/// when a rule is saved from an approval; the policy is returned as it is otherwise.
export function withoutSite(policy: Policy, site: string): Policy {
  const rules = policy.rules.filter(
    (rule) => !(rule.action === "allow" && siteRule(rule.tool)?.site === site),
  );
  return rules.length === policy.rules.length ? policy : { ...policy, preset: "custom", rules };
}

/// What Dots stored with a browser call that asks: `dots_browser` in the approval's input.
const BrowserAsk = z.object({
  action: z.string(),
  site: z.string().nullable(),
  page: z.string().nullable(),
  labels: z.array(z.string().nullable()),
  risky: z.boolean(),
  dangerous: z.boolean(),
});
export type BrowserAsk = z.infer<typeof BrowserAsk>;

/// The card of a browser approval; null for any other approval.
export function browserAsk(approval: Approval): BrowserAsk | null {
  if (!approval.tool.startsWith(BROWSER_PREFIX)) return null;
  const input = approval.input;
  if (typeof input !== "object" || input === null) return null;
  const parsed = BrowserAsk.safeParse((input as Record<string, unknown>).dots_browser);
  return parsed.success ? parsed.data : null;
}

/// A part of a card's sentence: plain, or in bold.
export type Part = { text: string; strong: boolean };

const plain = (text: string): Part => ({ text, strong: false });
const strong = (text: string): Part => ({ text, strong: true });

/// An element by its label in quotes; one Dots could not identify says so.
const quoted = (label: string | null | undefined) =>
  label === null || label === undefined ? "an element Dots could not identify" : `"${label}"`;

/// What the dot wants to do, as the card says it (browser spec section 3.6): "wants to open
/// **shop.com/cart**", "wants to click **"Pay now"** on **shop.com**".
export function askParts(ask: BrowserAsk): Part[] {
  const where =
    ask.site === null ? [plain(" on a page with no site")] : [plain(" on "), strong(ask.site)];
  const [first, second] = ask.labels;
  switch (ask.action) {
    case "open":
      return [plain("wants to open "), strong(ask.page ?? "a page")];
    case "click":
      return [plain("wants to click "), strong(quoted(first)), ...where];
    case "type":
      return [plain("wants to type into "), strong(quoted(first)), ...where];
    case "fill":
      return [plain("wants to fill in "), strong(ask.labels.map(quoted).join(", ")), ...where];
    case "select":
      return [plain("wants to choose an option in "), strong(quoted(first)), ...where];
    case "press":
      return [plain("wants to press "), strong(first ?? "a key"), ...where];
    case "hover":
      return [plain("wants to point at "), strong(quoted(first)), ...where];
    case "drag":
      return [
        plain("wants to drag "),
        strong(quoted(first)),
        plain(" to "),
        strong(quoted(second)),
        ...where,
      ];
    case "drop":
      return [plain("wants to drop data onto "), strong(quoted(first)), ...where];
    case "dialog":
      return [plain("wants to accept a dialog"), ...where];
    case "run_code":
      return [plain("wants to run its own code in the page"), ...where];
    case "upload":
      return [plain("wants to upload files from this computer"), ...where];
    case "network":
      return [plain("wants to read the page's network requests"), ...where];
    default:
      return [plain("wants to use "), strong(first ?? ask.action), ...where];
  }
}

/// The longest value a card shows in full.
const DETAIL_MAX = 300;

/// `text` cut to `DETAIL_MAX` characters.
function cut(text: string): string {
  return text.length > DETAIL_MAX ? `${text.slice(0, DETAIL_MAX - 1)}…` : text;
}

/// What a browser call would send, one line each, for its card: the full address, the text
/// typed, the fields filled, the files and data dropped or uploaded, the code run. Empty for a
/// call that sends nothing of its own (a click).
export function askDetails(tool: string, input: unknown): string[] {
  const args = typeof input === "object" && input !== null ? (input as Record<string, unknown>) : {};
  const text = (value: unknown) => (typeof value === "string" ? value : JSON.stringify(value));
  const list = (value: unknown) => (Array.isArray(value) ? value : []);
  const lines: string[] = [];
  const name = tool.startsWith(BROWSER_PREFIX) ? tool.slice(BROWSER_PREFIX.length) : tool;
  if (typeof args.url === "string") lines.push(`Address: ${args.url}`);
  if (name === "browser_type" && args.text !== undefined) lines.push(`Text: ${text(args.text)}`);
  for (const f of list(args.fields)) {
    const fieldArgs = typeof f === "object" && f !== null ? (f as Record<string, unknown>) : {};
    lines.push(`${text(fieldArgs.name ?? fieldArgs.target ?? "field")}: ${text(fieldArgs.value ?? "")}`);
  }
  for (const v of list(args.values)) lines.push(`Choice: ${text(v)}`);
  for (const path of list(args.paths)) lines.push(`File: ${text(path)}`);
  if (args.data !== undefined) lines.push(`Data: ${text(args.data)}`);
  for (const key of ["function", "code"]) {
    if (args[key] !== undefined) lines.push(`Code: ${text(args[key])}`);
  }
  return lines.map(cut);
}

/// The card's sentence as plain text.
export function askText(ask: BrowserAsk): string {
  return askParts(ask)
    .map((part) => part.text)
    .join("");
}

/// The trimmed string argument `key` of a call, or "".
function field(input: unknown, key: string): string {
  const value =
    typeof input === "object" && input !== null
      ? (input as Record<string, unknown>)[key]
      : undefined;
  return typeof value === "string" ? value.trim() : "";
}

/// The run-view line of a browser call (browser spec section 7): the lines Dots put in front of
/// its result (`Dots: Opened shop.com/cart`), joined; before its result, or for a call that was
/// not performed, what the call is.
export function browserLine(
  tool: string,
  input: unknown,
  result: { output: string; isError: boolean } | null,
): string {
  const reported: string[] = [];
  for (const line of (result?.output ?? "").split("\n")) {
    if (!line.startsWith("Dots: ")) break;
    reported.push(line.slice("Dots: ".length));
  }
  if (reported.length > 0) return reported.join(" · ");
  const name = tool.startsWith(BROWSER_PREFIX) ? tool.slice(BROWSER_PREFIX.length) : tool;
  const url = field(input, "url");
  switch (name) {
    case "browser_navigate":
      return url ? `Open ${url}` : "Open a page";
    case "browser_snapshot":
      return "Read the page";
    case "browser_take_screenshot":
      return "Take a screenshot";
    case "browser_click":
      return "Click";
    case "browser_type":
      return "Type";
    default:
      return `Use ${name}`;
  }
}

/// The screenshot a result names, by its file name: Playwright saves one as
/// `downloads\page-<time>.png` in the run's workspace.
export function screenshotName(output: string): string | null {
  const match = /\(downloads[\\/](page-[0-9TZ-]+\.(?:png|jpeg|webp))\)/.exec(output);
  return match ? match[1]! : null;
}
