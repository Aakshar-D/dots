import { hashKey, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Dot, DotSpec } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { FollowUpsList } from "@/components/FollowUpsList";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { scheduleError, scheduleOf, specErrors, specOf } from "@/lib/dot-rules";
import { useRefusal } from "@/lib/access";
import { errorMessage, notifyError } from "@/lib/notify";
import { orgRemovedText, useDotOrg } from "@/lib/dot-org";
import { invalidateDots, keys, useSwitchingDot } from "@/lib/query";
import { PERSONAL, useCurrentScope } from "@/lib/scope";
import { useDebounced } from "@/lib/use-debounced";
import { DotForm, type Draft } from "@/screens/dot-editor/DotForm";
import { MemoryTab } from "@/screens/dot-editor/MemoryTab";
import { RunsList } from "@/screens/dot-editor/RunsList";
import { WatchersTab } from "@/screens/dot-editor/WatchersTab";

/// The tabs of a dot's editor; `?tab=` opens one (the ⚙ menu's Memory… opens `memory`; the
/// Inbox opens `watchers`).
const TABS = ["settings", "runs", "follow-ups", "memory", "watchers"];

/// The form with its Save button. `onSave` stores the spec, with the org a new dot is made in;
/// its rejection is shown above the form (the core's answer when it refuses what the form let
/// through). `blocked` says why the dot can't be saved: Save is off, and `setEnabled` switches
/// the dot on or off at once instead.
function Editor(props: {
  title: string;
  initial: DotSpec;
  existing: Dot | null;
  saveLabel: string;
  onSave: (spec: DotSpec, org: string | null) => Promise<unknown>;
  blocked?: string;
  setEnabled?: (enabled: boolean) => Promise<unknown>;
  actions?: ReactNode;
  children?: (form: ReactNode) => ReactNode;
}) {
  const [draft, setDraft] = useState<Draft>(() => ({ spec: props.initial, org: null }));
  const [tried, setTried] = useState(false);
  // A new dot starts in the current account (Personal has no org), once the scope is known.
  const scope = useCurrentScope();
  const orgDefaulted = useRef(false);
  const isNew = props.existing === null;
  useEffect(() => {
    if (!isNew || !scope || orgDefaulted.current) return;
    orgDefaulted.current = true;
    if (scope.account !== PERSONAL) setDraft((d) => (d.org === null ? { ...d, org: scope.account } : d));
  }, [isNew, scope]);
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data;
  const save = useMutation({
    mutationFn: (v: { spec: DotSpec; org: string | null }) => props.onSave(v.spec, v.org),
  });
  // A Viewer, or a PC off the member list, can't save in the dot's account (a new dot's is the
  // one chosen in the form).
  const refusal = useRefusal(isNew ? draft.org : (props.existing?.org_id ?? null), "edit");
  const { setEnabled } = props;
  // The stored dot changes at once; the form follows, keeping what else was typed.
  const onEnabled = setEnabled
    ? (enabled: boolean) =>
        void setEnabled(enabled).then(
          () => setDraft((d) => ({ ...d, spec: { ...d.spec, enabled } })),
          notifyError,
        )
    : undefined;

  // The core validates the schedule and computes the preview; the UI parses no schedule. It
  // only holds back one with an emptied field, which the core could not even be sent.
  const schedule = scheduleOf(useDebounced(draft.spec.schedule, 300));
  const sendable = schedule !== null && scheduleError(schedule) === undefined;
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: sendable,
  });
  // The core refuses a new or changed schedule that would never run, and keeps one saved
  // unchanged (a dot whose once has passed can still be edited). So its objection to the
  // stored schedule is a note, and blocks a save only once the schedule changed.
  const stored = props.existing?.schedule ?? null;
  const changed = hashKey([schedule]) !== hashKey([scheduleOf(stored)]);
  const objection = sendable && preview.isError ? errorMessage(preview.error) : undefined;

  const errors = specErrors(draft.spec);
  if (objection && changed) errors.schedule ??= objection;

  function submit() {
    setTried(true);
    if (Object.keys(errors).length > 0) return;
    save.mutate({
      spec: {
        ...draft.spec,
        schedule: scheduleOf(draft.spec.schedule),
        endpoint_url: draft.spec.endpoint_url?.trim() || null,
      },
      org: draft.org,
    });
  }

  const form = (
    <div className="flex flex-col gap-4">
      {save.isError && <ErrorNote error={save.error} />}
      {tried && Object.keys(errors).length > 0 && (
        <p role="alert" className="text-destructive">
          Fix the marked fields, then save again.
        </p>
      )}
      <DotForm
        draft={draft}
        onChange={setDraft}
        // Until the first save attempt only the core's objection to the schedule shows: it
        // comes while the user types, and an empty new form should not start out red.
        errors={tried ? errors : objection && changed ? { schedule: objection } : {}}
        existing={props.existing}
        endpoints={settings?.local_endpoints ?? []}
        schedule={{
          times: sendable && preview.data ? preview.data : [],
          pending: preview.isFetching,
          note: changed ? undefined : objection,
        }}
        onEnabled={onEnabled}
      />
    </div>
  );
  return (
    <Page
      title={props.title}
      actions={
        <>
          {props.actions}
          <Button
            size="sm"
            disabled={save.isPending || props.blocked !== undefined || refusal !== undefined}
            title={props.blocked ?? refusal}
            onClick={submit}
          >
            {props.saveLabel}
          </Button>
        </>
      }
    >
      {props.children ? props.children(form) : form}
    </Page>
  );
}

