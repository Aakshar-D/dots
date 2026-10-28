import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { AppFile, AppProposal, ProposalDecision, Run } from "@/api/types";
import { AppFrame } from "@/components/AppFrame";
import { ConfirmButton } from "@/components/ConfirmButton";
import { DiffView } from "@/components/DiffView";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { previewSrc } from "@/lib/app-bridge";
import { formatBytes, relativeTime } from "@/lib/format";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { plural } from "@/screens/library/parts";

/// The most characters "Ask for changes" takes (`propose::MAX_NOTE`).
const MAX_ASK = 2000;

/// The cards of the apps `runs` sent for review that still wait, under those runs: a chat
/// thread's runs, or the first run of a run topic (a scheduled or manual run). When the list
/// cannot be read it shows nothing: the conversation shows that error once (`Timeline`).
export function AppReviews({ runs, dotName }: { runs: Run[]; dotName: string }) {
  const dotId = runs[0]?.dot_id ?? null;
  const proposals = useQuery({
    queryKey: keys.appProposals(dotId),
    queryFn: () => commands.appProposalsList(dotId),
    enabled: dotId !== null,
  });
  if (proposals.isError) return null;
  const waiting = (proposals.data ?? []).filter(
    (p) => p.status === "pending" && runs.some((run) => run.id === p.run_id),
  );
  return waiting.map((proposal) => (
    <AppReviewCard key={proposal.id} proposal={proposal} dotName={dotName} />
  ));
}

/// A dot's app waiting for the person in its conversation (app library spec section 6.2): the
/// app running in its sandbox on sample data (Preview), every file or the diff of a change
/// (Code), what it is and reaches (Summary), and the decision.
export function AppReviewCard({ proposal, dotName }: { proposal: AppProposal; dotName: string }) {
  const { summary } = proposal;
  const update = summary.update_from !== null;
  const heading = update
    ? `Change to ${summary.title}: version ${summary.update_from} and on`
    : `New app: ${summary.title}`;
  return (
    <article
      aria-label={`Review of ${summary.title}`}
      className="flex flex-col overflow-hidden rounded-lg border border-needs-border bg-card"
    >
      <header className="border-b p-3">
        <p className="font-semibold">
          {heading} <span className="font-normal text-muted-foreground">· from ● {dotName}</span>
        </p>
        <p className="text-xs whitespace-pre-wrap text-muted-foreground">{proposal.note}</p>
      </header>
      <Tabs defaultValue="preview" className="gap-0">
        <TabsList className="mx-3 mt-2">
          <TabsTrigger value="preview">Preview</TabsTrigger>
          <TabsTrigger value="code">Code</TabsTrigger>
          <TabsTrigger value="summary">Summary</TabsTrigger>
        </TabsList>
        {/* Kept while another tab shows, so the preview's changes last until the card closes. */}
        <TabsContent value="preview" forceMount className="p-3 data-[state=inactive]:hidden">
          <Preview proposal={proposal} />
        </TabsContent>
        <TabsContent value="code" className="p-3">
          <Code proposal={proposal} />
        </TabsContent>
        <TabsContent value="summary" className="p-3">
          <Summary proposal={proposal} />
        </TabsContent>
      </Tabs>
      <Decide proposal={proposal} />
    </article>
  );
}

