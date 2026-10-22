import { useQuery } from "@tanstack/react-query";
import { Link, Navigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { sortDots } from "@/lib/dot-status";
import { keys } from "@/lib/query";
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
  const rows = sortDots(overview.data ?? []);
  let pane = null;
  if (overview.isError) {
    pane = (
      <div className="flex-1 p-4">
        <ErrorNote error={overview.error} />
      </div>
    );
  } else if (overview.isSuccess && dotId === undefined) {
    const remembered = lastDot();
    const open = rows.find((row) => row.dot.id === remembered) ?? rows[0];
    pane = open ? <Navigate to={`/home/${open.dot.id}`} replace /> : <EmptyHome />;
  } else if (overview.isSuccess) {
    const row = rows.find((r) => r.dot.id === dotId);
    pane = row ? (
      <DotPane key={row.dot.id} row={row} rootRunId={rootRunId} />
    ) : (
      <Navigate to="/home" replace />
    );
  }
  return (
    <div className="flex h-full min-h-0">
      <DotsColumn rows={rows} openId={dotId} />
      {pane}
    </div>
  );
}

