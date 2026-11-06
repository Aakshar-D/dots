import { QueryClient, useIsMutating, type QueryKey } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type {
  RunEventRecord,
  Schedule,
  Status,
  WorkflowRunFilter,
  WorkflowSpec,
} from "@/api/types";

/// Data stays fresh until a runtime event or a mutation marks it stale (lib/runtime-events.ts):
/// the backend tells the UI when something changed, so nothing polls.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      // "always": every query and command is local IPC, so none waits for the network.
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnWindowFocus: false,
        networkMode: "always",
      },
      mutations: { retry: false, networkMode: "always" },
    },
  });
}

export const keys = {
  startupError: ["startup-error"] as const,
  status: ["status"] as const,
  managedPolicy: ["managed-policy"] as const,
  update: ["update"] as const,
  settings: ["settings"] as const,
  /// Settings › Share usage counts, and the next report under it.
  usageCounts: ["usage-counts"] as const,
  usageCountsPreview: ["usage-counts", "preview"] as const,
  chatGptAccount: ["chatgpt-account"] as const,
  overview: ["overview"] as const,
  /// The sum of the dots' unread counts (the Home link). Under the overview's key, so that
  /// whatever marks the overview stale marks it too.
  unreadTotal: ["overview", "unread"] as const,
  inbox: ["inbox"] as const,
  library: ["library"] as const,
  /// The Connectors screen, under the Library's key: a Library or vault change marks it stale.
  connectors: ["library", "connectors"] as const,
  /// Whether this PC has the program a connector needs (Node.js).
  connectorRuntime: (id: string) => ["connector-runtime", id] as const,
  installedPlugins: ["installed-plugins"] as const,
  /// The Claude Code agents on this PC, for the import dialog.
  claudeAgents: ["claude-agents"] as const,
  environmentVariables: (names: string[]) => ["environment-variables", names] as const,
  env: ["env"] as const,
  envImport: ["env-import"] as const,
  envBackups: ["env-backups"] as const,
  /// This install's Dots ID (Settings → Profile).
  identity: ["identity"] as const,
  /// The account and workspace the person works in now.
  scope: ["scope"] as const,
  /// The workspaces of an account (`personal` or an org id) this PC may see.
  workspaces: (account: string) => ["workspaces", account] as const,
  /// The API keys of an account (`personal` or an org id).
  apiKeys: (scope: string) => ["apiKeys", scope] as const,
  /// The orgs joined on this PC (Settings → Accounts, and the org names other screens
  /// show). What is read per org belongs under this prefix, so an org event reaches it too.
  orgs: ["orgs"] as const,
  /// An org's entries for the Library's Env tab, under the orgs' key.
  orgEntries: (orgId: string) => ["orgs", orgId, "entries"] as const,
  /// An org's member list for the Members tab, under the orgs' key.
  orgMembers: (orgId: string) => ["orgs", orgId, "members"] as const,
  /// This PC's age public key for a SOPS org: made once, so no org event asks for it again.
  orgSopsKey: (orgId: string) => ["org-sops-key", orgId] as const,
  dots: ["dots"] as const,
  dot: (id: string) => ["dot", id] as const,
  run: (id: string) => ["run", id] as const,
  /// The status of a dot's grants, under the dot's key: a change to the dot marks it stale, and
  /// so does an org event (lib/runtime-events.ts).
  dotVariables: (id: string) => ["dot", id, "variables"] as const,
  /// What the person may not do with a dot, under the dot's key: a move or an org event (its
  /// member list) marks it stale.
  dotAccess: (id: string) => ["dot", id, "access"] as const,
  newDotSpec: ["new-dot-spec"] as const,
  /// The models of an engine, or of a local endpoint (model switcher spec section 2.1).
  models: (engine: string, endpoint: string | null) => ["models", engine, endpoint] as const,
  schedulePreview: (schedule: Schedule | null) => ["schedule-preview", schedule] as const,
  webhookUrl: (dotId: string) => ["webhook-url", dotId] as const,
  runs: (dotId: string) => ["runs", dotId] as const,
  /// The Files panel of a conversation (its topics' roots), under the runs' key: a run that
  /// changes may have attached or written a file.
  conversationFiles: (dotId: string, roots: string[]) =>
    ["runs", dotId, "files", ...roots] as const,
  /// Today's sent, delivered and not delivered counts for a dot, under the runs' key so a
  /// run_updated refreshes it.
  deliveryToday: (dotId: string) => ["runs", dotId, "delivery-today"] as const,
  thread: (runId: string) => ["thread", runId] as const,
  /// A dot's pending scheduled messages (Send later).
  scheduledMessages: (dotId: string) => ["scheduled-messages", dotId] as const,
  events: (runId: string) => ["events", runId] as const,
  approvals: (runId: string) => ["approvals", runId] as const,
  approvalRules: (approvalId: string) => ["approval-rules", approvalId] as const,
  approvalPrivate: (approvalId: string) => ["approval-private", approvalId] as const,
  diff: (runId: string) => ["diff", runId] as const,
  messages: (dotId: string | null) => ["messages", dotId] as const,
  followUps: (dotId: string | null) => ["follow-ups", dotId] as const,
  /// The slots a dot skipped because it was busy.
  scheduleSkips: (dotId: string) => ["schedule-skips", dotId] as const,
  /// The rows of a dot's topics: names and archive marks.
  topics: (dotId: string) => ["topics", dotId] as const,
  memories: (dotId: string) => ["memories", dotId] as const,
  /// The goals of one workspace of an account (Intents).
  intents: (scope: string, workspace: string) => ["intents", scope, workspace] as const,
  /// The instructions a dot proposed (conversational chat spec section 4).
  instructionProposals: (dotId: string) => ["instruction-proposals", dotId] as const,
  watchers: (dotId: string) => ["watchers", dotId] as const,
  /// The apps dots proposed, of one dot or of all (null).
  appProposals: (dotId: string | null) => ["app-proposals", dotId] as const,
  /// The installed apps (the Apps screen), and below them the data removed apps kept.
  apps: ["apps"] as const,
  appsKept: ["apps", "kept"] as const,
  /// The apps that come with Dots and are not installed ("Install Projects").
  appsBundled: ["apps", "bundled"] as const,
  limits: ["limits"] as const,
  updateStatus: ["update-status"] as const,
  publisher: ["publisher"] as const,
  releasesPage: ["releases-page"] as const,
  releaseNotes: (version: string) => ["release-notes", version] as const,
  usage: (period: string) => ["usage", period] as const,
  /// Every team, and below it each team with its members and projects.
  teams: ["teams"] as const,
  team: (id: string) => ["teams", id] as const,
  /// A team's workflows, and one workflow. Both say what runs in flight and the last run, which
  /// change with a workflow run as well.
  workflows: (teamId: string) => ["workflows", teamId] as const,
  workflow: (id: string) => ["workflow", id] as const,
  /// The builder's live check of its draft, which reads the team's members.
  workflowProblem: (teamId: string, spec: WorkflowSpec) =>
    ["workflow-problem", teamId, spec] as const,
  workflowWebhookUrl: (id: string) => ["workflow-webhook-url", id] as const,
  /// Lists of workflow runs: by filter (the team page), and the runs chat runs started (a
  /// conversation's cards). Any workflow run event marks them all stale.
  workflowRuns: (filter: Partial<WorkflowRunFilter>) =>
    ["workflow-runs", "list", filter] as const,
  workflowRunsForOrigins: (runIds: string[]) => ["workflow-runs", "origins", runIds] as const,
  /// A workflow run, and below it what its check shows and a branch's diff: its event marks them
  /// stale.
  workflowRun: (id: string) => ["workflow-run", id] as const,
  workflowCheck: (id: string) => ["workflow-run", id, "check"] as const,
  workflowBranchDiff: (id: string, branch: string, commit: string) =>
    ["workflow-run", id, "diff", branch, commit] as const,
  /// The dot runs of a workflow run's steps (the run page's links). The workflow run's event and
  /// the event of a run that works for a step mark them stale.
  workflowDotRuns: (id: string) => ["workflow-dot-runs", id] as const,
};