/// The proposal's screen on its own throwaway database, opened with the card and closed with it.
/// The preview's data changes only through the page's own calls, so a successful call of a tool
/// that writes is the change it is told of.
function Preview({ proposal }: { proposal: AppProposal }) {
  const [opened, setOpened] = useState<{ id: string } | { error: unknown } | null>(null);
  // The calls of tools that write which went through: the page hears of each (`dots.onChange`).
  const [changes, setChanges] = useState(0);
  useEffect(() => {
    let gone = false;
    let id: string | null = null;
    commands.appPreviewOpen(proposal.id).then(
      (open) => {
        if (gone) void commands.appPreviewClose(open).catch(() => {});
        else {
          id = open;
          setOpened({ id: open });
        }
      },
      (error: unknown) => {
        if (!gone) setOpened({ error });
      },
    );
    return () => {
      gone = true;
      if (id !== null) void commands.appPreviewClose(id).catch(() => {});
    };
  }, [proposal.id]);
  if (opened === null) return <p className="text-muted-foreground">Starting the preview…</p>;
  if ("error" in opened) return <ErrorNote error={opened.error} />;
  const preview = opened.id;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex h-[420px] flex-col overflow-hidden rounded-md border-2 border-dashed border-primary/40">
        <AppFrame
          src={previewSrc(preview)}
          title={`${proposal.summary.title} preview`}
          tools={proposal.summary.tools.map((t) => t.name)}
          call={(tool, input) =>
            commands.appPreviewCall(preview, tool, input).then((result) => {
              if (proposal.summary.tools.some((t) => t.name === tool && t.writes)) {
                setChanges((n) => n + 1);
              }
              return result;
            })
          }
          changes={changes}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Running in its sandbox on sample data. Your changes here vanish when the card closes, and
        it cannot start dots.
      </p>
    </div>
  );
}

function FileText({ file }: { file: AppFile }) {
  return (
    <details className="rounded-md border">
      <summary className="cursor-pointer px-2 py-1 font-mono text-xs">{file.path}</summary>
      {file.text === null ? (
        <p className="border-t px-2 py-1 text-xs text-muted-foreground">Binary file.</p>
      ) : (
        <pre className="overflow-x-auto border-t p-2 text-xs">{file.text}</pre>
      )}
    </details>
  );
}

/// Every file; for a change, first what it changes against the active version.
function Code({ proposal }: { proposal: AppProposal }) {
  const update = proposal.summary.update_from !== null;
  const files = useQuery({
    queryKey: [...keys.appProposals(proposal.dot_id), proposal.id, "files"],
    queryFn: () => commands.appProposalFiles(proposal.id),
  });
  const diff = useQuery({
    queryKey: [...keys.appProposals(proposal.dot_id), proposal.id, "diff"],
    queryFn: () => commands.appProposalDiff(proposal.id),
    enabled: update,
  });
  return (
    <div className="flex flex-col gap-3">
      {update && (
        <section aria-label="What it changes" className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold text-muted-foreground uppercase">
            What it changes
          </h3>
          {diff.isError ? (
            <ErrorNote error={diff.error} />
          ) : diff.data ? (
            <DiffView diff={diff.data} />
          ) : (
            <p className="text-muted-foreground">Loading the changes…</p>
          )}
        </section>
      )}
      <section aria-label="Every file" className="flex flex-col gap-2">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase">Every file</h3>
        {files.isError ? (
          <ErrorNote error={files.error} />
        ) : (
          (files.data ?? []).map((file) => <FileText key={file.path} file={file} />)
        )}
      </section>
    </div>
  );
}

function Label({ children }: { children: string }) {
  return <dt className="mt-2 text-xs text-muted-foreground uppercase">{children}</dt>;
}

function Summary({ proposal }: { proposal: AppProposal }) {
  const { summary } = proposal;
  const results = summary.tests.results;
  const passed = results.filter((r) => r.passed).length;
  const update = summary.update_from !== null;
  return (
    <dl className="text-sm">
      <Label>What it does</Label>
      <dd>{summary.description}</dd>
      <Label>Tools dots get</Label>
      <dd>
        <ul className="flex flex-col gap-1">
          {summary.tools.map((tool) => (
            <li key={tool.name}>
              <code className="font-mono text-xs">{tool.name}</code>
              {tool.writes && <span className="text-xs text-needs"> (writes)</span>}{" "}
              <span className="text-xs text-muted-foreground">{tool.description}</span>
            </li>
          ))}
        </ul>
      </dd>
      <Label>Reaches outside its data</Label>
      <dd>
        {summary.permissions.includes("start_dots")
          ? "Can start dots"
          : "Nothing (no files, no network)"}
      </dd>
      <dd className="text-xs text-muted-foreground">Sees the names of the dots that use it.</dd>
      <Label>Tests</Label>
      <dd>
        {passed} of {plural(results.length, "test")} passed
        <ul className="text-xs text-muted-foreground">
          {results.map((r) => (
            <li key={`${r.file}/${r.name}`}>
              {r.passed ? "✓" : "✗"} {r.name || r.file}
            </li>
          ))}
        </ul>
      </dd>
      <Label>Size</Label>
      <dd>
        {plural(summary.files, "file")} · {formatBytes(summary.bytes)}
      </dd>
      <Label>{update ? "Migrations it runs on your data" : "Migrations"}</Label>
      <dd>
        {summary.migrations.length === 0 ? "None" : summary.migrations.join(", ")}
      </dd>
    </dl>
  );
}

