import { useQueries, useQuery } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import {
  conversation,
  olderTopics,
  topicOf,
  topicsOf,
  type Topic,
} from "@/lib/home-topics";
import { keys } from "@/lib/query";

/// How many of the dot's newest runs the home loads; older ones are reached through History.
export const LOADED_RUNS = 200;

/// What a dot's side of the home shows (chat-first home spec, section 4.3): the dot's topics
/// from its newest runs; the topics shown, each loaded whole with `run_thread` (the
/// conversation, or the older topic `rootRunId` alone); the topics History offers; and the
/// dot's notes and questions, oldest first.
export function useConversation(dotId: string, rootRunId: string | undefined) {
  const list = useQuery({
    // Its own key under "runs": the dot editor lists the same dot's runs with another limit.
    queryKey: [...keys.runs(dotId), "home"],
    queryFn: () => commands.listRuns(dotId, LOADED_RUNS),
  });
  const topics = topicsOf(list.data ?? []);
  const current = conversation(topics);
  const roots = rootRunId !== undefined ? [rootRunId] : current.map((topic) => topic.rootId);
  const threads = useQueries({
    queries: roots.map((root) => ({
      queryKey: keys.thread(root),
      queryFn: () => commands.runThread(root),
    })),
  });
  // A thread on its way shows as far as the list has it.
  const shown: Topic[] = roots.flatMap((root, i) => {
    const runs = threads[i]?.data ?? topics.find((topic) => topic.rootId === root)?.runs ?? [];
    return runs.length === 0 ? [] : [topicOf(runs)];
  });
  const messages = useQuery({
    queryKey: keys.messages(dotId),
    queryFn: () => commands.messagesList(dotId),
  });
  const checkIns = [...(messages.data ?? [])].sort((a, b) =>
    a.created_at.localeCompare(b.created_at),
  );
  return {
    list,
    threads,
    shown,
    older: olderTopics(topics, current),
    checkIns,
    /// Nothing to show yet: the list, or the older topic, is still on its way.
    pending: list.isPending || (rootRunId !== undefined && threads[0]?.isPending === true),
  };
}
