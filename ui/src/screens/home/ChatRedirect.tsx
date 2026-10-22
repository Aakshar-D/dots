import { useQuery } from "@tanstack/react-query";
import { Navigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import { conversation, topicsOf } from "@/lib/home-topics";
import { keys } from "@/lib/query";
import { LOADED_RUNS } from "@/screens/home/use-conversation";

/// An address of the old Chat screen: `/chat/:dotId` opens the dot's conversation, and
/// `/chat/:dotId/:runId` the topic of that run, in the conversation when it is there, else
/// alone. A run that cannot be found opens the dot's conversation.
export function ChatRedirect() {
  const { dotId, runId } = useParams();
  const thread = useQuery({
    queryKey: keys.thread(runId ?? ""),
    queryFn: () => commands.runThread(runId!),
    enabled: runId !== undefined,
  });
  const list = useQuery({
    queryKey: [...keys.runs(dotId!), "home"],
    queryFn: () => commands.listRuns(dotId!, LOADED_RUNS),
    enabled: runId !== undefined,
  });
  const home = `/home/${dotId}`;
  if (runId === undefined || thread.isError || list.isError) return <Navigate to={home} replace />;
  if (thread.isPending || list.isPending) return null;
  const root = thread.data[0]?.root_run_id ?? runId;
  const shown = conversation(topicsOf(list.data)).some((topic) => topic.rootId === root);
  return <Navigate to={shown ? home : `${home}/${root}`} replace />;
}
