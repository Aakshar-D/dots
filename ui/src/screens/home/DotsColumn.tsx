import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import { Download, Plus } from "lucide-react";
import { useState, type KeyboardEvent } from "react";
import { Link, useNavigate } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Button } from "@/components/ui/button";
import { useScopeRefusal } from "@/lib/access";
import { orgRemoved, orgRemovedText, useOrgList } from "@/lib/dot-org";
import { dotStatus, lastLine, nextLine } from "@/lib/dot-status";
import { errorMessage } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";
import { ImportAgentsDialog } from "@/screens/home/ImportAgentsDialog";
import { DotAvatar } from "@/components/DotAvatar";
import { StatusMark } from "@/screens/home/StatusMark";
import { UnreadBadge } from "@/screens/home/UnreadBadge";
import { NewDotDialog } from "@/screens/setup/NewDotDialog";

/// The rows with each made sol right under its maker, one step in for each maker above it
/// (dots-make-dots design section 11). The order given is kept among the sols at the top and
/// among the sols of one maker. A made sol whose maker is deleted, or is not in the list, stays
/// at the top.
export function underMakers(rows: DotOverview[]): { row: DotOverview; depth: number }[] {
  const ids = new Set(rows.map((row) => row.dot.id));
  const makerOf = (row: DotOverview) => {
    const by = row.dot.made?.by_dot_id ?? null;
    return by !== null && by !== row.dot.id && ids.has(by) ? by : null;
  };
  const placed: { row: DotOverview; depth: number }[] = [];
  const seen = new Set<string>();
  const place = (row: DotOverview, depth: number) => {
    if (seen.has(row.dot.id)) return;
    seen.add(row.dot.id);
    placed.push({ row, depth });
    for (const made of rows) if (makerOf(made) === row.dot.id) place(made, depth + 1);
  };
  for (const row of rows) if (makerOf(row) === null) place(row, 0);
  // Makers that name each other in a ring have no top: their sols still show.
  for (const row of rows) place(row, 0);
  return placed;
}

