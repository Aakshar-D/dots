import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { OrgSummary, UseRules } from "@/api/types";
import { keys } from "@/lib/query";

/// What a dot whose org is not joined on this PC says, on its card and in its editor, and why
/// it can't be saved or run.
export function orgRemovedText(orgId: string): string {
  return `Its org '${orgId}' was removed. Join it again to run this dot.`;
}

/// Whether `dot`'s org is not joined on this PC, told from the joined orgs (`org_list`), as the
/// core tells it (`Vault::org_removed`): the list holds every joined org, a damaged one too,
/// and not a SOPS join that still waits for its key. False for a Personal dot, and while the
/// list is not loaded, so nothing is marked before the answer.
export function orgRemoved(dot: { org_id: string | null }, orgs: OrgSummary[] | undefined) {
  return dot.org_id !== null && orgs !== undefined && !orgs.some((org) => org.id === dot.org_id);
}

/// The orgs joined on this PC, asked for only when `enabled`: a screen with no dot of an org
/// makes no call. `undefined` until they are loaded.
export function useOrgList(enabled: boolean): OrgSummary[] | undefined {
  return useQuery({ queryKey: keys.orgs, queryFn: commands.orgList, enabled }).data;
}

/// A dot's org (`orgId`, null for Personal): the joined org, its name for the screen (the id
/// while the list loads, or when it was removed), and whether it was removed.
export function useDotOrg(orgId: string | null) {
  const orgs = useOrgList(orgId !== null);
  const org = orgs?.find((o) => o.id === orgId);
  const removed = orgRemoved({ org_id: orgId }, orgs);
  return { org, name: org?.name ?? orgId ?? "Personal", removed };
}

/// Whether an org's `use` rules let its dots be granted `name`, as the core decides it
/// (`OrgPolicy::may_use`): an `allow` glob matches and no `deny` glob does. No rules (no
/// accepted policy) allow nothing.
export function mayUse(rules: UseRules | null, name: string): boolean {
  if (rules === null) return false;
  const matches = (globs: string[]) => globs.some((glob) => globMatches(glob, name));
  return !matches(rules.deny) && matches(rules.allow);
}

/// A `use` glob against a name: `*` stands for any run of characters, and case does not
/// matter, since names compare that way.
function globMatches(glob: string, name: string): boolean {
  const [g, n] = [glob.toLowerCase(), name.toLowerCase()];
  let [gi, ni] = [0, 0];
  let star: [number, number] | null = null;
  while (ni < n.length) {
    if (gi < g.length && g[gi] === "*") {
      star = [gi, ni];
      gi += 1;
    } else if (gi < g.length && g[gi] === n[ni]) {
      gi += 1;
      ni += 1;
    } else if (star !== null) {
      gi = star[0] + 1;
      ni = star[1] + 1;
      star = [star[0], star[1] + 1];
    } else {
      return false;
    }
  }
  return [...g.slice(gi)].every((c) => c === "*");
}