type DotSwitch = { id: string; enabled: boolean };

/// The mutation that switches a dot on or off (the Dots list). It has a key so that the dot's
/// editor can see one in flight (`useSwitchingDot`).
export const setDotEnabledMutation = {
  mutationKey: ["set-dot-enabled"] as const,
  mutationFn: (v: DotSwitch) => commands.setDotEnabled(v.id, v.enabled),
};

/// Whether a switch of this dot is on its way to the core. Until it has answered, a copy of the
/// dot loaded from the core may be the one from before the switch.
export function useSwitchingDot(dotId: string): boolean {
  return (
    useIsMutating({
      mutationKey: setDotEnabledMutation.mutationKey,
      predicate: (m) => (m.state.variables as DotSwitch | undefined)?.id === dotId,
    }) > 0
  );
}

/// Marks the queries under `queryKey` stale, every query without one, and loads those on screen
/// again: what follows a change in the backend.
///
/// `invalidateQueries` alone loses the mark of a query whose fetch is in flight and is not
/// restarted, which is a first load, and a refetch of a query no screen shows. That fetch may
/// have read the backend before the change, and its answer clears the mark. So each query that
/// is fetching now is marked again when that fetch has settled. A fetch running by then started
/// after the change, so it is joined, not restarted: several changes share one question. This
/// holds for a fetch that was restarted too: its promise, taken before, settles at once.
export function invalidate(qc: QueryClient, queryKey?: QueryKey): void {
  const fetching = qc
    .getQueryCache()
    .findAll({ queryKey, fetchStatus: "fetching" })
    .map((query) => ({ key: query.queryKey, settled: query.promise }));
  void qc.invalidateQueries({ queryKey });
  for (const { key, settled } of fetching) {
    const again = () =>
      void qc.invalidateQueries({ queryKey: key, exact: true }, { cancelRefetch: false });
    void settled?.then(again, again);
  }
}