/// The step in of a made sol's row; a chain of makers stops stepping in at the second.
const STEP_IN = ["", "ml-4", "ml-8"];

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
  // Select turns the list into one of check boxes, to delete several sols at once: the ids
  // picked so far, or null while the list is the usual one of links.
  const [picked, setPicked] = useState<ReadonlySet<string> | null>(null);
  // Each made sol stands under its maker.
  const placed = underMakers(rows);
  // Only the sols still in the list count: one deleted elsewhere meanwhile is not asked for.
  // In the list's order, so a maker is deleted before the sols it made: deleting a made sol
  // first can wake its maker, which starts a run of the maker just before it is deleted.
  const chosen =
    picked === null ? [] : placed.map(({ row }) => row).filter((row) => picked.has(row.dot.id));
  const qc = useQueryClient();
  const navigate = useNavigate();
  // One sol after the other, each on its own: one that cannot be deleted does not keep the
  // others, and stays picked with its reason shown.
  const remove = useMutation({
    mutationFn: async (dots: { id: string; name: string }[]) => {
      const deleted: string[] = [];
      const failed: { id: string; name: string; reason: string }[] = [];
      for (const dot of dots) {
        try {
          await commands.deleteDot(dot.id);
          // Removed first: a deleted sol must not be loaded again.
          qc.removeQueries({ queryKey: keys.dot(dot.id) });
          deleted.push(dot.id);
        } catch (e) {
          failed.push({ ...dot, reason: errorMessage(e) });
        }
      }
      return { deleted, failed };
    },
    onSuccess: ({ deleted, failed }) => {
      invalidateDots(qc);
      if (deleted.length > 0) {
        toast.success(deleted.length === 1 ? "1 sol deleted" : `${deleted.length} sols deleted`);
      }
      for (const dot of failed) toast.error(`${dot.name} was not deleted: ${dot.reason}`);
      setPicked(failed.length > 0 ? new Set(failed.map((dot) => dot.id)) : null);
      if (openId !== undefined && deleted.includes(openId)) void navigate("/home");
    },
  });
  function toggle(id: string) {
    const next = new Set(picked);
    if (!next.delete(id)) next.add(id);
    setPicked(next);
  }
  const everyPicked = rows.length > 0 && chosen.length === rows.length;
  // Makers among the sols picked: what they made is not deleted with them.
  const pickedMakers = chosen.filter((row) => row.made_dots > 0);
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
      <div className="flex items-center gap-1 pt-3 pr-2 pb-1 pl-4">
        <h2 className="flex-1 text-xs font-medium text-muted-foreground">
          {picked === null ? "Sols" : `${chosen.length} selected`}
        </h2>
        {/* Deleting sols is refused with making them, so Select goes with New sol. */}
        {!refusal && rows.length > 0 && picked === null && (
          <Button type="button" variant="ghost" size="xs" onClick={() => setPicked(new Set())}>
            Select
          </Button>
        )}
        {picked !== null && (
          <>
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={() =>
                setPicked(new Set(everyPicked ? [] : rows.map((row) => row.dot.id)))
              }
            >
              {everyPicked ? "None" : "All"}
            </Button>
            <Button type="button" variant="ghost" size="xs" onClick={() => setPicked(null)}>
              Done
            </Button>
          </>
        )}
      </div>
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-auto px-2 pb-2" onKeyDown={onKeyDown}>
        {placed.map(({ row, depth }) => {
          const open = row.dot.id === openId;
          const next = nextLine(row, followUps);
          const stepIn = STEP_IN[Math.min(depth, STEP_IN.length - 1)];
          const { made, making } = row.dot;
          if (picked !== null) {
            return (
              <li key={row.dot.id} aria-label={row.dot.name} data-depth={depth} className={stepIn}>
                <label
                  className={cn(
                    "flex cursor-pointer items-center gap-2 rounded-[10px] p-2.5 hover:bg-sidebar-accent/60",
                    picked.has(row.dot.id) && "bg-sidebar-accent hover:bg-sidebar-accent",
                  )}
                >
                  <input
                    type="checkbox"
                    className="size-4 shrink-0 accent-primary"
                    aria-label={`Select ${row.dot.name}`}
                    checked={picked.has(row.dot.id)}
                    onChange={() => toggle(row.dot.id)}
                  />
                  <DotAvatar seed={row.dot.id} />
                  <span className="truncate font-medium">{row.dot.name}</span>
                </label>
              </li>
            );
          }
          return (
            <li key={row.dot.id} aria-label={row.dot.name} data-depth={depth} className={stepIn}>
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
                {made && (
                  <span className="truncate pl-10.5 text-xs text-muted-foreground">
                    made by {made.by_name}
                    {made.by_dot_id === null && " (deleted)"}
                  </span>
                )}
                {making.on && (
                  <span className="pl-10.5 text-xs text-muted-foreground">
                    can make sols · {row.made_dots} of {making.total}
                  </span>
                )}
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
      {picked !== null && (
        <div className="flex flex-col border-t p-2">
          <ConfirmButton
            variant="destructive"
            disabled={chosen.length === 0 || remove.isPending}
            title={chosen.length === 1 ? "Delete 1 sol?" : `Delete ${chosen.length} sols?`}
            description={
              <>
                <span className="block">{chosen.map((row) => row.dot.name).join(", ")}</span>
                <span className="mt-2 block">
                  Each sol, its runs, their transcripts and its memories are deleted. Worktrees
                  and branches its runs left behind stay in your repository. This cannot be
                  undone.
                </span>
                {pickedMakers.length > 0 && (
                  <span className="mt-2 block">
                    {pickedMakers.map((row) => row.dot.name).join(", ")} made sols. Those are
                    not deleted with their maker: only the sols you selected are. A made sol
                    that stays shows its maker as deleted, and a maker's requests that wait for
                    your answer are dropped.
                  </span>
                )}
              </>
            }
            confirmLabel="Delete"
            onConfirm={() =>
              remove.mutate(chosen.map((row) => ({ id: row.dot.id, name: row.dot.name })))
            }
          >
            {remove.isPending
              ? "Deleting…"
              : chosen.length === 1
                ? "Delete 1 sol"
                : `Delete ${chosen.length} sols`}
          </ConfirmButton>
        </div>
      )}
    </nav>
  );
}