/// Add to library (or Update), Ask for changes and Discard. A refusal (the dot's chat is still
/// working, the app moved on) stays on the card, which keeps waiting.
function Decide({ proposal }: { proposal: AppProposal }) {
  const qc = useQueryClient();
  const { summary } = proposal;
  const kept = summary.kept_data;
  const [onKept, setOnKept] = useState(true);
  const [asking, setAsking] = useState(false);
  const [text, setText] = useState("");
  const decide = useMutation({
    mutationFn: (decision: ProposalDecision) => commands.appProposalDecide(proposal.id, decision),
    onSuccess: () => {
      for (const queryKey of [
        ["app-proposals"],
        keys.apps,
        keys.library,
        keys.inbox,
        keys.overview,
        keys.runs(proposal.dot_id),
        keys.thread(proposal.root_run_id),
      ]) {
        invalidate(qc, queryKey);
      }
    },
  });
  const update = summary.update_from !== null;
  const starts = summary.permissions.includes("start_dots");
  const add = () =>
    decide.mutate(kept !== null && onKept ? { kind: "add_with_kept_data" } : { kind: "add" });
  return (
    <footer className="flex flex-col gap-2 border-t p-3">
      {starts && (
        <p className="text-xs text-muted-foreground">
          It asks to start dots. It is added with "Can start dots" off; you can switch it on in{" "}
          <Link to="/apps" className="text-foreground hover:underline">
            Apps
          </Link>
          .
        </p>
      )}
      {kept !== null && (
        <label className="flex items-start gap-2 text-xs">
          <input
            type="checkbox"
            checked={onKept}
            onChange={(e) => setOnKept(e.target.checked)}
          />
          <span>
            Start on the data kept from the removed {kept.title} (removed{" "}
            {relativeTime(kept.removed_at)}, {formatBytes(kept.bytes)})
            {kept.migrations.length > 0 && `; it then runs ${kept.migrations.join(", ")} on it`}
          </span>
        </label>
      )}
      {decide.isError && <ErrorNote error={decide.error} />}
      {asking && (
        <div className="flex flex-col gap-2">
          <Textarea
            aria-label="What to change"
            value={text}
            maxLength={MAX_ASK}
            onChange={(e) => setText(e.target.value)}
            placeholder="Say what the dot should change"
          />
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={decide.isPending || text.trim() === ""}
              onClick={onSingleClick(() =>
                decide.mutate({ kind: "ask_for_changes", text: text.trim() }),
              )}
            >
              Send
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setAsking(false)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={decide.isPending} onClick={onSingleClick(add)}>
          {update ? "Update" : "Add to library"}
        </Button>
        {!asking && (
          <Button
            size="sm"
            variant="outline"
            disabled={decide.isPending}
            onClick={() => setAsking(true)}
          >
            Ask for changes
          </Button>
        )}
        <ConfirmButton
          title={`Discard ${summary.title}?`}
          description="The dot's files for it are deleted here, and the dot hears that you discarded it."
          confirmLabel="Discard"
          disabled={decide.isPending}
          onConfirm={() => decide.mutate({ kind: "discard" })}
        >
          Discard
        </ConfirmButton>
      </div>
    </footer>
  );
}
