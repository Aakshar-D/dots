import { useMutation, useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Check } from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { OrgSummary } from "@/api/types";
import { accessOf, accessRole, roleLabel } from "@/lib/access";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";
import {
  accountPickKey,
  PERSONAL,
  useCurrentScope,
  useSwitchAccount,
  useSwitchWorkspace,
  useWorkspaces,
  waitingByAccount,
} from "@/lib/scope";
import { ORG_TONE } from "@/screens/Accounts";

/// Up to two initials of `name`; "?" when it is blank.
function initials(name: string | undefined): string {
  const words = (name ?? "").trim().split(/\s+/).filter(Boolean);
  return (
    words
      .slice(0, 2)
      .map((w) => Array.from(w)[0].toUpperCase())
      .join("") || "?"
  );
}

type Account = { id: string; name: string; org: OrgSummary | null };

const ITEM =
  "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent focus-visible:bg-accent focus-visible:outline-none";

/// The first button of the rail: who the person is, and the account they work in. Opens a menu
/// of Personal and the joined accounts, with what waits in each.
export function AccountSwitcher() {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const switchAccount = useSwitchAccount();
  const pick = useMutation({ mutationKey: accountPickKey, mutationFn: switchAccount });
  const switchWorkspace = useSwitchWorkspace();
  const scope = useCurrentScope();
  const identity = useQuery({ queryKey: keys.identity, queryFn: commands.identityGet }).data;
  // The joined accounts are read when the menu opens, or to name an org already chosen; what
  // waits in each only when the menu opens, so the rail adds no calls of its own.
  const inOrg = scope !== undefined && scope.account !== PERSONAL;
  const joined = useQuery({
    queryKey: keys.orgs,
    queryFn: commands.orgList,
    enabled: open || inOrg,
  }).data;
  const orgs = joined ?? [];
  const overview = useQuery({
    queryKey: keys.overview,
    queryFn: commands.dotOverview,
    enabled: open,
  }).data;
  const waiting = waitingByAccount(overview ?? [], joined);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        trigger.current?.focus();
      }
    };
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open]);

  // Focus the first item when the menu opens, so the keyboard can take it from there.
  useEffect(() => {
    if (open) root.current?.querySelector<HTMLElement>('[role^="menuitem"]')?.focus();
  }, [open]);

  // The current account's workspaces; the list shows only when it has more than Default, and
  // an empty answer leaves the account alone.
  const workspaces = useWorkspaces(scope?.account ?? PERSONAL, open && scope !== undefined) ?? [];

  const accounts: Account[] = [
    { id: PERSONAL, name: "Personal", org: null },
    ...orgs.map((org) => ({ id: org.id, name: org.name, org })),
  ];
  const currentId = scope?.account ?? PERSONAL;
  const currentName = accounts.find((a) => a.id === currentId)?.name ?? currentId;

  function choose(id: string) {
    setOpen(false);
    trigger.current?.focus();
    // On the home, a new account opens its own first dot rather than keeping one it does not
    // list; the scope is loaded again first, so the home picks from the new account's dots.
    const onHome = pathname === "/home" || pathname.startsWith("/home/");
    const changed = id !== currentId;
    pick.mutateAsync(id).then(() => {
      if (onHome && changed) navigate("/home", { replace: true });
    }, notifyError);
  }
  function chooseWorkspace(id: string) {
    setOpen(false);
    trigger.current?.focus();
    if (id === scope?.workspace) return;
    const onHome = pathname === "/home" || pathname.startsWith("/home/");
    switchWorkspace(currentId, id).then(() => {
      if (onHome) navigate("/home", { replace: true });
    }, notifyError);
  }
  function manage() {
    setOpen(false);
    navigate("/settings#accounts");
  }
  function onMenuKey(e: React.KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...(root.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    const next = (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  }

  return (
    <div ref={root} className="relative">
      <button
        ref={trigger}
        type="button"
        aria-label={`Account: ${currentName}`}
        title={`Account: ${currentName}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="relative flex size-9 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground"
      >
        {initials(identity?.name)}
        <span
          aria-hidden
          className="absolute -right-1 -bottom-1 flex size-4 items-center justify-center rounded-full border bg-background text-[9px] text-foreground"
        >
          {Array.from(currentName)[0]?.toUpperCase()}
        </span>
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Accounts"
          onKeyDown={onMenuKey}
          className="absolute top-0 left-full z-50 ml-2 flex w-64 flex-col gap-0.5 rounded-lg border bg-popover p-1 text-popover-foreground shadow-md"
        >
          {accounts.map((a) => {
            const count = waiting.get(a.id) ?? 0;
            const role = accessRole(accessOf(a.org ?? undefined, a.org ? a.id : null));
            const status = a.org && a.org.status !== "connected" ? a.org : null;
            return (
              <Fragment key={a.id}>
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={a.id === currentId}
                  className={ITEM}
                  onClick={() => choose(a.id)}
                >
                  <Check
                    className={cn("size-4 shrink-0", a.id !== currentId && "invisible")}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate">{a.name}</span>
                  {role && <span className="text-xs text-muted-foreground">{roleLabel(role)}</span>}
                  {status && (
                    <span className={cn("text-xs first-letter:uppercase", ORG_TONE[status.status])}>
                      {status.status_text}
                    </span>
                  )}
                  {count > 0 && (
                    <span
                      data-tone="needs"
                      className="rounded-full bg-needs px-1.5 text-[10px] text-needs-foreground"
                    >
                      {count}
                    </span>
                  )}
                </button>
                {a.id === currentId && workspaces.length > 1 && (
                  <div role="group" aria-label="Workspaces" className="flex flex-col gap-0.5 pl-4">
                    {workspaces.map((w) => (
                      <button
                        key={w.id}
                        type="button"
                        role="menuitemradio"
                        aria-checked={w.id === scope?.workspace}
                        className={ITEM}
                        onClick={() => chooseWorkspace(w.id)}
                      >
                        <Check
                          className={cn("size-4 shrink-0", w.id !== scope?.workspace && "invisible")}
                          aria-hidden
                        />
                        <span className="min-w-0 flex-1 truncate">{w.name}</span>
                      </button>
                    ))}
                  </div>
                )}
              </Fragment>
            );
          })}
          <div role="separator" className="my-1 h-px bg-border" />
          <button type="button" role="menuitem" className={ITEM} onClick={manage}>
            Join an account
          </button>
          <button type="button" role="menuitem" className={ITEM} onClick={manage}>
            Manage accounts
          </button>
        </div>
      )}
    </div>
  );
}
