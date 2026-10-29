import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { DotAccess, OrgSummary, Role } from "@/api/types";
import { keys } from "@/lib/query";
import { PERSONAL, useCurrentScope, useWorkspaces } from "@/lib/scope";

/// This PC's standing in an account, as the core's `Access` tells it (members spec section 6):
/// Personal is always Owner, an account with no member list acts as Member, and an account
/// whose list this PC is not on gives nothing.
export type Access = {
  kind: "owner" | "role" | "legacy" | "waiting" | "removed";
  role?: Role;
  /// For waiting and removed: the core's own refusal (it tells a changed Dots ID from plain
  /// waiting).
  text?: string;
};

/// What a person does that a role may not: the three the UI switches controls by, as the
/// core's `Refusals` name them.
export type Action = "edit" | "run" | "answer";

/// This PC's access to `orgId` (null is Personal), told from the joined `org` (undefined when
/// it is not joined, which acts as Legacy, as in the core). An active member without a role
/// fails closed, as waiting.
export function accessOf(org: OrgSummary | undefined, orgId: string | null): Access {
  if (orgId === null) return { kind: "owner" };
  if (org === undefined) return { kind: "legacy" };
  switch (org.membership) {
    case "legacy":
      return { kind: "legacy" };
    case "removed":
      return { kind: "removed", text: org.not_member_text ?? undefined };
    case "waiting":
      return { kind: "waiting", text: org.not_member_text ?? undefined };
    case "active":
      return org.role === null ? { kind: "waiting" } : { kind: "role", role: org.role };
  }
}

/// What a refusal can name: the three actions, and managing the member list (Admin or Owner
/// only, which `allows` doesn't decide: the core's `MembersView.can_manage` does).
export type Refused = Action | "members";

const ACTION: Record<Refused, string> = {
  edit: "create or change dots",
  run: "run dots",
  answer: "answer approvals",
  members: "manage members",
};

/// "Admin" for `admin`.
export function roleLabel(role: Role): string {
  return role.charAt(0).toUpperCase() + role.slice(1);
}

/// The role `access` shows as, or null when it holds none (Personal is Owner; no list is Member).
export function accessRole(access: Access): Role | null {
  switch (access.kind) {
    case "owner":
      return "owner";
    case "legacy":
      return "member";
    case "role":
      return access.role ?? null;
    default:
      return null;
  }
}

/// Why `access` doesn't allow `p`, in the core's words (`Error::NotAllowed`).
export function notAllowedText(accountName: string, access: Access, p: Refused): string {
  switch (access.kind) {
    case "waiting":
      return (
        access.text ??
        `${accountName}: you aren't on its member list yet. Send your Dots ID to an Admin.`
      );
    case "removed":
      return access.text ?? `${accountName}: you were removed from it.`;
    default: {
      const role = accessRole(access);
      return `${accountName}: a ${role ? roleLabel(role) : "Member"} can't ${ACTION[p]}. Ask an Admin or Owner.`;
    }
  }
}

/// Why the core refuses `p` in the workspace `workspace` of `account` (`"personal"` or an org
/// id), from that workspace's `refusals`; undefined while it allows it, while the scope or the
/// workspaces load (so nothing is switched off before the answer), and for a workspace this PC
/// doesn't see, which the core refuses when asked.
export function useWorkspaceRefusal(
  account: string | undefined,
  workspace: string | undefined,
  p: Action,
): string | undefined {
  const views = useWorkspaces(account ?? PERSONAL, account !== undefined);
  return views?.find((w) => w.id === workspace)?.refusals[p] ?? undefined;
}

/// What the core says the person may not do with the dot `dotId` (`dot_access`); undefined
/// while it loads and without a dot.
export function useDotAccess(dotId: string | undefined): DotAccess | undefined {
  return useQuery({
    queryKey: keys.dotAccess(dotId ?? ""),
    queryFn: () => commands.dotAccess(dotId ?? ""),
    enabled: dotId !== undefined,
  }).data;
}

/// Why the core refuses `p` on the dot of the run `runId` (its account and workspace), for the
/// approval cards; undefined while it allows it or while that loads.
export function useRunRefusal(runId: string, p: Action): string | undefined {
  const run = useQuery({ queryKey: keys.run(runId), queryFn: () => commands.getRun(runId) }).data;
  return useDotAccess(run?.dot_id)?.[p] ?? undefined;
}

/// Why the core refuses `p` in the workspace the person works in now (the scope), for the New
/// dot button.
export function useScopeRefusal(p: Action): string | undefined {
  const scope = useCurrentScope();
  return useWorkspaceRefusal(scope?.account, scope?.workspace, p);
}
