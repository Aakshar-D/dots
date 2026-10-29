import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { PERSONAL } from "@/lib/scope";

import { WorkspaceList } from "./AccountWorkspaces";

/// Personal's workspaces (workspaces spec): kept on this PC alone, so Add, Rename and Delete
/// are plain commands, not signed edits. Deleting one moves its dots to Default.
export function PersonalWorkspaces() {
  const qc = useQueryClient();
  const spaces = useQuery({
    queryKey: keys.workspaces(PERSONAL),
    queryFn: () => commands.workspaceList(PERSONAL),
  });
  const run = useMutation({
    mutationFn: (task: () => Promise<unknown>) => task(),
    // The dots moved by a delete are listed with their workspace, so they ask again too.
    onSuccess: () => {
      invalidate(qc, ["workspaces"]);
      invalidate(qc, keys.dots);
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
      onAdd={(name) => run.mutate(() => commands.personalWorkspaceAdd(name))}
      onRename={(id, name) => run.mutate(() => commands.personalWorkspaceRename(id, name))}
      onDelete={(id) => run.mutate(() => commands.personalWorkspaceDelete(id))}
    />
  );
}
