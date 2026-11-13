import { useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { AuditEvent, AuditView, HistoryRow, OrgSummary } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { localDateTime, localDayTime } from "@/lib/format";
import { keys } from "@/lib/query";

/// How often an account's audit view is read again while it is on screen: a send changes it
/// with no runtime event.
export const AUDIT_REFRESH_MS = 30_000;

/// The account's History and how sending its audit records stands.
export function useAuditView(orgId: string) {
  return useQuery({
    queryKey: keys.orgAudit(orgId),
    queryFn: () => commands.orgAudit(orgId),
    refetchInterval: AUDIT_REFRESH_MS,
  });
}

export const EVENT: Record<AuditEvent, string> = {
  resolved: "Used",
  denied: "Refused",
  fetched: "Fetched",
  offline: "Offline",
  revoked: "Access revoked",
  expired: "Expired",
  policy_invalid: "Policy invalid",
};

const STATE: Record<HistoryRow["state"], string> = {
  local: "Kept on this PC",
  queued: "Waiting to send",
  sent: "Sent",
  dropped: "Dropped unsent",
};

/// The core's reason codes (docs/vault-audit-format.md) in plain words.
const REASON: Record<string, string> = {
  not_granted: "not granted",
  not_defined: "not in the account",
  denied_by_policy: "the policy does not allow it",
  denied_by_workspace: "the workspace does not allow it",
  shell_secrets_forbidden: "no secrets in the shell",
  org_offline: "offline, with no saved copy",
  org_expired: "the saved copy expired",
  org_revoked: "access revoked",
  policy_invalid: "the policy is invalid",
  org_removed: "the account is not joined",
  not_member: "this PC is not on the member list",
  denied: "the source refused the sign-in",
  removed: "taken off the member list",
  offline_copy: "the saved copy ran out",
  queue_full: "the queue was full; the oldest records were dropped",
};

/// `mcp:github` as "github", the shell as "Shell".
function consumerText(consumer: string | null): string {
  if (!consumer) return "";
  return consumer === "shell" ? "Shell" : consumer.replace(/^mcp:/, "");
}

/// What Overview and History say while sending fails: how many records wait, when the last
/// send failed and why. null while sending works, or when the account sends no records.
export function auditProblem(view: AuditView | undefined): string | null {
  if (!view?.endpoint || !view.send.last_error) return null;
  const waiting = `${view.queued.toLocaleString()} ${view.queued === 1 ? "record" : "records"}`;
  // A failure can be days old (no token, nothing written since): give its date then.
  const when = localDayTime(view.send.failed_at);
  return `Audit: ${waiting} waiting; last send failed ${when}: ${view.send.last_error}`;
}

/// The Overview line for a failing send, or nothing.
export function AuditProblem({ orgId }: { orgId: string }) {
  const audit = useAuditView(orgId);
  const problem = auditProblem(audit.data);
  return problem ? <p className="text-xs text-needs">{problem}</p> : null;
}

/// The History tab: this PC's audit records for the account, newest first. A record never
/// holds a value.
export function AccountHistory({ org }: { org: OrgSummary }) {
  const audit = useAuditView(org.id);
  if (audit.isPending) return <p className="text-xs text-muted-foreground">Loading…</p>;
  if (audit.isError) return <ErrorNote error={audit.error} />;
  const view = audit.data;
  const problem = auditProblem(view);
  return (
    <div className="flex flex-col gap-2 text-xs">
      {problem && <p className="text-needs">{problem}</p>}
      <p className="text-muted-foreground">
        {view.endpoint
          ? `Records go to ${view.endpoint}.`
          : `${org.name} has no audit endpoint: these records stay on this PC.`}{" "}
        The last 30 days on this PC, newest first. Values are never recorded.
      </p>
      {view.history.length === 0 ? (
        <p className="text-muted-foreground">Nothing recorded yet.</p>
      ) : (
        <table aria-label={`History of ${org.name}`} className="w-full text-left">
          <thead className="text-muted-foreground">
            <tr>
              <th className="font-normal">When</th>
              <th className="font-normal">What</th>
              <th className="font-normal">Variable</th>
              <th className="font-normal">Sol</th>
              <th className="font-normal">For</th>
              <th className="font-normal">Why</th>
              <th className="font-normal">Sent</th>
            </tr>
          </thead>
          <tbody>
            {view.history.map((row) => (
              <tr key={row.id}>
                <td>{localDateTime(row.at)}</td>
                <td>{EVENT[row.event]}</td>
                <td className="font-mono">{row.name ?? ""}</td>
                <td>{row.dot_name ?? ""}</td>
                <td>{consumerText(row.consumer)}</td>
                <td>{row.reason ? (REASON[row.reason] ?? row.reason) : ""}</td>
                <td>{STATE[row.state]}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
