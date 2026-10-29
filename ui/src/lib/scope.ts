import { useIsMutating, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import { commands } from "@/api/commands";
import type { CurrentScope, DotOverview, OrgSummary, WorkspaceView } from "@/api/types";
import { orgRemoved } from "@/lib/dot-org";
import { keys } from "@/lib/query";

/// The key of a switch the person picked in the account switcher, so that the home does not
/// follow a link to another account meanwhile.
export const accountPickKey = ["account-pick"] as const;

/// Whether a switch the person picked is on its way.
export function usePickingAccount(): boolean {
  return useIsMutating({ mutationKey: accountPickKey }) > 0;
}

/// The account Personal dots belong to, as the scope names it.
export const PERSONAL = "personal";

/// The account and workspace the person works in now; `undefined` while it loads.
export function useCurrentScope(): CurrentScope | undefined {
  return useQuery({ queryKey: keys.scope, queryFn: commands.scopeGet }).data;
}

/// Switches the current account (`scopeSet`), then reloads the scope so every screen follows.
/// The promise rejects when the core refuses the switch.
export function useSwitchAccount(): (account: string) => Promise<void> {
  const queryClient = useQueryClient();
  return useCallback(
    async (account: string) => {
      await commands.scopeSet(account, "default");
      await queryClient.invalidateQueries({ queryKey: keys.scope });
    },
    [queryClient],
  );
}

/// Switches the workspace within `account` (`scopeSet`), then reloads the scope.
export function useSwitchWorkspace(): (account: string, workspace: string) => Promise<void> {
  const queryClient = useQueryClient();
  return useCallback(
    async (account: string, workspace: string) => {
      await commands.scopeSet(account, workspace);
      await queryClient.invalidateQueries({ queryKey: keys.scope });
    },
    [queryClient],
  );
}

/// The workspaces of `account` (`"personal"` or an org id) this PC may see; `undefined` while
/// they load, and while `enabled` is false.
export function useWorkspaces(account: string, enabled = true): WorkspaceView[] | undefined {
  return useQuery({
    queryKey: keys.workspaces(account),
    queryFn: () => commands.workspaceList(account),
    enabled,
  }).data;
}

/// The account `dot` is listed under: its org, or Personal for a dot with none and for one
/// whose org is no longer joined (`orgs`, the joined orgs). `undefined` for a dot with an org
/// while the joined orgs load, so nothing is guessed.
export function accountOf(
  dot: { org_id: string | null },
  orgs: OrgSummary[] | undefined,
): string | undefined {
  if (dot.org_id === null || orgRemoved(dot, orgs)) return PERSONAL;
  return orgs === undefined ? undefined : dot.org_id;
}

/// Whether `dot` belongs to the account of `scope`: Personal matches the dots with no org, and
/// those whose org is no longer joined once the joined orgs (`orgs`) are known, and is in the
/// workspace of `scope` (a dot with no `workspace_id` is in the default one).
export function inScope(
  dot: { org_id: string | null; workspace_id?: string },
  scope: CurrentScope,
  orgs?: OrgSummary[],
): boolean {
  if ((dot.workspace_id ?? "default") !== scope.workspace) return false;
  if (scope.account === PERSONAL) return dot.org_id === null || orgRemoved(dot, orgs);
  return dot.org_id === scope.account;
}

/// What waits for the person in each account, keyed by `"personal"` or the org id; a dot whose
/// org is no longer joined counts under Personal, where it is listed.
export function waitingByAccount(rows: DotOverview[], orgs?: OrgSummary[]): Map<string, number> {
  const waiting = new Map<string, number>();
  for (const row of rows) {
    const key = orgRemoved(row.dot, orgs) ? PERSONAL : (row.dot.org_id ?? PERSONAL);
    waiting.set(key, (waiting.get(key) ?? 0) + row.needs_you);
  }
  return waiting;
}
