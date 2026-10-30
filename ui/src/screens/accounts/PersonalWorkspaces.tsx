import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { PERSONAL, useCurrentScope } from "@/lib/scope";

import { WorkspaceList } from "./AccountWorkspaces";

/// Personal's workspaces (workspaces spec): kept on this PC alone, so Add, Rename and Delete
/// are plain commands, not signed edits. Deleting one moves its dots to Default.
export function PersonalWorkspaces() {
  const qc = useQueryClient();
  const spaces = useQuery({
    queryKey: keys.workspaces(PERSONAL),
    queryFn: () => commands.workspaceList(PERSONAL),
  });
  const scope = useCurrentScope();
  const inView = (id: string) => scope?.account === PERSONAL && scope.workspace === id;
  const run = useMutation({
    mutationFn: (v: { task: () => Promise<unknown>; touchesScope: boolean }) => v.task(),
    // The dots moved by a delete are listed with their workspace, so they ask again too. A
    // delete, or a rename of the workspace in view, changes what the scope shows.
    onSuccess: (_answer, { touchesScope }) => {
      invalidate(qc, ["workspaces"]);
      if (touchesScope) {
        invalidate(qc, keys.scope);
        invalidateDots(qc);
      } else {
        invalidate(qc, keys.dots);
      }
    },
    onError: notifyError,
  });
  if (spaces.isError) return <ErrorNote error={spaces.error} />;
  if (!spaces.data) return <p className="text-xs text-muted-foreground">Loading the workspaces…</p>;
  return (
    <WorkspaceList
      label="Personal"
      items={spaces.data}
      lock={run.isPending ? "Waiting for the last change to finish." : undefined}
      onAdd={(name) =>
        run.mutate({ task: () => commands.personalWorkspaceAdd(name), touchesScope: false })
      }
      onRename={(id, name) =>
        run.mutate({
          task: () => commands.personalWorkspaceRename(id, name),
          touchesScope: inView(id),
        })
      }
      onDelete={(id) =>
        run.mutate({ task: () => commands.personalWorkspaceDelete(id), touchesScope: true })
      }
    />
  );
}
