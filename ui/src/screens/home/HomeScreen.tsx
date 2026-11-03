import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Navigate, useNavigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { useScopeRefusal } from "@/lib/access";
import { sortDots } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { notifyError } from "@/lib/notify";
import {
  accountOf,
  inScope,
  useCurrentScope,
  usePickingAccount,
  useSwitchAccount,
} from "@/lib/scope";
import { DotPane } from "@/screens/home/DotPane";
import { DotsColumn } from "@/screens/home/DotsColumn";
import { lastDot } from "@/screens/home/last-dot";
import { SetupFlow } from "@/screens/setup/SetupFlow";

/// No dot yet: the setup view, to describe the first one (setup agent spec §4). Where the role
/// in the account in view refuses making dots, the reason New dot gives instead.
function EmptyHome() {
  const refusal = useScopeRefusal("edit");
  return (
    <div className="m-auto flex w-full max-w-xl flex-col gap-3 p-6">
      {refusal ? <p className="text-muted-foreground">{refusal}</p> : <SetupFlow />}
    </div>
  );
}

/// The home (chat-first home spec): the dots column, and the open dot's conversation. At
/// `/home` it opens the dot last opened, else the first of the column; a dot that is gone sends
/// it back to `/home`.
export function HomeScreen() {
  const { dotId, rootRunId } = useParams();
  const navigate = useNavigate();
  const overview = useQuery({ queryKey: keys.overview, queryFn: commands.dotOverview });
  // Home shows the current account's dots; every dot while the scope loads. A dot whose
  // account was removed is listed under Personal, once the joined accounts say so.
  const scope = useCurrentScope();
  const all = sortDots(overview.data ?? []);
  const orgList = useQuery({
    queryKey: keys.orgs,
    queryFn: commands.orgList,
    enabled: all.some((r) => r.dot.org_id !== null),
  });
  const orgs = orgList.data;
  const rows = all.filter((r) => !scope || inScope(r.dot, scope, orgs));
  // A link to a dot of another account (an Inbox "Open") switches to that account and keeps
  // the route, rather than treating the dot as gone. Only a new route does: once a dot was
  // handled, a change of account with it open (the switcher) never switches back.
  const linked = dotId === undefined ? undefined : all.find((r) => r.dot.id === dotId);
  const outside = linked !== undefined && !rows.includes(linked);
  const [handled, setHandled] = useState<string | undefined>(undefined);
  // The switch a link asked for, until it settles or the scope has reached its account.
  const [pending, setPending] = useState<{ dotId: string; account: string } | undefined>();
  // A switch the person picked in the switcher wins over a link still waiting to switch.
  const picking = usePickingAccount();
  const switchAccount = useSwitchAccount();
  useEffect(() => {
    if (dotId === undefined) {
      // No dot open: a later link, even to the same dot, is a new one.
      if (handled !== undefined) setHandled(undefined);
      if (pending !== undefined) setPending(undefined);
      return;
    }
    if (pending !== undefined && (pending.dotId !== dotId || scope?.account === pending.account)) {
      setPending(undefined);
    }
    if (handled === dotId) return;
    if (picking) {
      setHandled(dotId);
      return;
    }
    if (linked === undefined || scope === undefined) return;
    if (!outside) {
      setHandled(dotId);
      return;
    }
    // Its account is not known while the joined accounts load: wait rather than guess.
    const account = accountOf(linked.dot, orgs);
    if (account === undefined) return;
    setHandled(dotId);
    setPending({ dotId, account });
    // Once the switch settles, the usual filter decides: the dot shows if its account is now
    // current, else the home opens one it lists (a pick may have overtaken this switch).
    switchAccount(account)
      .catch((error: unknown) => {
        notifyError(error);
        navigate("/home", { replace: true });
      })
      .finally(() => setPending(undefined));
  }, [dotId, handled, pending, picking, linked, scope, outside, orgs, switchAccount, navigate]);
  // A failed reload keeps what was loaded, and the open conversation with what is being typed;
  // only a first load that failed has nothing to show but the error.
  const loaded = overview.data !== undefined;
  // The linked dot waits, with nothing in the pane, for its switch: not yet decided, asked for
  // and not yet reached, or behind a switch the person picked.
  const waiting =
    linked !== undefined &&
    (handled !== dotId || pending?.dotId === dotId || picking);
  let pane = null;
  if (!loaded && overview.isError) {
    pane = (
      <div className="flex-1 p-4">
        <ErrorNote error={overview.error} />
      </div>
    );
  } else if (loaded && dotId === undefined) {
    const remembered = lastDot();
    const open = rows.find((row) => row.dot.id === remembered) ?? rows[0];
    pane = open ? <Navigate to={`/home/${open.dot.id}`} replace /> : <EmptyHome />;
  } else if (loaded) {
    const row = rows.find((r) => r.dot.id === dotId);
    if (row) {
      // An older topic, or the conversation again, opens with its own reply target and scroll.
      pane = <DotPane key={`${row.dot.id}/${rootRunId ?? ""}`} row={row} rootRunId={rootRunId} />;
    } else if (waiting && handled !== dotId && orgs === undefined && orgList.isError) {
      // The dot's account can't be told without the joined accounts.
      pane = (
        <div className="flex-1 p-4">
          <ErrorNote error={orgList.error} />
        </div>
      );
    } else if (!waiting) {
      // Gone, or of another account once the link was handled: the home opens a dot it lists.
      pane = <Navigate to="/home" replace />;
    }
  }
  return (
    <div className="relative flex h-full min-h-0">
      <DotsColumn rows={rows} openId={dotId} />
      {/* Its own slot, so that the conversation is not mounted again when it comes and goes. */}
      {loaded && overview.isError ? (
        <div className="absolute top-2 right-4 z-10 max-w-sm">
          <ErrorNote error={overview.error} />
        </div>
      ) : null}
      {pane}
    </div>
  );
}

