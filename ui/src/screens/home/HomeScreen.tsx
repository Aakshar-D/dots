import { useQuery } from "@tanstack/react-query";
import { Link, Navigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { sortDots } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { inScope, useCurrentScope } from "@/lib/scope";
import { DotPane } from "@/screens/home/DotPane";
import { DotsColumn } from "@/screens/home/DotsColumn";
import { lastDot } from "@/screens/home/last-dot";

/// No dot yet: what a dot is, and the way to make one.
function EmptyHome() {
  return (
    <div className="m-auto flex max-w-sm flex-col items-center gap-3 p-6 text-center">
      <p className="text-muted-foreground">
        A dot is an agent with instructions, a folder to work in, and a schedule or a webhook
        that starts it. You talk to it here.
      </p>
      <Button asChild>
        <Link to="/dots/new">Create your first dot</Link>
      </Button>
    </div>
  );
}

/// The home (chat-first home spec): the dots column, and the open dot's conversation. At
/// `/home` it opens the dot last opened, else the first of the column; a dot that is gone sends
/// it back to `/home`.
export function HomeScreen() {
  const { dotId, rootRunId } = useParams();
  const overview = useQuery({ queryKey: keys.overview, queryFn: commands.dotOverview });
  // Home shows the current account's dots; every dot while the scope loads.
  const scope = useCurrentScope();
  const rows = sortDots(overview.data ?? []).filter((r) => !scope || inScope(r.dot, scope));
  // A failed reload keeps what was loaded, and the open conversation with what is being typed;
  // only a first load that failed has nothing to show but the error.
  const loaded = overview.data !== undefined;
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
    pane = row ? (
      // An older topic, or the conversation again, opens with its own reply target and scroll.
      <DotPane key={`${row.dot.id}/${rootRunId ?? ""}`} row={row} rootRunId={rootRunId} />
    ) : (
      <Navigate to="/home" replace />
    );
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

