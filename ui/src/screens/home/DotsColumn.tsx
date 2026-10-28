import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Plus } from "lucide-react";
import type { KeyboardEvent } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { Button } from "@/components/ui/button";
import { useScopeRefusal } from "@/lib/access";
import { orgRemoved, orgRemovedText, useOrgList } from "@/lib/dot-org";
import { dotStatus, lastLine, nextLine } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { StatusMark } from "@/screens/home/StatusMark";
import { UnreadBadge } from "@/screens/home/UnreadBadge";

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
  // A dot whose org is no longer joined says so; the orgs are asked for only when a dot has one.
  const orgs = useOrgList(rows.some((row) => row.dot.org_id !== null));
  // A Viewer, or a PC off the member list, can't make a dot in the account it works in.
  const refusal = useScopeRefusal("edit");
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
    <nav aria-label="Dots" className="flex w-40 shrink-0 flex-col border-r bg-sidebar sm:w-64">
      <div className="flex items-center gap-2 px-3 py-2">
        <h2 className="flex-1 font-semibold">Dots</h2>
        {refusal ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="New dot"
            title={refusal}
            aria-disabled="true"
            aria-describedby="new-dot-refusal"
            className="cursor-not-allowed opacity-50"
          >
            <Plus aria-hidden />
            <span id="new-dot-refusal" className="sr-only">
              {refusal}
            </span>
          </Button>
        ) : (
          <Button asChild size="icon-sm" variant="ghost">
            <Link to="/dots/new" aria-label="New dot" title="New dot">
              <Plus aria-hidden />
            </Link>
          </Button>
        )}
      </div>
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
                  <StatusMark status={dotStatus(row, messages, held)} />
                  <span className="truncate font-medium">{row.dot.name}</span>
                  <UnreadBadge row={row} />
                </span>
                <span className="truncate pl-4.5 text-xs text-muted-foreground">
                  {lastLine(row)}
                </span>
                {next && <span className="pl-4.5 text-xs text-muted-foreground">{next}</span>}
                {row.dot.org_id !== null && orgRemoved(row.dot, orgs) && (
                  <span
                    className="pl-4.5 text-xs text-destructive"
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
