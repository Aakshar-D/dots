import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { StatusBadge } from "@/components/StatusBadge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { duration, localDateTime } from "@/lib/format";
import { keys } from "@/lib/query";

/// The dot's latest runs, newest first.
export function RunsList({ dotId }: { dotId: string }) {
  const runs = useQuery({
    queryKey: keys.runs(dotId),
    queryFn: () => commands.listRuns(dotId, 50),
  });
  if (runs.isPending) return null;
  if (runs.isError) return <ErrorNote error={runs.error} />;
  if (runs.data.length === 0) {
    return <p className="text-muted-foreground">This dot has not run yet.</p>;
  }
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Queued</TableHead>
          <TableHead>Trigger</TableHead>
          <TableHead>Status</TableHead>
          <TableHead>Took</TableHead>
          <TableHead>Result</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {runs.data.map((run) => (
          <TableRow key={run.id}>
            <TableCell>
              <Link to={`/runs/${run.id}`} className="hover:underline">
                {localDateTime(run.queued_at)}
              </Link>
            </TableCell>
            <TableCell>{run.trigger}</TableCell>
            <TableCell>
              <StatusBadge status={run.status} />
            </TableCell>
            <TableCell>{duration(run.started_at, run.ended_at)}</TableCell>
            <TableCell className="max-w-md truncate text-muted-foreground">
              {run.error ?? run.summary ?? ""}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
