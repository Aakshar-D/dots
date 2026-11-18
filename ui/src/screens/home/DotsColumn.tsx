import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Download, Plus } from "lucide-react";
import { useState, type KeyboardEvent } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useScopeRefusal } from "@/lib/access";
import { orgRemoved, orgRemovedText, useOrgList } from "@/lib/dot-org";
import { dotStatus, lastLine, nextLine } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { ImportAgentsDialog } from "@/screens/home/ImportAgentsDialog";
import { DotAvatar } from "@/components/DotAvatar";
import { StatusMark } from "@/screens/home/StatusMark";
import { UnreadBadge } from "@/screens/home/UnreadBadge";
import { NewDotDialog } from "@/screens/setup/NewDotDialog";

/// The home's list of dots, like a list of chat contacts (chat-first home spec, section 3).
/// `rows` come sorted; `openId` is the dot whose conversation is open.
export function DotsColumn({ rows, openId }: { rows: DotOverview[]; openId: string | undefined }) {
  // Every dot's open questions mark it as waiting; the frame loads the same list for its count.
  const messages =
    useQuery({ queryKey: keys.messages(null), queryFn: () => commands.messagesList(null) }).data ??
    [];
  const followUps =
    useQuery({
      queryKey: keys.followUps(null),
      queryFn: () => commands.followUpsList(null),
    }).data ?? [];
  // A dot whose queued run waits for Claude usage says so rather than "running".
  const held =
    useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data?.held_run_ids ?? [];
  // A question the person snoozed or marked done does not make the dot wait for them.
  const hidden =
    useQuery({ queryKey: keys.inbox, queryFn: commands.inbox }).data?.hidden_messages ?? [];
  // A dot whose org is no longer joined says so; the orgs are asked for only when a dot has one.
  const orgs = useOrgList(rows.some((row) => row.dot.org_id !== null));
  // A Viewer, or a PC off the member list, can't make a dot in the account it works in.
  const refusal = useScopeRefusal("edit");
  const [importing, setImporting] = useState(false);
  // New dot opens the setup view in a dialog; the form stays at /dots/new.
  const [creating, setCreating] = useState(false);
  // With no dot in view the home already shows the setup view: New dot goes there rather than
  // opening a second one.
  function newDot() {
    const flow = document.querySelector<HTMLElement>("[data-setup-flow]");
    if (rows.length > 0 || !flow) return setCreating(true);
    (flow.querySelector<HTMLElement>("[data-setup-composer]") ?? flow).focus();
  }
  // Up and Down move the focus to the dot above or below, Home and End to the first and the
  // last; Enter (the link's own) opens the dot in focus. Moving opens nothing, so that passing
  // over dots neither loads them nor marks their questions read.
  function onKeyDown(e: KeyboardEvent<HTMLUListElement>) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const links = [...e.currentTarget.querySelectorAll<HTMLAnchorElement>("a[data-dot]")];
    const focused = links.indexOf(document.activeElement as HTMLAnchorElement);
    const from = focused !== -1 ? focused : rows.findIndex((row) => row.dot.id === openId);
    const to =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? links.length - 1
          : Math.min(links.length - 1, Math.max(0, from + (e.key === "ArrowDown" ? 1 : -1)));
    links[to]?.focus();
  }
  return (
    // Narrower in a small window, so the conversation keeps room and nothing scrolls
    // sideways (420 px self-QA).
    <nav aria-label="Sols" className="flex w-40 shrink-0 flex-col border-r bg-sidebar sm:w-64">
      {/* New dot first, then the list under a quiet heading, as a chat app's column. When the
          role in the account in view refuses making dots, the button stays, off, with the
          core's reason. */}
      <div className="px-2 pt-2">
        {refusal ? (
          <Button
            variant="ghost"
            title={refusal}
            aria-disabled="true"
            aria-describedby="new-dot-refusal"
            className="w-full cursor-not-allowed justify-start opacity-50"
          >
            <Plus aria-hidden />
            New sol
          </Button>
        ) : (
          <Button
            type="button"
            variant="ghost"
            className="w-full justify-start"
            onClick={newDot}
          >
            <Plus aria-hidden />
            New sol
          </Button>
        )}
        {creating && <NewDotDialog onClose={() => setCreating(false)} />}
        {refusal && (
          <span id="new-dot-refusal" className="sr-only">
            {refusal}
          </span>
        )}
        {/* Claude Code agents become dots in the same account, so the same refusal hides it. */}
        {!refusal && (
          <Button
            type="button"
            variant="ghost"
            className="w-full justify-start text-muted-foreground"
            onClick={() => setImporting(true)}
          >
            <Download aria-hidden />
            Import from Claude Code
          </Button>
        )}
        {importing && <ImportAgentsDialog onClose={() => setImporting(false)} />}
      </div>
      <h2 className="px-4 pt-3 pb-1 text-xs font-medium text-muted-foreground">Sols</h2>
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-auto px-2 pb-2" onKeyDown={onKeyDown}>
        {rows.map((row) => {
          const open = row.dot.id === openId;
          const next = nextLine(row, followUps);
          return (
            <li key={row.dot.id} aria-label={row.dot.name}>
              <Link
                to={`/home/${row.dot.id}`}
                data-dot={row.dot.id}
                aria-current={open ? "page" : undefined}
                className={cn(
                  "flex flex-col gap-0.5 rounded-[10px] p-2.5 hover:bg-sidebar-accent/60",
                  open && "bg-sidebar-accent hover:bg-sidebar-accent",
                )}
              >
                <span className="flex items-center gap-2">
                  <DotAvatar seed={row.dot.id} />
                  <StatusMark status={dotStatus(row, messages, held, hidden)} />
                  <span className="truncate font-medium">{row.dot.name}</span>
                  <UnreadBadge row={row} />
                </span>
                <span className="truncate pl-10.5 text-xs text-muted-foreground">
                  {lastLine(row)}
                </span>
                {next && <span className="pl-10.5 text-xs text-muted-foreground">{next}</span>}
                {row.dot.org_id !== null && orgRemoved(row.dot, orgs) && (
                  <span
                    className="pl-10.5 text-xs text-destructive"
                    title={orgRemovedText(row.dot.org_id)}
                  >
                    account removed
                  </span>
                )}
              </Link>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
