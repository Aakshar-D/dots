import { LIMITS } from "@/api/types";
import { formatCost } from "@/lib/usage-format";

/// The core's words for a cap out of bounds (`spend::USD_RULE`, `spend::TOKENS_RULE`).
export const USD_RULE = "A cost cap is $0.01 to $100,000.";
export const TOKENS_RULE = "A token cap is 1 to 10,000,000,000 tokens.";

/// What a cap field holds: empty is no cap (null); anything else is read as a number (NaN when
/// it is not one, which the checks refuse).
export function capOf(text: string): number | null {
  const t = text.trim().replace(/^\$/, "").replaceAll(",", "");
  return t === "" ? null : Number(t);
}

/// What is wrong with a cost cap, mirroring `spend::check_usd`.
export function usdCapError(cap: number | null): string | undefined {
  if (cap === null) return undefined;
  return Number.isFinite(cap) && cap >= LIMITS.min_cap_usd && cap <= LIMITS.max_cap_usd
    ? undefined
    : USD_RULE;
}

/// What is wrong with a token cap, mirroring `spend::check_tokens`.
export function tokenCapError(cap: number | null): string | undefined {
  if (cap === null) return undefined;
  return Number.isInteger(cap) && cap >= 1 && cap <= LIMITS.max_cap_tokens
    ? undefined
    : TOKENS_RULE;
}

export type CapLevel = "under" | "warn" | "reached";

/// Where `spent` stands against `cap`: reached at the cap or over, a warning from 80%.
export function capLevel(spent: number, cap: number): CapLevel {
  if (spent + 1e-9 >= cap) return "reached";
  return spent >= cap * LIMITS.spend_warn_at ? "warn" : "under";
}

/// "200,000".
export const thousands = (n: number) => Math.round(n).toLocaleString("en-US");

/// "$1.60 of $2.00" or "$1.60" with no cap.
export function costAgainst(spent: number, cap: number | null): string {
  return cap === null ? formatCost(spent) : `${formatCost(spent)} of ${formatCost(cap)}`;
}

/// "120,000 of 200,000 tokens" or "120,000 tokens" with no cap.
export function tokensAgainst(spent: number, cap: number | null): string {
  return cap === null
    ? `${thousands(spent)} tokens`
    : `${thousands(spent)} of ${thousands(cap)} tokens`;
}

/// The worst level among the caps that are set; null when none is.
export function worstLevel(pairs: [number, number | null][]): CapLevel | null {
  const levels = pairs.flatMap(([spent, cap]) => (cap === null ? [] : [capLevel(spent, cap)]));
  if (levels.length === 0) return null;
  if (levels.includes("reached")) return "reached";
  return levels.includes("warn") ? "warn" : "under";
}