function NewDot() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const blank = useQuery({ queryKey: keys.newDotSpec, queryFn: commands.newDotSpec });
  if (blank.isPending) return <Page title="New dot" />;
  if (blank.isError) {
    return (
      <Page title="New dot">
        <ErrorNote error={blank.error} />
      </Page>
    );
  }
  return (
    <Editor
      title="New dot"
      initial={blank.data}
      existing={null}
      saveLabel="Create dot"
      onSave={async (spec, org) => {
        // Personal sends no org.
        const dot = await commands.createDot(spec, org);
        invalidateDots(qc);
        qc.setQueryData(keys.dot(dot.id), dot);
        toast.success("Dot created");
        void navigate(`/dots/${dot.id}`);
      }}
    />
  );
}

function ExistingDot({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const asked = params.get("tab");
  const tab = asked !== null && TABS.includes(asked) ? asked : "settings";
  // The form below is filled once, from the first copy of the dot that is here. While the Dots
  // list's switch of this dot is on its way to the core, the core may still answer with the
  // copy from before it, so the dot is not asked for: the switch's answer is the copy (it is
  // written to the cache), and after a switch without an answer the dot is loaded.
  const switching = useSwitchingDot(dotId);
  const dot = useQuery({
    queryKey: keys.dot(dotId),
    queryFn: () => commands.getDot(dotId),
    enabled: !switching,
  });
  const org = useDotOrg(dot.data?.org_id ?? null);
  const blocked = org.removed && dot.data?.org_id ? orgRemovedText(dot.data.org_id) : undefined;
  const runNow = useMutation({
    mutationFn: () => commands.runNow(dotId),
    onSuccess: (run) => void navigate(`/runs/${run.id}`),
    onError: notifyError,
  });
  const remove = useMutation({
    mutationFn: () => commands.deleteDot(dotId),
    onSuccess: () => {
      // Removed first: the deleted dot must not be loaded again.
      qc.removeQueries({ queryKey: keys.dot(dotId) });
      invalidateDots(qc);
      toast.success("Dot deleted");
      void navigate("/home");
    },
    onError: notifyError,
  });
  // A dot that was loaded stays on the page while it is loaded again, and when that fails: the
  // form below holds what the user typed, and it is filled only once, when it mounts. So it is
  // not mounted from a cached copy that is being loaded again because it was marked stale (a
  // rule saved from the Inbox changed the dot): saving that copy would undo the change. During
  // a switch the query is off, and the switch's answer is the copy.
  const refreshing = dot.isFetching && !dot.isFetchedAfterMount;
  if (dot.data === undefined || refreshing) {
    return <Page title="Dot">{dot.isError && <ErrorNote error={dot.error} />}</Page>;
  }
  return (
    <Editor
      title={dot.data.name}
      initial={specOf(dot.data)}
      existing={dot.data}
      saveLabel="Save"
      blocked={blocked}
      setEnabled={
        blocked === undefined
          ? undefined
          : async (enabled) => {
              const saved = await commands.setDotEnabled(dotId, enabled);
              qc.setQueryData(keys.dot(dotId), saved);
              invalidateDots(qc);
            }
      }
      onSave={async (spec) => {
        const saved = await commands.updateDot(dotId, spec);
        invalidateDots(qc);
        qc.setQueryData(keys.dot(dotId), saved);
        toast.success("Saved");
      }}
      actions={
        <>
          <ConfirmButton
            title={`Delete ${dot.data.name}?`}
            description="The dot, its runs, their transcripts and its memories are deleted. Worktrees and branches its runs left behind stay in your repository."
            confirmLabel="Delete the dot"
            onConfirm={() => remove.mutate()}
            disabled={remove.isPending}
          >
            Delete
          </ConfirmButton>
          <Button
            size="sm"
            variant="outline"
            disabled={runNow.isPending || blocked !== undefined}
            title={blocked}
            onClick={() => runNow.mutate()}
          >
            <Play aria-hidden /> Run now
          </Button>
        </>
      }
    >
      {(form) => (
        <>
          {dot.isError && <ErrorNote error={dot.error} />}
          <Tabs
            value={tab}
            onValueChange={(next) =>
              setParams(next === "settings" ? {} : { tab: next }, { replace: true })
            }
          >
            {/* Five tabs outgrow a narrow window: the strip scrolls, not the page. */}
            <TabsList className="max-w-full justify-start overflow-x-auto overflow-y-hidden">
              <TabsTrigger value="settings">Settings</TabsTrigger>
              <TabsTrigger value="runs">Runs</TabsTrigger>
              <TabsTrigger value="follow-ups">Follow-ups</TabsTrigger>
              <TabsTrigger value="memory">Memory</TabsTrigger>
              <TabsTrigger value="watchers">Watchers</TabsTrigger>
            </TabsList>
            <TabsContent value="settings">{form}</TabsContent>
            <TabsContent value="runs">
              <RunsList dotId={dotId} />
            </TabsContent>
            <TabsContent value="follow-ups">
              <FollowUpsList dotId={dotId} />
            </TabsContent>
            <TabsContent value="memory">
              <MemoryTab dotId={dotId} />
            </TabsContent>
            {/* Its rows' switches and buttons are the tab stops: a focusable panel around them
                would crowd their touch targets (WCAG 2.5.8). */}
            <TabsContent value="watchers" tabIndex={-1}>
              <WatchersTab dotId={dotId} />
            </TabsContent>
          </Tabs>
        </>
      )}
    </Editor>
  );
}

export function DotEditorScreen() {
  const { dotId } = useParams();
  // The key gives each dot its own draft.
  return dotId ? <ExistingDot key={dotId} dotId={dotId} /> : <NewDot />;
}
