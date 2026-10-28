import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { CurrentScope, DotOverview } from "@/api/types";
import { keys } from "@/lib/query";

/// The account Personal dots belong to, as the scope names it.
export const PERSONAL = "personal";

/// The account and workspace the person works in now; `undefined` while it loads.
export function useCurrentScope(): CurrentScope | undefined {
  return useQuery({ queryKey: keys.scope, queryFn: commands.scopeGet }).data;
}

/// Whether `dot` belongs to the account of `scope`: Personal matches the dots with no org.
export function inScope(dot: { org_id: string | null }, scope: CurrentScope): boolean {
  return scope.account === PERSONAL ? dot.org_id === null : dot.org_id === scope.account;
}

/// What waits for the person in each account, keyed by `"personal"` or the org id.
export function waitingByAccount(rows: DotOverview[]): Map<string, number> {
  const waiting = new Map<string, number>();
  for (const row of rows) {
    const key = row.dot.org_id ?? PERSONAL;
    waiting.set(key, (waiting.get(key) ?? 0) + row.needs_you);
  }
  return waiting;
}