/// Takes a status the backend sent: the tray's event, or the answer of a command that changed
/// it. A question about the status that is on its way may have been answered before that
/// change, and its answer would replace this status. So that question is dropped for one asked
/// now: marking a query that has data and is on screen stale starts its fetch again.
export function takeStatus(qc: QueryClient, status: Status): void {
  const asking = qc.getQueryState(keys.status)?.fetchStatus === "fetching";
  qc.setQueryData(keys.status, status);
  if (asking) invalidate(qc, keys.status);
}

/// Marks stale everything a change to a dot can show up in: the dot lists, every loaded dot,
/// the Inbox (a deleted dot takes its approvals and runs with it), every list of runs, and the
/// Library's "used by": the Personal variables, every org's entries and the servers. No runtime
/// event announces a dot change, so each command that changes a dot calls this.
export function invalidateDots(qc: QueryClient): void {
  const orgEntries = qc
    .getQueryCache()
    .findAll({ queryKey: keys.orgs })
    .map((query) => query.queryKey)
    .filter((key) => key.length === 3 && key[2] === "entries");
  const stale: QueryKey[] = [
    keys.dots,
    ["dot"],
    keys.overview,
    keys.inbox,
    ["runs"],
    keys.env,
    keys.library,
    ...orgEntries,
  ];
  for (const queryKey of stale) invalidate(qc, queryKey);
}

/// `have` plus the events of `more` it does not hold yet, ordered by `seq`.
export function mergeEvents(have: RunEventRecord[], more: RunEventRecord[]): RunEventRecord[] {
  const seen = new Set(have.map((e) => e.seq));
  const fresh = more.filter((e) => !seen.has(e.seq));
  if (fresh.length === 0) return have;
  return [...have, ...fresh].sort((a, b) => a.seq - b.seq);
}

/// A run's events. A refetch asks only for what is missing: the events after the last one
/// already loaded. The answer is merged into the list as it is when the answer arrives, so an
/// event a runtime event appended in the meantime stays.
export function eventsQuery(qc: QueryClient, runId: string) {
  const queryKey = keys.events(runId);
  return {
    queryKey,
    queryFn: async () => {
      const have = qc.getQueryData<RunEventRecord[]>(queryKey) ?? [];
      const more = await commands.listRunEvents(runId, have.at(-1)?.seq ?? 0);
      return mergeEvents(qc.getQueryData<RunEventRecord[]>(queryKey) ?? have, more);
    },
  };
}
